use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::router;
use crate::session::runtime::SpawnSpec;

const FORK_MATERIALIZE_POLL: Duration = Duration::from_millis(25);
const CODEX_FORK_ROLLOUT_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn materialize(
    spec: &SpawnSpec,
    plan: &router::runtime::ForkPlan,
    timeout: Duration,
) -> Result<String> {
    let router::runtime::ForkPlan::Headless { args, source_key } = plan else {
        return Err(Error::msg(
            "direct fork plan cannot run as a headless materializer",
        ));
    };
    let mut args = args.clone();
    if spec
        .env
        .get(crate::runtimes::REMOTE_PLUGIN_ENV)
        .is_some_and(|value| value == "false")
    {
        args.extend(["-c".into(), "features.remote_plugin=false".into()]);
    }
    let codex_sessions_root = codex_fork_sessions_root(spec)?;
    let inherited_path = std::env::var("PATH").ok();
    let home = runner_core::app_paths::home_dir();
    let path = crate::session::launch::compose_path(
        spec.shim_dir.as_deref(),
        spec.bundled_bin_dir.as_deref(),
        spec.shell_path.as_deref(),
        home.as_deref(),
        inherited_path.as_deref(),
    );
    let mut command = Command::new(&spec.command);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PATH", path);
    if let Some(cwd) = spec.cwd.as_deref() {
        command.current_dir(cwd);
    }
    for (name, value) in &spec.env {
        if crate::session::launch::is_reserved_env_name(name) {
            continue;
        }
        if !crate::session::launch::is_valid_env_name(name) {
            return Err(Error::msg(format!(
                "invalid env var name {name:?}: must match [A-Za-z_][A-Za-z0-9_]*"
            )));
        }
        command.env(name, value);
    }
    if let Some((cols, rows)) = spec.initial_size {
        command.env("COLUMNS", cols.to_string());
        command.env("LINES", rows.to_string());
    }
    crate::session::process::prepare_headless_fork(&mut command);

    let mut child = command
        .spawn()
        .map_err(|error| Error::msg(format!("fork materialization spawn: {error}")))?;
    #[cfg(windows)]
    let process_tree =
        crate::session::process::ProcessTree::adopt(child.id()).map_err(|error| {
            let _ = child.kill();
            let _ = child.wait();
            Error::msg(format!("fork materialization adopt process tree: {error}"))
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::msg("fork materialization stdout unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::msg("fork materialization stderr unavailable"))?;

    run_thread_started_headless_fork(
        child,
        stdout,
        stderr,
        codex_sessions_root,
        source_key,
        timeout,
        #[cfg(windows)]
        process_tree,
    )
}

fn run_thread_started_headless_fork(
    mut child: std::process::Child,
    stdout: std::process::ChildStdout,
    stderr: std::process::ChildStderr,
    sessions_root: PathBuf,
    source_key: &str,
    timeout: Duration,
    #[cfg(windows)] process_tree: crate::session::process::ProcessTree,
) -> Result<String> {
    let (key_tx, key_rx) = std::sync::mpsc::sync_channel(1);
    let stdout_reader = thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        let _ = key_tx.send(read_thread_started_event(&mut stdout));
        let _ = std::io::copy(&mut stdout, &mut std::io::sink());
    });
    let stderr_reader = thread::spawn(move || {
        let mut stderr = stderr;
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let deadline = Instant::now() + timeout;
    let mut key = None;
    let mut rollout_deadline = None;
    let mut child_status = None;
    let mut failed_status = None;
    let mut terminate = false;
    let result = loop {
        if key.is_none() {
            match key_rx.try_recv() {
                Ok(Ok(received)) => {
                    key = Some(received);
                    rollout_deadline = Some(std::cmp::min(
                        deadline,
                        Instant::now() + CODEX_FORK_ROLLOUT_TIMEOUT,
                    ));
                }
                Ok(Err(error)) => {
                    terminate = child_status.is_none();
                    break Err(error);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    terminate = child_status.is_none();
                    break Err(Error::msg(
                        "fork materialization stdout reader stopped before thread.started",
                    ));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }

        if child_status.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => child_status = Some(status),
                Ok(None) => {}
                Err(error) => {
                    terminate = true;
                    break Err(Error::msg(format!(
                        "fork materialization wait failed: {error}"
                    )));
                }
            }
        }
        if child_status
            .as_ref()
            .is_some_and(|status| !status.success())
        {
            failed_status = child_status.take();
            break Err(Error::msg("fork materialization exited unsuccessfully"));
        }

        if let Some(key) = key.as_deref() {
            if super::capture::fork_rollout_is_ready(&sessions_root, key, source_key) {
                terminate = child_status.is_none();
                break Ok(key.to_string());
            }
            if rollout_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                terminate = child_status.is_none();
                break Err(Error::msg(format!(
                    "fork materialization rollout for thread {key} did not appear before the deadline"
                )));
            }
        }

        if Instant::now() >= deadline {
            terminate = child_status.is_none();
            break Err(Error::msg(format!(
                "fork materialization timed out after {} seconds",
                timeout.as_secs()
            )));
        }

        thread::sleep(FORK_MATERIALIZE_POLL);
    };

    if terminate {
        #[cfg(unix)]
        crate::session::process::kill_headless_fork(&mut child);
        #[cfg(windows)]
        crate::session::process::kill_headless_fork(&mut child, &process_tree);
    }
    // Close descendant pipe writers before joining readers, even if the launcher already exited.
    #[cfg(windows)]
    drop(process_tree);
    stdout_reader
        .join()
        .map_err(|_| Error::msg("fork materialization stdout reader panicked"))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| Error::msg("fork materialization stderr reader panicked"))?;
    if let Some(status) = failed_status {
        let stderr = String::from_utf8_lossy(&stderr);
        return Err(Error::msg(format!(
            "fork materialization exited with {status}: {}",
            stderr.trim()
        )));
    }
    result
}

fn read_thread_started_event(reader: &mut impl BufRead) -> Result<String> {
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| Error::msg(format!("fork materialization output: {error}")))?;
        if read == 0 {
            return Err(Error::msg(
                "fork materialization missing thread.started event",
            ));
        }
        if !line.trim().is_empty() {
            break;
        }
    }
    let event: serde_json::Value = serde_json::from_str(line.trim_end()).map_err(|error| {
        Error::msg(format!(
            "fork materialization first event is invalid JSON: {error}"
        ))
    })?;
    if event.get("type").and_then(serde_json::Value::as_str) != Some("thread.started") {
        return Err(Error::msg(
            "fork materialization first event is not thread.started",
        ));
    }
    let key = event
        .get("thread_id")
        .and_then(serde_json::Value::as_str)
        .filter(|key| uuid::Uuid::parse_str(key).is_ok())
        .ok_or_else(|| Error::msg("fork materialization thread.started has no valid thread_id"))?;
    Ok(key.to_string())
}

fn codex_fork_sessions_root(spec: &SpawnSpec) -> Result<PathBuf> {
    let codex_home = spec
        .env
        .get("CODEX_HOME")
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("CODEX_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| runner_core::app_paths::home_dir().map(|home| home.join(".codex")))
        .ok_or_else(|| Error::msg("fork materialization cannot resolve Codex sessions root"))?;
    let codex_home = if codex_home.is_absolute() {
        codex_home
    } else if let Some(cwd) = spec.cwd.as_deref() {
        cwd.join(codex_home)
    } else {
        std::env::current_dir()
            .map_err(|error| Error::msg(format!("fork materialization cwd: {error}")))?
            .join(codex_home)
    };
    Ok(codex_home.join("sessions"))
}
