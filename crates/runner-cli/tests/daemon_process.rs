use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(windows)]
use runner_core::app_paths::IpcEndpoint;
use runner_core::daemon_process::{self, Launch, NativePaths};
use runner_core::protocol::socket::{ConnectError, SocketTransport};
use runner_core::protocol::terminal::TerminalFrame;
use runner_core::protocol::wire::Hello;
use runner_core::protocol::{DaemonClient, ProjectScope};

struct Daemon {
    root: tempfile::TempDir,
    launch: Launch,
    child: Option<Child>,
    hash: String,
}
impl Daemon {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let source = PathBuf::from(env!("CARGO_BIN_EXE_runner-agent-cli"));
        let daemon = bin.join(runner_core::cli_install::DAEMON_DEST_BIN_NAME);
        std::fs::copy(&source, &daemon).unwrap();
        let cli = bin.join(runner_core::cli_install::AGENT_DEST_BIN_NAME);
        std::fs::copy(&source, &cli).unwrap();
        #[cfg(windows)]
        for name in ["conpty.dll", "OpenConsole.exe"] {
            let companion = source.parent().unwrap().join(name);
            if companion.exists() {
                std::fs::copy(companion, bin.join(name)).unwrap();
            }
        }
        let home = root.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let paths = NativePaths {
            home_dir: Some(home),
            app_data_dir: root.path().join("data"),
            log_dir: root.path().join("logs"),
        };
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let mut launch = Launch::new(paths, cli, false);
        launch.isolated = true;
        #[cfg(windows)]
        {
            let key = root.path().file_name().unwrap().to_string_lossy();
            launch.daemon_endpoint =
                IpcEndpoint(PathBuf::from(format!(r"\\.\pipe\runnerd-test-{key}")));
            launch.mcp_endpoint =
                IpcEndpoint(PathBuf::from(format!(r"\\.\pipe\runner-mcp-test-{key}")));
        }
        let hash = daemon_process::executable_hash(&daemon).unwrap();
        Self {
            root,
            launch,
            child: None,
            hash,
        }
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(
            self.launch
                .source
                .parent()
                .unwrap()
                .join(runner_core::cli_install::DAEMON_DEST_BIN_NAME),
        );
        cmd.arg("--app-data-dir")
            .arg(&self.launch.paths.app_data_dir)
            .arg("--log-dir")
            .arg(&self.launch.paths.log_dir)
            .arg("--home-dir")
            .arg(self.launch.paths.home_dir.as_ref().unwrap())
            .arg("--endpoint")
            .arg(&self.launch.daemon_endpoint.0)
            .arg("--mcp-endpoint")
            .arg(&self.launch.mcp_endpoint.0)
            .arg("--isolated")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(self.root.path().join("stderr.log")).unwrap());
        #[cfg(unix)]
        cmd.env("SHELL", "/bin/sh");
        cmd
    }
    fn start(&mut self) -> Arc<SocketTransport> {
        self.child = Some(self.command().spawn().unwrap());
        self.connect()
    }
    fn connect(&self) -> Arc<SocketTransport> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match SocketTransport::connect(&self.launch.daemon_endpoint, self.hello(&self.hash)) {
                Ok(client) => return client,
                Err(ConnectError::NotRunning) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => panic!(
                    "daemon connect: {error}; {}",
                    std::fs::read_to_string(self.root.path().join("stderr.log"))
                        .unwrap_or_default()
                ),
            }
        }
    }
    fn hello(&self, hash: &str) -> Hello {
        Hello {
            exe_sha256: hash.into(),
            client: "test".into(),
        }
    }
    fn stopped(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(12);
        while self
            .child
            .as_mut()
            .is_some_and(|child| child.try_wait().unwrap().is_none())
        {
            assert!(Instant::now() < deadline, "runnerd failed to exit");
            std::thread::sleep(Duration::from_millis(20));
        }
        daemon_process::wait_unlocked(&self.launch.paths.app_data_dir, Duration::from_secs(1))
            .unwrap();
        #[cfg(unix)]
        {
            assert!(!self.launch.daemon_endpoint.0.exists());
            assert!(!self.launch.mcp_endpoint.0.exists());
        }
    }
    fn shell(&self, client: &DaemonClient) -> runner_core::protocol::SpawnedSession {
        let mut spawned = client
            .session_start_shell_in(
                ProjectScope::Root,
                Some(self.root.path().to_string_lossy().into()),
                Some(80),
                Some(24),
            )
            .unwrap();
        let pool = runner_daemon::db::open_pool(&self.launch.paths.app_data_dir.join("runner.db"))
            .unwrap();
        spawned.pid = runner_daemon::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
            .unwrap()
            .unwrap()
            .pid
            .map(|pid| pid as u32);
        spawned
    }
}
impl Drop for Daemon {
    fn drop(&mut self) {
        if let Ok(client) = SocketTransport::connect(&self.launch.daemon_endpoint, self.hello("")) {
            let _ = client.shutdown(true);
            let _ = daemon_process::wait_unlocked(
                &self.launch.paths.app_data_dir,
                Duration::from_secs(10),
            );
        }
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn output_until(
    frames: &mut Box<dyn runner_core::protocol::terminal::TerminalSubscription>,
    marker: &[u8],
) -> Vec<u8> {
    struct Closed;
    impl runner_core::protocol::terminal::TerminalSubscription for Closed {
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            Arc::new(|| {})
        }
        fn recv(&mut self) -> Result<TerminalFrame, runner_core::protocol::ClientError> {
            Err(runner_core::protocol::ClientError::msg(
                "test subscription moved",
            ))
        }
    }
    let mut incoming = std::mem::replace(frames, Box::new(Closed));
    let marker = marker.to_vec();
    let (done, result) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        loop {
            match incoming.recv().unwrap() {
                TerminalFrame::Output { bytes, .. } => output.extend(bytes),
                TerminalFrame::Resized { .. } => (),
                TerminalFrame::Resync => panic!("unexpected lag"),
            }
            if output.windows(marker.len()).any(|chunk| chunk == marker) {
                let _ = done.send((output, incoming));
                return;
            }
        }
    });
    let (output, incoming) = result
        .recv_timeout(Duration::from_secs(10))
        .expect("terminal output deadline");
    *frames = incoming;
    output
}

