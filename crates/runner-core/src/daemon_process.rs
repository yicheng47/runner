use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app_paths::{self, IpcEndpoint};
use crate::protocol::socket::{ConnectError, SocketTransport};
use crate::protocol::wire::{Hello, Welcome, TAKEOVER_REFUSED};
use sha2::{Digest, Sha256};

pub const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(60);
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(8);
pub const RESUME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
pub const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
pub const DAEMON_STOP_TIMEOUT: Duration = Duration::from_secs(
    SHUTDOWN_TIMEOUT.as_secs()
        + RESUME_SHUTDOWN_TIMEOUT.as_secs()
        + RUNTIME_SHUTDOWN_TIMEOUT.as_secs()
        + 5,
);
// An app that quits keeping its sessions disconnects only when its process
// exits, which can trail an updater's relaunch of the new build.
pub const IN_USE_GRACE: Duration = Duration::from_secs(3);
// A starter may wait out another app and, within one stop budget, an old daemon that is slow,
// stopping or being replaced, before starting the new one.
pub const STARTUP_LOCK_TIMEOUT: Duration = Duration::from_secs(
    IN_USE_GRACE.as_secs() + DAEMON_STOP_TIMEOUT.as_secs() + DAEMON_START_TIMEOUT.as_secs() + 5,
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePaths {
    pub home_dir: Option<PathBuf>,
    pub app_data_dir: PathBuf,
    pub log_dir: PathBuf,
}
impl NativePaths {
    pub fn new(app_data_dir: PathBuf, log_dir: PathBuf) -> Self {
        Self {
            home_dir: None,
            app_data_dir,
            log_dir,
        }
    }
    pub fn for_home(home: &Path, debug: bool) -> Self {
        Self {
            home_dir: Some(home.to_owned()),
            app_data_dir: app_paths::app_data_dir_for_home(home, debug),
            log_dir: app_paths::log_dir_for_home(home, debug),
        }
    }
    pub fn resolve() -> io::Result<Self> {
        let home =
            app_paths::home_dir().ok_or_else(|| io::Error::other("home directory unavailable"))?;
        Ok(Self::for_home(&home, cfg!(debug_assertions)))
    }
}
#[derive(Clone)]
pub struct Launch {
    pub paths: NativePaths,
    pub daemon_endpoint: IpcEndpoint,
    pub mcp_endpoint: IpcEndpoint,
    pub source: PathBuf,
    pub app: bool,
    pub isolated: bool,
}
impl Launch {
    pub fn new(paths: NativePaths, source: PathBuf, app: bool) -> Self {
        Self {
            daemon_endpoint: app_paths::daemon_endpoint(
                &paths.app_data_dir,
                cfg!(debug_assertions),
            ),
            mcp_endpoint: app_paths::mcp_endpoint(&paths.app_data_dir, cfg!(debug_assertions)),
            paths,
            source,
            app,
            isolated: false,
        }
    }
    pub fn spawn(&self) -> io::Result<Child> {
        let exe = if self.app {
            self.paths
                .app_data_dir
                .join("bin")
                .join(crate::cli_install::DAEMON_DEST_BIN_NAME)
        } else {
            self.source
                .canonicalize()?
                .parent()
                .unwrap()
                .join(crate::cli_install::DAEMON_DEST_BIN_NAME)
        };
        let mut command = Command::new(exe);
        command
            .arg("--app-data-dir")
            .arg(&self.paths.app_data_dir)
            .arg("--log-dir")
            .arg(&self.paths.log_dir)
            .arg("--endpoint")
            .arg(&self.daemon_endpoint.0)
            .arg("--mcp-endpoint")
            .arg(&self.mcp_endpoint.0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(home) = &self.paths.home_dir {
            command.arg("--home-dir").arg(home).current_dir(home);
        }
        if self.isolated {
            command.arg("--isolated");
        }
        clean_command(&mut command, &self.paths.app_data_dir);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() < 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            }
        }
        #[cfg(windows)]
        {
            spawn_windows(
                &mut command,
                self.app,
                self.isolated,
                system_is_shutting_down(),
            )
        }
        #[cfg(unix)]
        command.spawn()
    }
    pub fn connect_or_spawn(&self, hash: &str) -> Result<Arc<SocketTransport>, ConnectError> {
        self.connect_or_spawn_with_restart(hash)
            .map(|(client, _)| client)
    }
    pub fn connect_or_spawn_with_restart(
        &self,
        hash: &str,
    ) -> Result<(Arc<SocketTransport>, bool), ConnectError> {
        self.connect_or_spawn_within(hash, DAEMON_STOP_TIMEOUT)
    }
    /// `stop_budget` covers both waiting out a slow or stopping daemon and replacing one.
    fn connect_or_spawn_within(
        &self,
        hash: &str,
        stop_budget: Duration,
    ) -> Result<(Arc<SocketTransport>, bool), ConnectError> {
        let mut restarted = false;
        let hello = || Hello {
            exe_sha256: hash.to_owned(),
            client: if self.app { "app" } else { "cli" }.into(),
        };
        match SocketTransport::connect(&self.daemon_endpoint, hello()) {
            Ok(client) => return Ok((client, restarted)),
            Err(ConnectError::NotRunning | ConnectError::Mismatch(_)) => (),
            Err(error) => return Err(error),
        }
        let _starter = startup_lock(&self.paths.app_data_dir)
            .map_err(|error| ConnectError::Protocol(format!("runnerd startup lock: {error}")))?;
        let in_use_deadline = Instant::now() + IN_USE_GRACE;
        let stop_deadline = in_use_deadline + stop_budget;
        loop {
            match SocketTransport::connect(&self.daemon_endpoint, hello()) {
                Ok(client) => return Ok((client, restarted)),
                // runnerd holds its lock while it serves and while it stops, so a handshake
                // that fails meanwhile came from a slow or stopping daemon, not a missing one.
                Err(ConnectError::NotRunning) if daemon_locked(&self.paths.app_data_dir) => {
                    if Instant::now() >= stop_deadline {
                        return Err(ConnectError::Protocol(format!(
                            "runnerd holds its lock but did not answer within {}s; log: {}",
                            (IN_USE_GRACE + stop_budget).as_secs(),
                            self.paths.log_dir.join("runnerd.log").display()
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(ConnectError::Mismatch(welcome)) if self.app => {
                    let refused = if welcome.app_clients == 0 {
                        self.take_over_within(
                            stop_budget
                                .min(stop_deadline.saturating_duration_since(Instant::now())),
                        )?
                    } else {
                        Some(welcome)
                    };
                    match refused {
                        None => {
                            restarted = true;
                            break;
                        }
                        Some(welcome) if Instant::now() >= in_use_deadline => {
                            return Err(ConnectError::InUse(welcome))
                        }
                        Some(_) => std::thread::sleep(Duration::from_millis(100)),
                    }
                }
                Err(ConnectError::NotRunning) => break,
                Err(error) => return Err(error),
            }
        }
        if self.app {
            crate::cli_install::install_from_source(&self.paths.app_data_dir, &self.source)
                .map_err(|error| ConnectError::Protocol(error.to_string()))?;
            let _ = crate::cli_install::remove_stale_mcp_cli(&self.paths.app_data_dir);
        }
        let mut child = self.spawn().map_err(|error| {
            ConnectError::Protocol(format!("start runnerd: {error}. Open Runner and retry."))
        })?;
        let result = (|| {
            let deadline = Instant::now() + DAEMON_START_TIMEOUT;
            loop {
                self.check_startup(&mut child)?;
                match SocketTransport::connect(&self.daemon_endpoint, hello()) {
                    Ok(client) => return Ok((client, restarted)),
                    Err(ConnectError::NotRunning) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    Err(ConnectError::NotRunning) => return Err(self.startup_timeout()),
                    Err(error) => return Err(error),
                }
            }
        })();
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        result
    }
    /// Stops the running daemon unless another app is connected to it, which
    /// runnerd decides at the stop itself. A refusal returns that daemon's welcome.
    pub fn take_over(&self) -> Result<Option<Welcome>, ConnectError> {
        self.take_over_within(DAEMON_STOP_TIMEOUT)
    }
    fn take_over_within(&self, budget: Duration) -> Result<Option<Welcome>, ConnectError> {
        let deadline = Instant::now() + budget;
        let remaining = || deadline.saturating_duration_since(Instant::now());
        let stop_timeout = |cause: &str| {
            ConnectError::Protocol(format!(
                "runnerd did not stop within {}s ({cause}); log: {}",
                budget.as_secs(),
                self.paths.log_dir.join("runnerd.log").display()
            ))
        };
        let old = SocketTransport::connect(
            &self.daemon_endpoint,
            Hello {
                exe_sha256: String::new(),
                client: "stop".into(),
            },
        )?;
        #[cfg(windows)]
        let old_process = OldProcess::open(old.welcome.pid);
        let mut events = old.event_stream();
        old.take_over()
            .map_err(|error| ConnectError::Protocol(error.to_string()))?;
        loop {
            use tokio::sync::broadcast::error::TryRecvError;
            if remaining().is_zero() {
                return Err(stop_timeout("no reply to the takeover"));
            }
            match events.try_recv() {
                Ok(event) => match event.event {
                    Some(event) if event.name == TAKEOVER_REFUSED => {
                        return Ok(Some(Welcome {
                            app_clients: event.payload["app_clients"]
                                .as_u64()
                                .map_or(1, |count| count as u32),
                            ..old.welcome.clone()
                        }))
                    }
                    Some(event) if event.name == "daemon/disconnected" => break,
                    _ => (),
                },
                Err(TryRecvError::Lagged(_)) => (),
                Err(TryRecvError::Empty) if !old.is_closed() => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(_) => break,
            }
        }
        wait_unlocked(&self.paths.app_data_dir, remaining())
            .map_err(|error| stop_timeout(&error.to_string()))?;
        #[cfg(windows)]
        if let Some(process) = old_process {
            process.wait(remaining());
        }
        Ok(None)
    }
    pub fn check_startup(&self, child: &mut Child) -> Result<(), ConnectError> {
        match child.try_wait() {
            Ok(None) => Ok(()),
            Ok(Some(status)) => Err(ConnectError::Protocol(format!(
                "runnerd exited during startup ({status}); log: {}",
                self.paths.log_dir.join("runnerd.log").display()
            ))),
            Err(error) => Err(ConnectError::Protocol(format!(
                "check runnerd startup: {error}; log: {}",
                self.paths.log_dir.join("runnerd.log").display()
            ))),
        }
    }
    pub fn startup_timeout(&self) -> ConnectError {
        ConnectError::Protocol(format!(
            "runnerd did not start within {}s; log: {}",
            DAEMON_START_TIMEOUT.as_secs(),
            self.paths.log_dir.join("runnerd.log").display()
        ))
    }
}

/// runnerd releases its lock before its process exits, and Windows keeps a
/// running executable's file from being replaced, so a mismatch restart also
/// waits for the old process before installing the new sidecar over it.
#[cfg(windows)]
struct OldProcess(std::os::windows::io::OwnedHandle);

#[cfg(windows)]
impl OldProcess {
    fn open(pid: u32) -> Option<Self> {
        use std::os::windows::io::{FromRawHandle, OwnedHandle};
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        (!handle.is_null()).then(|| Self(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }

    fn wait(&self, timeout: Duration) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0, System::Threading::WaitForSingleObject,
        };
        let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        if unsafe { WaitForSingleObject(self.0.as_raw_handle(), millis) } != WAIT_OBJECT_0 {
            log::warn!("old runnerd still running after its lock was released");
        }
    }
}

pub fn system_is_shutting_down() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SHUTTINGDOWN};
        unsafe { GetSystemMetrics(SM_SHUTTINGDOWN) != 0 }
    }
    #[cfg(not(windows))]
    false
}

#[cfg(windows)]
fn spawn_windows(
    command: &mut Command,
    app: bool,
    isolated: bool,
    shutting_down: bool,
) -> io::Result<Child> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW,
    };
    if shutting_down {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Windows is shutting down; runnerd will not be started",
        ));
    }
    let flags = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;
    // Isolated fixtures stay in the test runner's job.
    let flags = if isolated {
        flags
    } else {
        flags | CREATE_BREAKAWAY_FROM_JOB
    };
    command.creation_flags(flags);
    match command.spawn() {
        Err(error) if error.raw_os_error() == Some(5) && app => {
            log::warn!("runnerd breakaway refused by app launcher; retrying without breakaway");
            if system_is_shutting_down() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Windows is shutting down; runnerd will not be started",
                ));
            }
            command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
            command.spawn()
        }
        result => result,
    }
}

