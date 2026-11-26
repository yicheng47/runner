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
            cell_width: 0,
            cell_height: 0,
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
            .browser_resize(
                "left",
                1,
                "Remote",
                100,
                30,
                0,
                0,
                true,
                false,
                &client_core.db,
            )
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
            .take_back_terminal("left", 80, 24, 0, 0, &client_core.db)
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kitty_socket_single_reader_single_reply_normalized_stream_splitting_and_queue_resync() {
    use base64::Engine as _;
    use runner_core::protocol::socket::SocketTransport;
    use runner_core::protocol::terminal::{TerminalFrame, TERMINAL_QUEUE_MAX_BYTES};
    use runner_terminal::terminal::TerminalMirror;
    use std::sync::Condvar;
    use std::time::Instant;

    #[derive(Default)]
    struct SocketKittyRuntime {
        outputs: Mutex<
            std::collections::HashMap<
                String,
                std::sync::mpsc::Sender<crate::session::runtime::RuntimeOutput>,
            >,
        >,
        writes: Mutex<Vec<Vec<u8>>>,
        writes_cv: Condvar,
    }
    impl SocketKittyRuntime {
        fn push_output(&self, session_id: &str, bytes: &[u8]) {
            self.outputs
                .lock()
                .unwrap()
                .get(session_id)
                .expect("spawned session")
                .send(crate::session::runtime::RuntimeOutput::Stream(
                    bytes.to_vec(),
                ))
                .unwrap();
        }
        fn wait_writes_len(&self, expected: usize) -> Vec<Vec<u8>> {
            let mut guard = self.writes.lock().unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while guard.len() < expected {
                let rem = deadline
                    .checked_duration_since(Instant::now())
                    .expect("timed out waiting for PTY replies");
                let (next, _) = self.writes_cv.wait_timeout(guard, rem).unwrap();
                guard = next;
            }
            guard.clone()
        }
        fn take_writes(&self) -> Vec<Vec<u8>> {
            std::mem::take(&mut *self.writes.lock().unwrap())
        }
    }
    impl crate::session::runtime::SessionRuntime for SocketKittyRuntime {
        fn spawn(
            &self,
            spec: crate::session::runtime::SpawnSpec,
        ) -> crate::session::runtime::RuntimeResult<(
            crate::session::runtime::RuntimeSession,
            crate::session::runtime::OutputStream,
        )> {
            let (tx, rx) = std::sync::mpsc::channel();
            self.outputs
                .lock()
                .unwrap()
                .insert(spec.session_id.clone(), tx);
            Ok((
                crate::session::runtime::RuntimeSession {
                    runtime: "socket-kitty".into(),
                    session_id: spec.session_id,
                },
                crate::session::runtime::OutputStream::new(
                    rx,
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                ),
            ))
        }
        fn stop(
            &self,
            session: &crate::session::runtime::RuntimeSession,
        ) -> crate::session::runtime::RuntimeResult<()> {
            self.outputs.lock().unwrap().remove(&session.session_id);
            Ok(())
        }
        fn send_bytes(
            &self,
            _: &crate::session::runtime::RuntimeSession,
            bytes: &[u8],
        ) -> crate::session::runtime::RuntimeResult<()> {
            self.writes.lock().unwrap().push(bytes.to_vec());
            self.writes_cv.notify_all();
            Ok(())
        }
        fn send_key(
            &self,
            _: &crate::session::runtime::RuntimeSession,
            _: &str,
        ) -> crate::session::runtime::RuntimeResult<()> {
            Ok(())
        }
        fn resize(
            &self,
            _: &crate::session::runtime::RuntimeSession,
            _: u16,
            _: u16,
            _: u16,
            _: u16,
        ) -> crate::session::runtime::RuntimeResult<()> {
            Ok(())
        }
        fn status(
            &self,
            _: &crate::session::runtime::RuntimeSession,
        ) -> crate::session::runtime::RuntimeResult<Option<crate::session::runtime::SessionStatus>>
        {
            Ok(Some(crate::session::runtime::SessionStatus {
                alive: true,
                ..Default::default()
            }))
        }
    }

    let root = tempfile::tempdir().unwrap();
    let runtime = Arc::new(SocketKittyRuntime::default());
    let core = crate::test_support::test_core_in_with_runtime(
        root.path().to_owned(),
        Arc::clone(&runtime) as _,
    );
    let role = {
        let conn = core.db.get().unwrap();
        crate::test_support::insert_test_role(
            &conn,
            "kitty-sock",
            "kitty-sock",
            "trae",
            "kitty-sock",
        );
        crate::ops::role::get(&conn, "kitty-sock").unwrap()
    };
    let session_tmp = tempfile::Builder::new()
        .prefix("runner-kitty-sock-")
        .tempdir_in(std::env::temp_dir())
        .unwrap();
    let spawned = core
        .sessions
        .spawn_direct(
            &role,
            crate::session::manager::DirectLaunch {
                cwd: Some(root.path().to_str().unwrap().into()),
                cols: Some(80),
                rows: Some(24),
                ..Default::default()
            },
            &core.app_data_dir,
            core.db.clone(),
            Arc::new(core.session_events()),
        )
        .unwrap();
    let id = spawned.id.clone();

    #[cfg(unix)]
    let endpoint = IpcEndpoint(root.path().join("kitty-runnerd.sock"));
    #[cfg(windows)]
    let endpoint = IpcEndpoint(std::path::PathBuf::from(format!(
        r"\\.\pipe\runner-kitty-sock-test-{}",
        root.path().file_name().unwrap().to_string_lossy()
    )));
    let mut listener = IpcListener::bind(&endpoint).unwrap();
    let shutdown = CancellationToken::new();
    let accept_shutdown = shutdown.clone();
    let server_core = core.clone();
    let server = tokio::spawn(async move {
        let mut conns = Vec::new();
        let mut next_id = 1u64;
        loop {
            tokio::select! {
                _ = accept_shutdown.cancelled() => break,
                accepted = listener.accept() => {
                    let Ok(stream) = accepted else { break };
                    let conn_core = server_core.clone();
                    let conn_shutdown = accept_shutdown.clone();
                    let conn_id = next_id;
                    next_id += 1;
                    conns.push(tokio::spawn(async move {
                        let windows = Arc::new(ConnectionWindows {
                            core: conn_core.clone(),
                            owners: Arc::new(Mutex::default()),
                            id: conn_id,
                            closed: CancellationToken::new(),
                        });
                        let (_report, resume_report) = tokio::sync::watch::channel(None);
                        let _ = connection(
                            stream,
                            conn_core,
                            wire::Welcome {
                                exe_sha256: "hash".into(),
                                pid: 1,
                                started_at: "now".into(),
                                app_clients: 0,
                            },
                            Default::default(),
                            conn_shutdown,
                            true,
                            windows,
                            resume_report,
                        )
                        .await;
                    }));
                }
            }
        }
        for handle in conns {
            let _ = handle.await;
        }
    });

    let runtime_t = Arc::clone(&runtime);
    let core_t = core.clone();
    let id_t = id.clone();
    let tmp_dir_path = session_tmp.path().to_path_buf();
    let endpoint_t = endpoint.clone();
    let shutdown_t = shutdown.clone();
    tokio::task::spawn_blocking(move || {
        let connect = || {
            SocketTransport::connect(
                &endpoint_t,
                wire::Hello {
                    exe_sha256: "hash".into(),
                    client: "kitty-test".into(),
                },
            )
            .unwrap()
        };
        let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let write_temp = |tag: &str, data: &[u8]| -> std::path::PathBuf {
            let path = tmp_dir_path.join(format!("tty-graphics-protocol-{tag}.bin"));
            std::fs::write(&path, data).unwrap();
            path
        };
        let wait_mirror_has = |mirror: &TerminalMirror, image_id: u32, expected: bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let present = mirror
                    .graphics()
                    .lock()
                    .unwrap()
                    .primary
                    .images()
                    .contains_key(&image_id);
                if present == expected {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for mirror image {image_id} present={expected}"
                );
                std::thread::yield_now();
            }
        };

        // 1. 0, 1, and 2 real socket mirrors: exactly one PTY reply per query/transmission,
        // `t=t` file read and deleted once, mirrors show image and never see forbidden wire tokens.
        let mut mirrors: Vec<(Arc<SocketTransport>, Arc<TerminalMirror>)> = Vec::new();
        let raw_transport = connect();
        let mut raw_sub = raw_transport.client().attach(&id_t).unwrap();

        for viewer_count in 0..=2usize {
            while mirrors.len() < viewer_count {
                let transport = connect();
                let mirror =
                    TerminalMirror::attach(transport.client(), id_t.clone(), Arc::new(|| {}))
                        .unwrap();
                mirrors.push((transport, mirror));
            }
            let q_id = 100 + viewer_count as u32;
            let img_id = 200 + viewer_count as u32;
            let q_file = write_temp(&format!("q-{viewer_count}"), &[1u8; 12]);
            let t_file = write_temp(&format!("t-{viewer_count}"), &[2u8; 12]);
            let q_cmd = format!(
                "\x1b_Ga=q,i={q_id},t=t,f=24,s=2,v=2;{}\x1b\\",
                b64(q_file.to_string_lossy().as_bytes())
            );
            let t_cmd = format!(
                "\x1b_Ga=T,i={img_id},t=t,f=24,s=2,v=2;{}\x1b\\",
                b64(t_file.to_string_lossy().as_bytes())
            );
            runtime_t.push_output(&id_t, q_cmd.as_bytes());
            let q_writes = runtime_t.wait_writes_len(1);
            assert_eq!(
                q_writes,
                vec![format!("\x1b_Gi={q_id};OK\x1b\\").into_bytes()],
                "viewer_count={viewer_count}: query must produce exactly one reply"
            );
            assert!(!q_file.exists(), "t=t query file must be deleted once");
            runtime_t.take_writes();

            runtime_t.push_output(&id_t, t_cmd.as_bytes());
            let t_writes = runtime_t.wait_writes_len(1);
            assert_eq!(
                t_writes,
                vec![format!("\x1b_Gi={img_id};OK\x1b\\").into_bytes()],
                "viewer_count={viewer_count}: transmission must produce exactly one reply"
            );
            assert!(
                !t_file.exists(),
                "t=t transmission file must be deleted once"
            );
            runtime_t.take_writes();

            let frame = raw_sub.frames.recv().unwrap();
            let TerminalFrame::Output { bytes, .. } = frame else {
                panic!("expected normalized output frame, got {frame:?}");
            };
            let wire_str = String::from_utf8_lossy(&bytes);
            assert!(
                !wire_str.contains("a=q"),
                "mirror received query: {wire_str}"
            );
            assert!(!wire_str.contains("t=t"), "mirror received t=t: {wire_str}");
            assert!(!wire_str.contains("t=f"), "mirror received t=f: {wire_str}");
            assert!(!wire_str.contains("t=s"), "mirror received t=s: {wire_str}");
            assert!(
                !wire_str.contains("I="),
                "mirror received image number: {wire_str}"
            );
            assert!(
                !wire_str.contains("m=1"),
                "mirror received continuation chunk: {wire_str}"
            );
            assert!(
                wire_str.contains(&format!("i={img_id}")),
                "normalized frame missing image id {img_id}: {wire_str}"
            );
            for (_, mirror) in &mirrors {
                wait_mirror_has(mirror, img_id, true);
            }
        }

        // Chunked I= transmission reaches mirrors with assigned i=, no I=, and no m=1.
        let chunk_payload = b64(&[9u8; 12]);
        let chunked = format!(
            "\x1b_Ga=T,I=77,f=24,s=2,v=2,m=1;{}\x1b\\\x1b_Gm=0;{}\x1b\\",
            &chunk_payload[..8],
            &chunk_payload[8..]
        );
        runtime_t.push_output(&id_t, chunked.as_bytes());
        let chunk_writes = runtime_t.wait_writes_len(1);
        assert!(String::from_utf8_lossy(&chunk_writes[0]).contains("I=77;OK"));
        runtime_t.take_writes();
        let TerminalFrame::Output { bytes, .. } = raw_sub.frames.recv().unwrap() else {
            panic!("expected normalized chunked frame");
        };
        let wire_str = String::from_utf8_lossy(&bytes);
        assert!(!wire_str.contains("I="), "mirror received I=: {wire_str}");
        assert!(!wire_str.contains("m=1"), "mirror received m=1: {wire_str}");
        assert!(
            wire_str.contains("q=2"),
            "normalized frame must carry q=2: {wire_str}"
        );

        // Retransmitting existing i=202 with invalid data deletes old i=202 as `a=d,d=I,i=202,q=2`.
        runtime_t.push_output(&id_t, b"\x1b_Ga=t,i=202,f=24,s=2,v=2;AA==\x1b\\");
        let err_writes = runtime_t.wait_writes_len(1);
        assert_eq!(
            err_writes,
            vec![b"\x1b_Gi=202;EINVAL: invalid data\x1b\\".to_vec()]
        );
        runtime_t.take_writes();
        let TerminalFrame::Output { bytes, .. } = raw_sub.frames.recv().unwrap() else {
            panic!("expected delete frame for failed retransmission");
        };
        assert_eq!(bytes, b"\x1b_Ga=d,d=I,i=202,q=2\x1b\\");
        for (_, mirror) in &mirrors {
            wait_mirror_has(mirror, 202, false);
        }

        // 2. Empty normalized burst (query only) publishes nothing and snapshot seq still matches.
        let seq_before_query = core_t.sessions.terminal_output_sequence(&id_t);
        runtime_t.push_output(&id_t, b"\x1b_Ga=q,i=999,f=24,s=1,v=1;AAAA\x1b\\");
        let q_only_writes = runtime_t.wait_writes_len(1);
        assert_eq!(q_only_writes, vec![b"\x1b_Gi=999;OK\x1b\\".to_vec()]);
        runtime_t.take_writes();
        let seq_after_query = core_t.sessions.terminal_output_sequence(&id_t);
        assert_eq!(seq_after_query, seq_before_query + 1);
        let snap_after_query = raw_transport.client().attach(&id_t).unwrap().snapshot;
        assert_eq!(snap_after_query.seq, seq_after_query);
        runtime_t.push_output(&id_t, b"after-query-gap");
        let TerminalFrame::Output { seq, bytes } = raw_sub.frames.recv().unwrap() else {
            panic!("expected text frame after query gap");
        };
        assert_eq!(seq, seq_after_query + 1);
        assert_eq!(bytes, b"after-query-gap");

        // 3. Single >1 MiB transmission travels alone between surrounding text segments.
        let big_raw = vec![0x5Au8; 600 * 600 * 3]; // 1.08 MB raw -> ~1.44 MB normalized (> 1 MiB)
        let big_file = tmp_dir_path.join("big-single.bin");
        std::fs::write(&big_file, &big_raw).unwrap();
        let big_burst = format!(
            "before-big\x1b_Ga=T,i=300,t=f,f=24,s=600,v=600;{}\x1b\\after-big",
            b64(big_file.to_string_lossy().as_bytes())
        );
        runtime_t.push_output(&id_t, big_burst.as_bytes());
        let big_writes = runtime_t.wait_writes_len(1);
        assert_eq!(big_writes, vec![b"\x1b_Gi=300;OK\x1b\\".to_vec()]);
        runtime_t.take_writes();
        let TerminalFrame::Output { bytes: f1, .. } = raw_sub.frames.recv().unwrap() else {
            panic!("expected leading text frame");
        };
        let TerminalFrame::Output { bytes: f2, .. } = raw_sub.frames.recv().unwrap() else {
            panic!("expected isolated large image frame");
        };
        let TerminalFrame::Output { bytes: f3, .. } = raw_sub.frames.recv().unwrap() else {
            panic!("expected trailing text frame");
        };
        assert_eq!(f1, b"before-big");
        assert!(f2.len() > 1024 * 1024);
        assert!(f2.starts_with(b"\x1b_G") && f2.ends_with(b"\x1b\\"));
        assert_eq!(f3, b"after-big");
        for (_, mirror) in &mirrors {
            wait_mirror_has(mirror, 300, true);
        }

        // 4. Burst of 73 sub-1 MiB `t=f` transmissions totalling > 64 MiB splits into <= 1 MiB frames
        // and never drops the daemon socket with "output too large".
        let sub_raw = vec![0x33u8; 500 * 466 * 3]; // 699,000 B raw -> ~932 KB normalized (< 1 MiB)
        let sub_file = tmp_dir_path.join("sub-1mib.bin");
        std::fs::write(&sub_file, &sub_raw).unwrap();
        let sub_b64 = b64(sub_file.to_string_lossy().as_bytes());

        // 4a. Unconditionally prove the complete 73-frame (> 64 MiB) split below the socket queues
        // so slow reader scheduling can never mask it via Resync.
        {
            use runner_terminal::kitty::{
                prepare_command, CellSize, ForwardedItem, GlobalImageBudget, TempDirs,
            };
            use runner_terminal::terminal::{
                ClipboardTarget, ModelOptions, TerminalHost, TerminalModel,
            };
            struct DirectHost;
            impl TerminalHost for DirectHost {
                fn write_input(&self, _: &str, _: &[u8]) -> anyhow::Result<()> {
                    Ok(())
                }
                fn write_reply(&self, _: &str, _: &[u8]) -> anyhow::Result<()> {
                    Ok(())
                }
                fn report_input_state(
                    &self,
                    _: &str,
                    _: runner_terminal::input_state::InputObservation,
                ) {
                }
                fn set_live_title(&self, _: &str, _: &str) -> anyhow::Result<()> {
                    Ok(())
                }
                fn resize(&self, _: &str, _: u16, _: u16) -> anyhow::Result<()> {
                    Ok(())
                }
                fn is_local_host(&self, _: &str) -> bool {
                    true
                }
                fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
            }
            let direct_model = TerminalModel::with_graphics(
                Arc::new(DirectHost),
                "direct-split-73".into(),
                80,
                24,
                ModelOptions::default(),
                CellSize::ZERO,
                Some(Arc::new(GlobalImageBudget::new(128 * 1024 * 1024))),
            )
            .unwrap();
            let temp_dirs = TempDirs::with_session_tmpdir(Some(&tmp_dir_path));
            let items: Vec<ForwardedItem> = (1..=73u32)
                .map(|n| {
                    let raw = format!("a=t,i={n},t=f,f=24,s=500,v=466,q=1;{sub_b64}");
                    ForwardedItem::Command(prepare_command(raw.as_bytes(), &temp_dirs).unwrap())
                })
                .collect();
            let (_obs, frames, last_seq) = direct_model.feed_forwarded(1, &items);
            assert_eq!(frames.len(), 73, "73 sub-1 MiB images must emit 73 frames");
            assert_eq!(last_seq, 73);
            let max_frame = frames.iter().map(Vec::len).max().unwrap_or(0);
            let total_bytes: usize = frames.iter().map(Vec::len).sum();
            assert!(
                max_frame <= 1024 * 1024,
                "normalized burst frame exceeded 1 MiB: {max_frame}"
            );
            assert!(
                total_bytes > 64 * 1024 * 1024,
                "expected >64 MiB total across frames, got {total_bytes}"
            );
            for worker in direct_model.stop_workers() {
                let _ = worker.join();
            }
        }

        // 4b. Over the real socket, first send a 40-image burst (< 64 frames, < 128 MiB) and
        // unconditionally assert every frame arrives intact with zero Resync and max <= 1 MiB.
        let mut burst_40 = Vec::new();
        for n in 1..=40u32 {
            let id_n = 350 + n;
            burst_40.extend_from_slice(
                format!("\x1b_Ga=t,i={id_n},t=f,f=24,s=500,v=466,q=1;{sub_b64}\x1b\\").as_bytes(),
            );
        }
        burst_40.extend_from_slice(b"\x1b_Ga=q,i=997,f=24,s=1,v=1;AAAA\x1b\\");
        runtime_t.push_output(&id_t, &burst_40);
        assert_eq!(
            runtime_t.wait_writes_len(1),
            vec![b"\x1b_Gi=997;OK\x1b\\".to_vec()]
        );
        runtime_t.take_writes();
        runtime_t.push_output(&id_t, b"burst-40-sentinel");
        let mut burst_40_count = 0usize;
        let mut burst_40_bytes = 0usize;
        let mut burst_40_max = 0usize;
        loop {
            match raw_sub.frames.recv().unwrap() {
                TerminalFrame::Output { bytes, .. } if bytes == b"burst-40-sentinel" => break,
                TerminalFrame::Output { bytes, .. } => {
                    burst_40_max = burst_40_max.max(bytes.len());
                    burst_40_bytes += bytes.len();
                    burst_40_count += 1;
                }
                TerminalFrame::Resized { .. } => {}
                TerminalFrame::Resync => panic!("40-frame (<128 MiB) burst must never Resync"),
            }
        }
        assert_eq!(burst_40_count, 40);
        assert!(burst_40_max <= 1024 * 1024);
        assert!(burst_40_bytes > 35 * 1024 * 1024);

        // 4c. Now push a 73-image (> 64 MiB) burst over the real socket and unconditionally assert
        // the transport survives without dropping ("output too large") and no frame exceeds 1 MiB.
        let mut burst_64m = Vec::new();
        for n in 1..=73u32 {
            let id_n = 400 + n;
            burst_64m.extend_from_slice(
                format!("\x1b_Ga=t,i={id_n},t=f,f=24,s=500,v=466,q=1;{sub_b64}\x1b\\").as_bytes(),
            );
        }
        burst_64m.extend_from_slice(b"\x1b_Ga=q,i=998,f=24,s=1,v=1;AAAA\x1b\\");

        let mut drain_sub = raw_sub;
        let reader = std::thread::spawn(move || {
            let mut max_frame = 0usize;
            loop {
                match drain_sub.frames.recv() {
                    Ok(TerminalFrame::Output { bytes, .. }) => {
                        if bytes == b"burst-done-sentinel" {
                            break;
                        }
                        max_frame = max_frame.max(bytes.len());
                    }
                    Ok(TerminalFrame::Resync) => break,
                    Ok(TerminalFrame::Resized { .. }) => {}
                    Err(error) => panic!("socket dropped during >64 MiB burst: {error}"),
                }
            }
            max_frame
        });
        runtime_t.push_output(&id_t, &burst_64m);
        let sentinel_writes = runtime_t.wait_writes_len(1);
        assert_eq!(sentinel_writes, vec![b"\x1b_Gi=998;OK\x1b\\".to_vec()]);
        runtime_t.take_writes();
        runtime_t.push_output(&id_t, b"burst-done-sentinel");
        let max_frame = reader.join().unwrap();
        assert!(
            max_frame <= 1024 * 1024,
            "normalized burst frame exceeded 1 MiB: {max_frame}"
        );
        // The socket transport itself is still alive and serves a fresh attach cleanly.
        let post_burst = raw_transport.client().attach(&id_t).unwrap();
        assert_eq!(
            post_burst.snapshot.seq,
            core_t.sessions.terminal_output_sequence(&id_t)
        );
        assert!(core_t
            .sessions
            .terminal_model(&id_t)
            .unwrap()
            .graphics()
            .lock()
            .unwrap()
            .primary
            .images()
            .contains_key(&473));

        // 5. Queueing > 128 MiB of frames (well below 64 frames) triggers Resync over the real
        // daemon socket. Push 8 * ~41.9 MB = ~335 MB (strictly exceeding 3 daemon + 1 in-flight +
        // 3 client = 7 frames) and synchronize on the daemon FrameQueue + a shared-connection
        // fence session so `overflow_sub.frames.recv()` can never hang.
        let sync_spawned = core_t
            .sessions
            .spawn_direct(
                &role,
                crate::session::manager::DirectLaunch {
                    cwd: Some(root.path().to_str().unwrap().into()),
                    cols: Some(80),
                    rows: Some(24),
                    ..Default::default()
                },
                &core_t.app_data_dir,
                core_t.db.clone(),
                Arc::new(core_t.session_events()),
            )
            .unwrap();
        let sync_id = sync_spawned.id.clone();

        let overflow_transport = connect();
        let mut overflow_sub = overflow_transport.client().attach(&id_t).unwrap();
        let mut sync_sub = overflow_transport.client().attach(&sync_id).unwrap();
        drop(mirrors);
        let raw_30m = vec![0x2Au8; 3200 * 3276 * 3]; // 31,449,600 B -> ~41.93 MB normalized
        let file_30m = tmp_dir_path.join("overflow-30mib.bin");
        std::fs::write(&file_30m, &raw_30m).unwrap();
        let file_30m_b64 = b64(file_30m.to_string_lossy().as_bytes());
        let cmd_30m = format!("\x1b_Ga=t,i=500,t=f,f=24,s=3200,v=3276;{file_30m_b64}\x1b\\");
        assert!(4 * (raw_30m.len() * 4 / 3) > TERMINAL_QUEUE_MAX_BYTES);
        for _ in 0..8 {
            runtime_t.push_output(&id_t, cmd_30m.as_bytes());
        }
        let overflow_writes = runtime_t.wait_writes_len(8);
        assert!(overflow_writes.iter().all(|w| w == b"\x1b_Gi=500;OK\x1b\\"));
        runtime_t.take_writes();

        // Wait until either the daemon FrameQueue overflowed (removing `overflow_sub` after
        // queuing Resync) or the daemon FrameQueue drained all 8 frames (335 MB) into the socket.
        let deadline = Instant::now() + Duration::from_secs(10);
        let daemon_overflowed = loop {
            match core_t
                .sessions
                .subscriber_queue_state(&id_t, overflow_sub.subscriber_id)
            {
                None => break true,
                Some((0, 0, false)) => break false,
                Some((_, _, true)) => break true,
                Some(_) => {
                    assert!(
                        Instant::now() < deadline,
                        "timed out waiting for overflow queue to fill or drain"
                    );
                    std::thread::yield_now();
                }
            }
        };
        if !daemon_overflowed {
            // All 8 frames (335 MB) already left the daemon FrameQueue into the shared connection;
            // send a fence on `sync_id` over the SAME socket connection so `read_loop` has already
            // processed all 8 frames into `overflow_sub`'s client queue before we read it.
            runtime_t.push_output(&sync_id, b"overflow-fence");
            let fence = sync_sub.frames.recv().unwrap();
            assert!(matches!(
                fence,
                TerminalFrame::Output { ref bytes, .. } if bytes == b"overflow-fence"
            ));
        }

        let mut got_resync = false;
        for _ in 0..16 {
            match overflow_sub.frames.recv() {
                Ok(TerminalFrame::Resync) => {
                    got_resync = true;
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        assert!(
            got_resync,
            "queueing >128 MiB must trigger TerminalFrame::Resync"
        );

        drop(post_burst);
        drop(sync_sub);
        drop(overflow_sub);
        drop(raw_transport);
        drop(overflow_transport);
        core_t.sessions.kill(&sync_id).unwrap();
        core_t.sessions.kill(&id_t).unwrap();
        core_t.sessions.drain_terminal_workers();
        shutdown_t.cancel();
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .unwrap()
        .unwrap();
}