#[test]
fn handshake_mismatch_lock_and_idle_lifetime() {
    let mut daemon = Daemon::new();
    let socket = daemon.start();
    assert!(socket.welcome.pid > 0);
    assert!(matches!(
        SocketTransport::connect(&daemon.launch.daemon_endpoint, daemon.hello("different")),
        Err(ConnectError::Mismatch(_))
    ));
    let mut second = daemon.command().spawn().unwrap();
    assert!(second.wait().unwrap().success());
    let shell = daemon.shell(&socket.client());
    socket.client().session_close(&shell.id).unwrap();
    drop(socket);
    std::thread::sleep(Duration::from_millis(150));
    assert!(daemon.child.as_mut().unwrap().try_wait().unwrap().is_none());
    let client = daemon.connect();
    client.shutdown(true).unwrap();
    daemon.stopped();
}

#[test]
fn shell_echo_two_clients_and_identical_reattach_snapshot() {
    let mut daemon = Daemon::new();
    let socket = daemon.start();
    let client = socket.client();
    let shell = daemon.shell(&client);
    let other = daemon.connect();
    let deadline = Instant::now() + Duration::from_secs(10);
    let (mut a, mut b) = loop {
        let a = client.attach(&shell.id).unwrap();
        let b = other.client().attach(&shell.id).unwrap();
        assert!(
            b.snapshot.seq >= a.snapshot.seq,
            "terminal sequence regressed"
        );
        if a.snapshot.seq == b.snapshot.seq {
            break (a, b);
        }
        assert!(Instant::now() < deadline, "shell startup did not settle");
        std::thread::sleep(Duration::from_millis(10));
    };
    client
        .input(&shell.id, b"echo SOCKET_ECHO_645\r\n")
        .unwrap();
    let first = output_until(&mut a.frames, b"SOCKET_ECHO_645");
    let second = output_until(&mut b.frames, b"SOCKET_ECHO_645");
    assert_eq!(first, second);
    let mut expected = client.attach(&shell.id).unwrap().snapshot;
    drop((a, b, client, socket));
    let deadline = Instant::now() + Duration::from_secs(10);
    let reattached = loop {
        let snapshot = daemon
            .connect()
            .client()
            .attach(&shell.id)
            .unwrap()
            .snapshot;
        assert!(snapshot.seq >= expected.seq, "terminal sequence regressed");
        if snapshot.seq == expected.seq {
            break snapshot;
        }
        assert!(Instant::now() < deadline, "shell output did not settle");
        std::thread::sleep(Duration::from_millis(10));
        expected = other.client().attach(&shell.id).unwrap().snapshot;
    };
    assert_eq!(expected.seq, reattached.seq);
    assert_eq!(expected.bytes, reattached.bytes);
    assert_eq!(
        (
            expected.cols,
            expected.rows,
            expected.unfinished_len,
            expected.preceding_char
        ),
        (
            reattached.cols,
            reattached.rows,
            reattached.unfinished_len,
            reattached.preceding_char
        )
    );
    other.shutdown(true).unwrap();
    daemon.stopped();
    assert!(!process_exists(shell.pid.unwrap()));
}

