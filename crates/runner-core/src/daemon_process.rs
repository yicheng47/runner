use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app_paths::{self, IpcEndpoint};
use crate::protocol::socket::{ConnectError, SocketTransport};
use crate::protocol::wire::Hello;
use sha2::{Digest, Sha256};

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
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::{
                CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW,
            };
            let flags = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;
            // Isolated fixtures stay in the test runner's job.
            let flags = if self.isolated {
                flags
            } else {
                flags | CREATE_BREAKAWAY_FROM_JOB
            };
            command.creation_flags(flags);
            match command.spawn() {
                Err(error) if error.raw_os_error() == Some(5) && self.app => {
                    log::warn!(
                        "runnerd breakaway refused by app launcher; retrying without breakaway"
                    );
                    command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
                    command.spawn()
                }
                result => result,
            }
        }
        #[cfg(unix)]
        command.spawn()
    }
    pub fn connect_or_spawn(&self, hash: &str) -> Result<Arc<SocketTransport>, ConnectError> {
        let hello = || Hello {
            exe_sha256: hash.to_owned(),
            client: if self.app { "app" } else { "cli" }.into(),
        };
        match SocketTransport::connect(&self.daemon_endpoint, hello()) {
            Ok(client) => return Ok(client),
            Err(ConnectError::NotRunning | ConnectError::Mismatch(_)) => (),
            Err(error) => return Err(error),
        }
        let _starter = startup_lock(&self.paths.app_data_dir)
            .map_err(|error| ConnectError::Protocol(format!("runnerd startup lock: {error}")))?;
        match SocketTransport::connect(&self.daemon_endpoint, hello()) {
            Ok(client) => return Ok(client),
            Err(ConnectError::Mismatch(_)) if self.app => {
                let old = SocketTransport::connect(
                    &self.daemon_endpoint,
                    Hello {
                        exe_sha256: String::new(),
                        client: "stop".into(),
                    },
                )?;
                old.shutdown(true)
                    .map_err(|error| ConnectError::Protocol(error.to_string()))?;
                wait_unlocked(&self.paths.app_data_dir, Duration::from_secs(10))
                    .map_err(|error| ConnectError::Protocol(error.to_string()))?;
            }
            Err(ConnectError::NotRunning) => (),
            Err(error) => return Err(error),
        }
        if self.app {
            crate::cli_install::install_from_source(&self.paths.app_data_dir, &self.source)
                .map_err(|error| ConnectError::Protocol(error.to_string()))?;
            let _ = crate::cli_install::remove_stale_mcp_cli(&self.paths.app_data_dir);
        }
        let mut child = self.spawn().map_err(|error| {
            ConnectError::Protocol(format!("start runnerd: {error}. Open Runner and retry."))
        })?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match SocketTransport::connect(&self.daemon_endpoint, hello()) {
                Ok(client) => return Ok(client),
                Err(ConnectError::NotRunning) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(error) => return Err(error),
            }
        }
    }
}
pub fn startup_lock(data: &Path) -> io::Result<File> {
    std::fs::create_dir_all(data)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(data.join("runnerd-start.lock"))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match fs2::FileExt::try_lock_exclusive(&lock) {
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
pub fn lock_file(data: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(data.join("runnerd.lock"))
}
pub fn wait_unlocked(data: &Path, timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    let lock = lock_file(data)?;
    loop {
        match fs2::FileExt::try_lock_exclusive(&lock) {
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
        if key.to_string_lossy().starts_with("RUNNER_") {
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
pub fn clean_environment(data: &Path) {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("RUNNER_") {
            std::env::remove_var(key);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        if let Ok(path) = cleaned_path(&path, data) {
            std::env::set_var("PATH", path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            result.recv_timeout(Duration::from_millis(50)),
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
        fs2::FileExt::try_lock_exclusive(&daemon).unwrap();
        let data = root.path().to_owned();
        let (done, result) = std::sync::mpsc::channel();
        let wait = std::thread::spawn(move || {
            done.send(wait_unlocked(&data, Duration::from_secs(2)))
                .unwrap();
        });
        assert!(matches!(
            result.recv_timeout(Duration::from_millis(50)),
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