pub struct LockFile(File);

impl LockFile {
    pub fn try_lock_exclusive(&self) -> io::Result<()> {
        fs2::FileExt::try_lock_exclusive(&self.0)
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        // A forked child's descriptor can outlive ours; closing alone leaves flock held.
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

pub fn startup_lock(data: &Path) -> io::Result<LockFile> {
    std::fs::create_dir_all(data)?;
    let lock = LockFile(
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(data.join("runnerd-start.lock"))?,
    );
    let deadline = Instant::now() + STARTUP_LOCK_TIMEOUT;
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => return Ok(lock),
            Err(error)
                if error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(error) => return Err(error),
        }
    }
}
pub fn lock_file(data: &Path) -> io::Result<LockFile> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(data.join("runnerd.lock"))
        .map(LockFile)
}
fn daemon_locked(data: &Path) -> bool {
    lock_file(data)
        .and_then(|lock| lock.try_lock_exclusive())
        .is_err_and(|error| error.raw_os_error() == fs2::lock_contended_error().raw_os_error())
}
pub fn wait_unlocked(data: &Path, timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    let lock = lock_file(data)?;
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => return Ok(()),
            Err(error)
                if error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(error) => return Err(error),
        }
    }
}
pub fn executable_hash(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = [0; 64 * 1024];
    loop {
        let size = file.read(&mut buf)?;
        if size == 0 {
            break;
        }
        hash.update(&buf[..size]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn clean_command(command: &mut Command, data: &Path) {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("RUNNER_") && key != "RUNNER_RECORD_INPUT_FIXTURE" {
            command.env_remove(key);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        if let Ok(path) = cleaned_path(&path, data) {
            command.env("PATH", path);
        }
    }
}
pub fn cleaned_path(
    path: &std::ffi::OsStr,
    data: &Path,
) -> Result<std::ffi::OsString, std::env::JoinPathsError> {
    std::env::join_paths(std::env::split_paths(path).filter(|part| {
        !part.starts_with(data.join("bin")) && !part.starts_with(data.join("missions"))
    }))
}
/// # Safety
/// Call before starting any threads that could read or write the process environment.
pub unsafe fn clean_environment(data: &Path) {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("RUNNER_") && key != "RUNNER_RECORD_INPUT_FIXTURE" {
            unsafe { std::env::remove_var(key) };
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        if let Ok(path) = cleaned_path(&path, data) {
            unsafe { std::env::set_var("PATH", path) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake runnerd that answers a takeover's handshake, reads its stop, then
    /// misbehaves as `after_stop` does.
    #[cfg(unix)]
    fn stalled_takeover(
        after_stop: impl FnOnce(std::os::unix::net::UnixStream) + Send + 'static,
    ) -> (tempfile::TempDir, Launch, std::thread::JoinHandle<()>) {
        use crate::protocol::wire::{self, Frame};
        let root = tempfile::tempdir().unwrap();
        let mut launch = Launch::new(
            NativePaths::new(root.path().join("data"), root.path().join("logs")),
            root.path().join("runner"),
            true,
        );
        std::fs::create_dir_all(&launch.paths.app_data_dir).unwrap();
        launch.daemon_endpoint = IpcEndpoint(root.path().join("runnerd.sock"));
        let listener = std::os::unix::net::UnixListener::bind(&launch.daemon_endpoint.0).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            assert_eq!(Frame::read(&mut stream).unwrap().kind, wire::HELLO);
            Frame::json(
                wire::WELCOME,
                &wire::Welcome {
                    exe_sha256: "old".into(),
                    pid: 1,
                    started_at: "now".into(),
                    app_clients: 0,
                },
            )
            .unwrap()
            .write(&mut stream)
            .unwrap();
            let stop: wire::Shutdown = Frame::read(&mut stream).unwrap().decode().unwrap();
            assert!(stop.stop_sessions && stop.takeover);
            after_stop(stream);
        });
        (root, launch, server)
    }

    #[cfg(unix)]
    #[test]
    fn takeover_stays_within_one_stop_budget_while_events_keep_arriving() {
        use crate::protocol::wire::{self, Frame};
        let (_root, launch, server) = stalled_takeover(|mut stream| {
            let event = Frame::json(
                wire::EVENT,
                &wire::Event {
                    event: Some(crate::protocol::ClientEvent {
                        name: "session/output".into(),
                        payload: serde_json::Value::Null,
                    }),
                    lagged: None,
                },
            )
            .unwrap();
            while event.write(&mut stream).is_ok() {}
        });
        let budget = Duration::from_millis(300);
        let started = Instant::now();
        let error = launch.take_over_within(budget).unwrap_err().to_string();
        assert!(
            started.elapsed() < budget + Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
        assert!(error.contains("did not stop"), "{error}");
        server.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn takeover_shares_its_stop_budget_with_the_lock_wait() {
        let (_root, launch, server) = stalled_takeover(drop);
        let held = lock_file(&launch.paths.app_data_dir).unwrap();
        held.try_lock_exclusive().unwrap();
        let budget = Duration::from_millis(300);
        let started = Instant::now();
        let error = launch.take_over_within(budget).unwrap_err().to_string();
        assert!(
            started.elapsed() < budget + Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
        assert!(error.contains("did not stop"), "{error}");
        server.join().unwrap();
    }

    #[cfg(unix)]
    fn locked_launch() -> (tempfile::TempDir, Launch, LockFile) {
        let root = tempfile::tempdir().unwrap();
        let mut launch = Launch::new(
            NativePaths::new(root.path().join("data"), root.path().join("logs")),
            root.path().join("runner"),
            true,
        );
        std::fs::create_dir_all(&launch.paths.app_data_dir).unwrap();
        launch.daemon_endpoint = IpcEndpoint(root.path().join("runnerd.sock"));
        let daemon = lock_file(&launch.paths.app_data_dir).unwrap();
        daemon.try_lock_exclusive().unwrap();
        (root, launch, daemon)
    }

    /// A fake runnerd that hands every handshake to `answer` until the returned
    /// closer sends its own.
    #[cfg(unix)]
    fn serve_handshakes(
        launch: &Launch,
        mut answer: impl FnMut(crate::protocol::wire::Hello, std::os::unix::net::UnixStream)
            + Send
            + 'static,
    ) -> impl FnOnce() {
        use crate::protocol::wire::{self, Frame};
        let endpoint = launch.daemon_endpoint.0.clone();
        let listener = std::os::unix::net::UnixListener::bind(&endpoint).unwrap();
        let server = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let hello: wire::Hello = Frame::read(&mut stream).unwrap().decode().unwrap();
                if hello.client == "done" {
                    return;
                }
                answer(hello, stream);
            }
        });
        move || {
            let mut done = std::os::unix::net::UnixStream::connect(endpoint).unwrap();
            Frame::json(
                wire::HELLO,
                &wire::Hello {
                    exe_sha256: String::new(),
                    client: "done".into(),
                },
            )
            .unwrap()
            .write(&mut done)
            .unwrap();
            server.join().unwrap();
        }
    }

    /// Answers nothing until the client gives up on this connection.
    #[cfg(unix)]
    fn stall(mut stream: std::os::unix::net::UnixStream) {
        use std::io::Read;
        let _ = stream.read(&mut [0]);
    }

    #[cfg(unix)]
    fn reply(stream: &mut std::os::unix::net::UnixStream, kind: u8, app_clients: u32) {
        use crate::protocol::wire::{Frame, Welcome};
        Frame::json(
            kind,
            &Welcome {
                exe_sha256: "other".into(),
                pid: 1,
                started_at: "now".into(),
                app_clients,
            },
        )
        .unwrap()
        .write(stream)
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_slow_handshake_from_a_running_daemon_still_pauses_an_app_as_in_use() {
        let (_root, launch, _daemon) = locked_launch();
        let mut handshakes = 0;
        let finish = serve_handshakes(&launch, move |_, mut stream| {
            handshakes += 1;
            // The probe and the first handshake under the start lock miss their deadline.
            if handshakes <= 2 {
                return stall(stream);
            }
            reply(&mut stream, crate::protocol::wire::MISMATCH, 1);
        });
        match launch.connect_or_spawn("this") {
            Err(ConnectError::InUse(welcome)) => assert_eq!(welcome.app_clients, 1),
            result => panic!(
                "a slow running daemon was taken for a missing one: {:?}",
                result.map(|client| client.welcome.pid)
            ),
        }
        finish();
    }

    #[cfg(unix)]
    #[test]
    fn a_daemon_that_keeps_its_lock_without_answering_is_never_started_over() {
        let (_root, launch, _daemon) = locked_launch();
        let budget = Duration::from_millis(300);
        let started = Instant::now();
        let error = match launch.connect_or_spawn_within("this", budget) {
            Err(error) => error.to_string(),
            Ok((client, _)) => panic!("connected to {}", client.welcome.pid),
        };
        assert!(error.contains("did not answer"), "{error}");
        assert!(started.elapsed() < IN_USE_GRACE + budget + Duration::from_secs(1));
        assert!(
            !launch.paths.app_data_dir.join("bin").exists(),
            "a second daemon was installed while the first held its lock"
        );
    }

    #[cfg(unix)]
    #[test]
    fn waiting_out_a_slow_daemon_and_replacing_it_share_one_stop_budget() {
        use crate::protocol::wire::{self, Frame};
        let (_root, launch, _daemon) = locked_launch();
        let budget = Duration::from_secs(4);
        // The daemon stays silent into the stop budget, leaving its takeover about a second.
        let answers_from = Instant::now() + IN_USE_GRACE + budget - Duration::from_secs(1);
        let (stopping, takeover) = std::sync::mpsc::channel();
        let finish = serve_handshakes(&launch, move |hello, mut stream| {
            if hello.client == "stop" {
                stopping.send(Instant::now()).unwrap();
                reply(&mut stream, wire::WELCOME, 0);
                let stop: wire::Shutdown = Frame::read(&mut stream).unwrap().decode().unwrap();
                assert!(stop.takeover);
                stall(stream)
            } else if Instant::now() < answers_from {
                stall(stream)
            } else {
                reply(&mut stream, wire::MISMATCH, 0)
            }
        });
        let error = match launch.connect_or_spawn_within("this", budget) {
            Err(error) => error.to_string(),
            Ok((client, _)) => panic!("connected to {}", client.welcome.pid),
        };
        assert!(error.contains("did not stop"), "{error}");
        let takeover = takeover
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .elapsed();
        assert!(
            takeover < budget - Duration::from_millis(1500),
            "the takeover started a fresh stop budget: {takeover:?}"
        );
        finish();
    }

    #[test]
    fn daemon_command_preserves_recording_without_mission_context() {
        const TEST_PROCESS: &str = "DAEMON_CLEAN_ENV_TEST";
        const RECORDING: &str = "RUNNER_RECORD_INPUT_FIXTURE";
        if std::env::var_os(TEST_PROCESS).is_some() {
            let data = Path::new("test-data");
            let mut command = Command::new("runnerd");
            clean_command(&mut command, data);
            let removals = command
                .get_envs()
                .filter_map(|(key, value)| value.is_none().then_some(key))
                .collect::<Vec<_>>();
            assert!(removals.contains(&std::ffi::OsStr::new("RUNNER_MISSION_ID")));
            assert!(!removals.contains(&std::ffi::OsStr::new(RECORDING)));
            return;
        }
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "daemon_process::tests::daemon_command_preserves_recording_without_mission_context",
                "--nocapture",
            ])
            .env(TEST_PROCESS, "1")
            .env(RECORDING, "fixture-prefix")
            .env("RUNNER_MISSION_ID", "parent-mission")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_shutdown_prevents_daemon_process_creation() {
        let root = tempfile::tempdir().unwrap();
        let mut command = Command::new(root.path().join("missing-runnerd.exe"));
        let error = spawn_windows(&mut command, true, true, true).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(error.to_string().contains("Windows is shutting down"));
        assert_eq!(
            spawn_windows(&mut command, true, true, false)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn exited_child_is_reported_immediately_with_status_and_log_path() {
        let root = tempfile::tempdir().unwrap();
        let launch = Launch::new(
            NativePaths::new(root.path().join("data"), root.path().join("logs")),
            root.path().join("runner"),
            false,
        );
        #[cfg(unix)]
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exit 23"])
            .spawn()
            .unwrap();
        #[cfg(windows)]
        let mut child = Command::new("cmd.exe")
            .args(["/C", "exit 23"])
            .spawn()
            .unwrap();
        let status = child.wait().unwrap();
        let started = Instant::now();
        let error = launch.check_startup(&mut child).unwrap_err().to_string();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(error.contains(&status.to_string()), "{error}");
        assert!(
            error.contains(
                &launch
                    .paths
                    .log_dir
                    .join("runnerd.log")
                    .display()
                    .to_string()
            ),
            "{error}"
        );
    }

    #[test]
    fn lifecycle_deadlines_cover_start_and_teardown() {
        assert!(DAEMON_START_TIMEOUT > Duration::from_secs(10));
        assert!(
            DAEMON_STOP_TIMEOUT
                > SHUTDOWN_TIMEOUT + RESUME_SHUTDOWN_TIMEOUT + RUNTIME_SHUTDOWN_TIMEOUT
        );
        assert!(STARTUP_LOCK_TIMEOUT > IN_USE_GRACE + DAEMON_START_TIMEOUT + DAEMON_STOP_TIMEOUT);
    }

    #[test]
    fn startup_lock_waits_for_the_active_starter() {
        let root = tempfile::tempdir().unwrap();
        let first = startup_lock(root.path()).unwrap();
        let data = root.path().to_owned();
        let (done, result) = std::sync::mpsc::channel();
        let next = std::thread::spawn(move || {
            done.send(startup_lock(&data)).unwrap();
        });
        assert!(matches!(
            result.recv_timeout(Duration::from_secs(11)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        drop(first);
        let second = result
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        next.join().unwrap();
        drop(second);
    }

    #[test]
    fn shutdown_waits_for_the_daemon_lock_to_be_released() {
        let root = tempfile::tempdir().unwrap();
        let daemon = lock_file(root.path()).unwrap();
        daemon.try_lock_exclusive().unwrap();
        let data = root.path().to_owned();
        let (done, result) = std::sync::mpsc::channel();
        let wait = std::thread::spawn(move || {
            done.send(wait_unlocked(&data, DAEMON_STOP_TIMEOUT))
                .unwrap();
        });
        assert!(matches!(
            result.recv_timeout(Duration::from_secs(11)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        drop(daemon);
        result
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        wait.join().unwrap();
    }
}