#[test]
fn killed_daemon_closes_child_and_next_start_demotes_rows() {
    let mut daemon = Daemon::new();
    let socket = daemon.start();
    let client = socket.client();
    let shell = daemon.shell(&client);
    daemon.child.as_mut().unwrap().kill().unwrap();
    daemon.child.as_mut().unwrap().wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while process_exists(shell.pid.unwrap()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!process_exists(shell.pid.unwrap()), "daemon child survived");
    assert!(client.role_list().is_err());
    let replacement = daemon.start();
    assert!(replacement.client().session_live_ids().unwrap().is_empty());
    replacement.shutdown(true).unwrap();
    daemon.stopped();
}

#[cfg(unix)]
#[test]
fn sigterm_stamps_resume_and_replaced_socket_stops_daemon() {
    let mut daemon = Daemon::new();
    let socket = daemon.start();
    let shell = daemon.shell(&socket.client());
    unsafe {
        libc::kill(socket.welcome.pid as i32, libc::SIGTERM);
    }
    daemon.stopped();
    assert!(!process_exists(shell.pid.unwrap()));
    let pool =
        runner_daemon::db::open_pool(&daemon.launch.paths.app_data_dir.join("runner.db")).unwrap();
    let conn = pool.get().unwrap();
    assert!(
        runner_daemon::repo::session::get_row(&conn, &shell.id)
            .unwrap()
            .unwrap()
            .resume_on_launch
    );
    conn.execute("UPDATE sessions SET resume_on_launch=0", [])
        .unwrap();
    drop(conn);
    drop(pool);
    let _socket = daemon.start();
    std::fs::remove_file(&daemon.launch.daemon_endpoint.0).unwrap();
    let replacement =
        std::os::unix::net::UnixListener::bind(&daemon.launch.daemon_endpoint.0).unwrap();
    // The replacement belongs to this test; runnerd must leave it alone.
    let deadline = Instant::now() + Duration::from_secs(8);
    while daemon.child.as_mut().unwrap().try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(daemon.launch.daemon_endpoint.0.exists());
    drop(replacement);
}

#[test]
fn cli_starts_only_on_not_running_and_never_from_ssh() {
    let daemon = Daemon::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        assert!(matches!(
            runner_cli::client::SocketClient::connect_or_start(&daemon.launch, true).await,
            Err(runner_cli::client::ClientError::NotRunning)
        ));
        let client = runner_cli::client::SocketClient::connect_or_start(&daemon.launch, false)
            .await
            .unwrap();
        assert_eq!(client.endpoint(), &daemon.launch.mcp_endpoint);
        drop(client);
    });
    let socket = daemon.connect();
    socket.shutdown(true).unwrap();
    daemon_process::wait_unlocked(&daemon.launch.paths.app_data_dir, Duration::from_secs(10))
        .unwrap();
}

#[test]
fn concurrent_app_starters_and_cli_share_one_daemon() {
    let daemon = Daemon::new();
    let mut app = daemon.launch.clone();
    app.app = true;
    let hash = daemon.hash.clone();
    let first = app.clone();
    let second = app.clone();
    let hash_b = hash.clone();
    let a = std::thread::spawn(move || first.connect_or_spawn(&hash).unwrap());
    let b = std::thread::spawn(move || second.connect_or_spawn(&hash_b).unwrap());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let cli = rt
        .block_on(runner_cli::client::SocketClient::connect_or_start(
            &daemon.launch,
            false,
        ))
        .unwrap();
    let a = a.join().unwrap();
    let b = b.join().unwrap();
    assert_eq!(a.welcome.pid, b.welcome.pid);
    assert_eq!(a.welcome.pid, daemon.connect().welcome.pid);
    drop(cli);
    a.shutdown(true).unwrap();
    daemon_process::wait_unlocked(&daemon.launch.paths.app_data_dir, Duration::from_secs(10))
        .unwrap();
}

#[cfg(unix)]
#[test]
fn cat_echo_and_da1_reply_once_with_zero_and_two_mirrors() {
    use runner_terminal::terminal::TerminalBridge;
    let mut daemon = Daemon::new();
    let socket = daemon.start();
    let client = socket.client();
    let other = daemon.connect();
    let a = TerminalBridge::new(client.clone(), Arc::new(|| {})).unwrap();
    let b = TerminalBridge::new(other.client(), Arc::new(|| {})).unwrap();
    for mirrors in [0, 2] {
        let shell = daemon.shell(&client);
        let reply = daemon.root.path().join(format!("da1-{mirrors}"));
        let mut held = Vec::new();
        if mirrors == 2 {
            held.push(a.attach(&shell.id).unwrap());
            held.push(b.attach(&shell.id).unwrap());
        }
        let path = reply.to_str().unwrap().replace('\'', "'\\''");
        let script = format!("stty raw -echo; printf '\\033[c'; dd bs=1 count=5 2>/dev/null | od -An -tx1 >'{path}'; sleep 0.2; stty min 0 time 1; dd bs=1 count=100 2>/dev/null | od -An -tx1 >>'{path}'; stty min 1 time 0; printf '\\r\\nDA_READY\\r\\n'; exec cat\n");
        let mut frames = client.attach(&shell.id).unwrap().frames;
        client.input(&shell.id, script.as_bytes()).unwrap();
        output_until(&mut frames, b"\r\nDA_READY\r\n");
        assert_eq!(
            std::fs::read_to_string(reply)
                .unwrap()
                .split_whitespace()
                .collect::<Vec<_>>(),
            ["1b", "5b", "3f", "36", "63"]
        );
        client.input(&shell.id, b"CAT_SOCKET_ECHO").unwrap();
        output_until(&mut frames, b"CAT_SOCKET_ECHO");
        client.session_close(&shell.id).unwrap();
        drop(held);
    }
    socket.shutdown(true).unwrap();
    daemon.stopped();
}

#[test]
fn mismatch_restart_waits_for_lock_and_installs_new_sidecar() {
    use std::io::{Seek, SeekFrom, Write};
    let mut daemon = Daemon::new();
    // Trailing bytes change the hash without changing the executable image.
    for path in [
        daemon.launch.source.clone(),
        daemon
            .launch
            .source
            .parent()
            .unwrap()
            .join(runner_core::cli_install::DAEMON_DEST_BIN_NAME),
    ] {
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(b"645-oldbin")
            .unwrap();
    }
    daemon.hash = daemon_process::executable_hash(&daemon.launch.source).unwrap();
    let old = daemon.start();
    let original_pid = old.welcome.pid;
    let mut source = std::fs::OpenOptions::new()
        .write(true)
        .open(&daemon.launch.source)
        .unwrap();
    source.seek(SeekFrom::End(-10)).unwrap();
    source.write_all(b"645-newbin").unwrap();
    source
        .set_times(
            std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
        )
        .unwrap();
    drop(source);
    let hash = daemon_process::executable_hash(&daemon.launch.source).unwrap();
    assert_ne!(hash, daemon.hash);
    let mut app = daemon.launch.clone();
    app.app = true;
    let replacement = app.connect_or_spawn(&hash).unwrap();
    assert_ne!(replacement.welcome.pid, original_pid);
    assert_eq!(replacement.welcome.exe_sha256, hash);
    assert_eq!(
        daemon_process::executable_hash(
            &app.paths
                .app_data_dir
                .join("bin")
                .join(runner_core::cli_install::DAEMON_DEST_BIN_NAME)
        )
        .unwrap(),
        hash
    );
    replacement.shutdown(true).unwrap();
    daemon.stopped();
}

#[test]
fn managed_connection_restarts_three_times_then_stops() {
    use runner_core::protocol::managed::ManagedTransport;
    let daemon = Daemon::new();
    let managed = ManagedTransport::connect(daemon.launch.clone(), daemon.hash.clone()).unwrap();
    for restart in 0..4 {
        let old = daemon.connect();
        let pid = old.welcome.pid;
        #[cfg(unix)]
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::{
                OpenProcess, TerminateProcess, PROCESS_TERMINATE,
            };
            let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
            assert!(!process.is_null());
            assert_ne!(TerminateProcess(process, 1), 0);
            CloseHandle(process);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        if restart < 3 {
            loop {
                if let Ok(new) = SocketTransport::connect(
                    &daemon.launch.daemon_endpoint,
                    daemon.hello(&daemon.hash),
                ) {
                    if new.welcome.pid != pid && managed.client().role_list().is_ok() {
                        break;
                    }
                }
                assert!(Instant::now() < deadline, "managed reconnect timed out");
                std::thread::sleep(Duration::from_millis(20));
            }
        } else {
            while !old.is_closed() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(20));
            }
            std::thread::sleep(Duration::from_millis(300));
            assert!(managed.client().role_list().is_err());
            assert!(matches!(
                SocketTransport::connect(
                    &daemon.launch.daemon_endpoint,
                    daemon.hello(&daemon.hash)
                ),
                Err(ConnectError::NotRunning)
            ));
        }
    }
}

#[test]
fn slow_sqlite_request_does_not_block_another_request() {
    let mut daemon = Daemon::new();
    let socket = daemon.start();
    let pool =
        runner_daemon::db::open_pool(&daemon.launch.paths.app_data_dir.join("runner.db")).unwrap();
    let conn = pool.get().unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    let client = socket.client();
    let slow = std::thread::spawn(move || client.session_rename("missing", Some("title".into())));
    std::thread::sleep(Duration::from_millis(100));
    assert!(!slow.is_finished());
    let start = Instant::now();
    assert!(socket.client().window_snapshot().unwrap().is_empty());
    assert!(start.elapsed() < Duration::from_millis(500));
    conn.execute_batch("ROLLBACK").unwrap();
    assert!(slow
        .join()
        .unwrap()
        .unwrap_err()
        .message
        .contains("session not found"));
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    let client = socket.client();
    let slow_window =
        std::thread::spawn(move || client.node_mark_viewed("slow", "missing", Vec::new(), None));
    std::thread::sleep(Duration::from_millis(100));
    assert!(!slow_window.is_finished());
    let start = Instant::now();
    socket.client().window_register("fast").unwrap();
    assert!(start.elapsed() < Duration::from_millis(500));
    conn.execute_batch("ROLLBACK").unwrap();
    assert!(slow_window.join().unwrap().unwrap().is_none());
    socket.shutdown(true).unwrap();
    daemon.stopped();
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}
#[cfg(windows)]
fn process_exists(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut code = 0;
        let alive = GetExitCodeProcess(process, &mut code) != 0 && code == 259;
        CloseHandle(process);
        alive
    }
}

#[cfg(unix)]
#[test]
#[ignore = "manual socket terminal latency and throughput gate"]
fn socket_terminal_benchmark() {
    use runner_terminal::replay::visible_lines;
    use runner_terminal::terminal::{TerminalBridge, TerminalMirror};
    use std::io::Write;
    use std::sync::{Condvar, Mutex};
    #[derive(Default)]
    struct Wake {
        tick: Mutex<u64>,
        changed: Condvar,
    }
    impl Wake {
        fn notify(&self) {
            *self.tick.lock().unwrap() += 1;
            self.changed.notify_all();
        }
        fn until(&self, timeout: Duration, ready: impl Fn() -> bool) {
            let deadline = Instant::now() + timeout;
            loop {
                let observed = *self.tick.lock().unwrap();
                if ready() {
                    return;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(!left.is_zero(), "benchmark timeout");
                let tick = self.tick.lock().unwrap();
                drop(
                    self.changed
                        .wait_timeout_while(tick, left, |tick| *tick == observed)
                        .unwrap(),
                );
            }
        }
    }
    fn contains(mirror: &TerminalMirror, marker: &str) -> bool {
        visible_lines(&*mirror.term.lock())
            .iter()
            .any(|line| line.trim() == marker)
    }
    let mut daemon = Daemon::new();
    let socket = daemon.start();
    let client = socket.client();
    let wake = Arc::new(Wake::default());
    let notify = wake.clone();
    let bridge = TerminalBridge::new(client.clone(), Arc::new(move || notify.notify())).unwrap();
    let cat = |script: &str| {
        let shell = daemon.shell(&client);
        let mirror = bridge.attach(&shell.id).unwrap();
        let view = mirror.view();
        client
            .input(&shell.id, format!("{script}\n").as_bytes())
            .unwrap();
        wake.until(Duration::from_secs(10), || contains(&mirror, "BENCH_READY"));
        drop(view);
        mirror
    };
    let echo = cat("stty raw -echo; printf 'BENCH_READY\\r\\n'; exec cat");
    let echo_view = echo.view();
    let mut samples = Vec::with_capacity(10_000);
    for i in 0..10_000 {
        let character = char::from(b'A' + (i % 26) as u8);
        let before = echo.output_activity().last_seq;
        let start = Instant::now();
        echo.send_text(&character.to_string()).unwrap();
        wake.until(Duration::from_secs(5), || {
            echo.output_activity().last_seq > before
        });
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        let term = echo.term.lock();
        let cursor = &term.grid().cursor;
        let mut point = cursor.point;
        if !cursor.input_needs_wrap {
            point.column.0 -= 1;
        }
        assert_eq!(term.grid()[point].c, character, "cat echo {i}");
    }
    samples.sort_by(f64::total_cmp);
    let mut burst_file = tempfile::NamedTempFile::new_in(daemon.root.path()).unwrap();
    const BYTES: usize = 50 * 1024 * 1024;
    let mut line = vec![b'x'; 79];
    line.push(b'\n');
    let block = line.repeat(8192);
    for _ in 0..BYTES / block.len() {
        burst_file.write_all(&block).unwrap();
    }
    burst_file.write_all(&block[..BYTES % block.len()]).unwrap();
    burst_file.flush().unwrap();
    let path = burst_file.path().to_str().unwrap().replace('\'', "'\\''");
    let burst=cat(&format!("stty raw -echo; printf 'BENCH_READY\\r\\n'; read start; cat '{path}'; printf '\\r\\nBENCH_DONE\\r\\n'; exec cat"));
    let burst_view = burst.view();
    let start = Instant::now();
    burst.write_user_bytes(b"\n").unwrap();
    wake.until(Duration::from_secs(180), || contains(&burst, "BENCH_DONE"));
    let elapsed = start.elapsed().as_secs_f64();
    println!("socket_terminal_benchmark platform={} samples={} echo_p50_ms={:.6} echo_p99_ms={:.6} burst_bytes={} burst_seconds={:.6} burst_mib_per_second={:.6}",std::env::consts::OS,samples.len(),samples[4999],samples[9899],BYTES,elapsed,50.0/elapsed);
    drop((echo_view, burst_view));
    client.session_close(echo.session_id()).unwrap();
    client.session_close(burst.session_id()).unwrap();
    socket.shutdown(true).unwrap();
    daemon.stopped();
}

#[test]
#[ignore = "manual main-thread socket round-trip inventory"]
fn socket_request_latencies() {
    use runner_core::protocol::{Request, Response, Transport};
    use serde_json::json;
    let mut daemon = Daemon::new();
    let socket = daemon.start();
    let home = daemon.launch.paths.home_dir.as_ref().unwrap();
    let inputs = json!({"home":home,"login_path":"","system_path":"","sidecar":daemon.launch.source,"local_bin":home.join("bin"),"system_bin":home.join("system-bin"),"system_bin_writable":false,"debug":true,"platform":if cfg!(windows) {"Windows"} else {"Unix"}});
    let fixtures = [
        (
            "agent_skill_install_root",
            json!({"root":home,"debug":false,"content":"missing"}),
        ),
        ("agent_skill_remove", json!({"home":home,"debug":false})),
        (
            "agent_skill_status_root",
            json!({"root":home,"debug":false,"expected":"missing"}),
        ),
        ("app_woke", json!({})),
        ("command_directory_writable", json!({"path":home})),
        (
            "command_install_default",
            json!({"inputs":inputs,"system":false}),
        ),
        ("command_status", json!({"inputs":inputs,"system":false})),
        ("crew_list_all", json!({})),
        ("file_link_environment", json!({})),
        ("mark_blurred", json!({"label":"missing"})),
        ("mark_direct_sessions_viewed", json!({"session_ids":[]})),
        ("mark_focused", json!({"label":"missing"})),
        (
            "mcp_client_status",
            json!({"client":"ClaudeCode","path":home,"binary":"missing"}),
        ),
        (
            "mcp_remove_runner_entry",
            json!({"client":"ClaudeCode","path":home,"binary":"missing"}),
        ),
        (
            "mcp_validate_edit",
            json!({"client":"ClaudeCode","name":"missing","text":"{}","also":[]}),
        ),
        ("mission_grid_hint_set", json!({"cols":80,"rows":24})),
        ("mission_list_summary_impl", json!({"crew_id":null})),
        (
            "mission_rename_impl",
            json!({"id":"missing","title":"missing"}),
        ),
        ("node_list", json!({})),
        (
            "node_mark_viewed",
            json!({"window_label":"missing","id":"missing","member_ids":[],"viewed_session_id":null}),
        ),
        (
            "node_mission_layout_set",
            json!({"node_id":"missing","layout":"{}"}),
        ),
        (
            "node_move",
            json!({"id":"missing","parent_id":null,"ordered_ids":[]}),
        ),
        ("node_rename", json!({"id":"missing","name":"missing"})),
        ("node_reorder_pinned", json!({"ordered_ids":[]})),
        ("node_set_pinned", json!({"id":"missing","pinned":false})),
        ("node_tab_delete", json!({"node_id":"missing"})),
        (
            "node_tab_upsert",
            json!({"input":{"id":"bench-tab","parent_id":null,"name":"Bench","layout":"{}"}}),
        ),
        ("project_create", json!({"name":"missing","cwd":home})),
        ("project_delete", json!({"id":"missing"})),
        ("project_list", json!({})),
        ("project_rename", json!({"id":"missing","name":"missing"})),
        (
            "report_subjects",
            json!({"label":"missing","subjects":[],"viewed_session_id":null}),
        ),
        ("role_list", json!({})),
        ("runtime_catalog", json!({})),
        ("runtime_check_updates", json!({"force":false})),
        ("runtime_refresh_models", json!({"runtimes":[]})),
        ("runtime_request_models", json!({"runtimes":[]})),
        ("runtime_status_list", json!({})),
        ("session_close", json!({"session_id":"missing"})),
        ("session_details", json!({})),
        ("session_get", json!({"session_id":"missing"})),
        ("session_list_recent_direct", json!({})),
        (
            "session_rename",
            json!({"session_id":"missing","title":null}),
        ),
        (
            "session_shell_has_foreground_process",
            json!({"session_id":"missing"}),
        ),
        (
            "session_start_direct_with_speed",
            json!({"role_id":"missing","runtime":null,"model":null,"effort":null,"speed":null,"scope":"Root","cwd":home,"cols":80,"rows":24}),
        ),
        (
            "session_start_runtime_with_speed",
            json!({"runtime":"invalid","scope":"Root","cwd":home,"cols":80,"rows":24,"model":null,"effort":null,"speed":null}),
        ),
        (
            "session_start_shell_in",
            json!({"scope":"Root","cwd":home,"cols":80,"rows":24}),
        ),
        ("session_status_snapshot", json!({})),
        (
            "session_take_resume_on_launch",
            json!({"session_id":"missing"}),
        ),
        ("unregister", json!({"label":"missing"})),
        ("usage_refresh", json!({"reason":"Button"})),
        ("usage_set_enabled", json!({"runtimes":[]})),
        ("window_register", json!({"label":"missing"})),
        (
            "window_set_mission",
            json!({"label":"missing","id":"missing","focused":false}),
        ),
    ];
    for (name, args) in fixtures {
        let mut samples = Vec::new();
        let mut errors = 0;
        for _ in 0..100 {
            let request: Request = serde_json::from_value(json!({name:args})).unwrap();
            let start = Instant::now();
            let response = socket.call(request).unwrap();
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
            if let Response::session_start_shell_in(Ok(shell)) = &response {
                socket.client().session_close(&shell.id).unwrap();
            }
            let result = serde_json::to_value(response).unwrap();
            if result
                .get(name)
                .and_then(|result| result.get("Err"))
                .is_some()
            {
                errors += 1;
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "request_latency name={name} samples=100 errors={errors} p50_ms={:.6} p99_ms={:.6}",
            samples[49], samples[98]
        );
    }
    socket.shutdown(true).unwrap();
    daemon.stopped();
}

#[test]
fn stopping_during_startup_preserves_every_resume_claim() {
    let mut daemon = Daemon::new();
    let socket = daemon.start();
    for _ in 0..12 {
        daemon.shell(&socket.client());
    }
    socket.shutdown(true).unwrap();
    daemon.stopped();
    drop(socket);
    for _ in 0..3 {
        let resumed = daemon.start();
        resumed.shutdown(true).unwrap();
        daemon.stopped();
        let pool =
            runner_daemon::db::open_pool(&daemon.launch.paths.app_data_dir.join("runner.db"))
                .unwrap();
        let pending: i64 = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM sessions WHERE resume_on_launch != 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            pending, 12,
            "stopping during resume must preserve all claims"
        );
    }
}

#[test]
fn disconnect_removes_owned_windows_and_preserves_other_clients() {
    use runner_core::protocol::window::Subject;
    let mut daemon = Daemon::new();
    let crashed = daemon.start();
    let survivor = daemon.connect();
    let client = crashed.client();
    client.window_register("crashed").unwrap();
    client
        .report_subjects(
            "crashed",
            vec![Subject::DirectChat("chat-1".into())],
            Some("chat-1"),
        )
        .unwrap();
    client.mark_focused("crashed").unwrap();
    client.window_register("replaced").unwrap();
    survivor.client().window_register("survivor").unwrap();
    survivor.client().window_register("replaced").unwrap();
    assert!(client.mark_focused("replaced").is_err());
    drop(client);
    drop(crashed);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let windows = survivor.client().window_snapshot().unwrap();
        if windows.len() == 2 {
            assert_eq!(
                windows
                    .iter()
                    .map(|entry| entry.label.as_str())
                    .collect::<Vec<_>>(),
                vec!["replaced", "survivor"]
            );
            assert!(windows
                .iter()
                .all(|entry| !entry.focused && entry.viewed_session_id.is_none()));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "disconnected window remains registered: {windows:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    survivor.shutdown(true).unwrap();
    daemon.stopped();
}
