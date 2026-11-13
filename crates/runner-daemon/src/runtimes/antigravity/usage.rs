//! Non-interactive usage fetching for Antigravity CLI (`agy`).
//!
//! Running `agy -p /usage` without confirming authentication first can make a
//! signed-out CLI open browser OAuth (`printmode.interactiveInput` passes if
//! stdin satisfies Go's `fs.ModeCharDevice`, which includes `/dev/null` and
//! `NUL`, or falls back to controlling terminal discovery on `/dev/tty` /
//! `CONIN$`), and pre-`1.5.8` builds have not been verified to honor the bare
//! auth-check gate. To uphold the zero-login guarantee:
//! 1. Version gate runs `agy --version` with a closed-pipe `stdin`
//!    (`!ModeCharDevice`) and session/console isolation (`setsid()` on Unix,
//!    `DETACHED_PROCESS` on Windows). Missing/unparseable output, non-zero exit,
//!    timeout, or version `< 1.5.8` fails closed (`AntigravityNoAnswer`)
//!    without invoking bare `agy`.
//! 2. Preflight runs bare `agy` with `AGY_CLI_CDE_AUTH_ACTION=check`, a
//!    closed-pipe `stdin`, and `setsid()` / `DETACHED_PROCESS`. This checks
//!    OS credentials and `~/.gemini` state without starting printmode or
//!    browser OAuth (`exit 0` signed in, `exit 1` signed out on macOS `1.5.8`).
//! 3. Only when version >= `1.5.8` and preflight exits `0` does Runner spawn
//!    `agy -p /usage --output-format json` with closed-pipe `stdin` and
//!    `setsid()` / `DETACHED_PROCESS`.
//!
//! Cross-platform note: `agy 1.5.8`, `AGY_CLI_CDE_AUTH_ACTION=check`,
//! closed-pipe `stdin`, and POSIX `setsid()` were live-verified on macOS.
//! Windows console detachment (`DETACHED_PROCESS`, without `CREATE_NO_WINDOW`)
//! and non-char `stdin` are covered by the Windows unit helper test (to be
//! executed by Windows CI); native Windows and Linux `agy` binary execution
//! were not live-tested on device.

use crate::runtime_status::direct_chat_path;
use crate::runtime_status::versions::{below_minimum, parse_version};
use crate::session::process::{kill_headless_fork, ProcessTree};
use crate::shell_path::LoginShellEnv;
use crate::usage::{parse_timestamp, UnavailableReason, UsageReading, UsageWindow};
use serde_json::Value;
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const MIN_USAGE_VERSION: &str = "1.5.8";
const AUTH_CHECK_TIMEOUT: Duration = Duration::from_secs(4);
const USAGE_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const READER_JOIN_GRACE: Duration = Duration::from_millis(500);

#[derive(Debug)]
struct CommandOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Debug)]
enum CommandError {
    Spawn,
    Timeout,
}

pub(crate) fn fetch_antigravity(
    command: &str,
    env: &LoginShellEnv,
) -> Result<UsageReading, UnavailableReason> {
    fetch_antigravity_with_timeouts(command, env, AUTH_CHECK_TIMEOUT, USAGE_TIMEOUT)
}

pub(crate) fn fetch_antigravity_with_timeouts(
    command: &str,
    env: &LoginShellEnv,
    auth_timeout: Duration,
    usage_timeout: Duration,
) -> Result<UsageReading, UnavailableReason> {
    verify_supported_version(command, env, auth_timeout)?;
    verify_signed_in(command, env, auth_timeout)?;

    let mut process = build_base_command(command, env);
    process.args(["-p", "/usage", "--output-format", "json"]);
    #[cfg(test)]
    if crate::golden::record_command(&process, None, usage_timeout) {
        return Err(UnavailableReason::AntigravityNoAnswer);
    }
    prepare_isolated_fork(&mut process);

    let output = run_command_with_timeout(process, usage_timeout).map_err(|err| match err {
        CommandError::Spawn | CommandError::Timeout => UnavailableReason::AntigravityNoAnswer,
    })?;

    if !output.status.success() {
        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if looks_like_sign_in(&combined) {
            return Err(UnavailableReason::SignIn);
        }
        return Err(UnavailableReason::AntigravityNoAnswer);
    }

    parse_antigravity_output(&output.stdout).map(Into::into)
}

fn verify_supported_version(
    command: &str,
    env: &LoginShellEnv,
    timeout: Duration,
) -> Result<(), UnavailableReason> {
    let mut process = build_base_command(command, env);
    process.arg("--version");
    #[cfg(test)]
    if crate::golden::record_command(&process, None, timeout) {
        return Ok(());
    }
    prepare_isolated_fork(&mut process);

    let output = run_command_with_timeout(process, timeout).map_err(|err| match err {
        CommandError::Spawn | CommandError::Timeout => UnavailableReason::AntigravityNoAnswer,
    })?;
    if !output.status.success() {
        return Err(UnavailableReason::AntigravityNoAnswer);
    }
    let Some(version) = parse_version(&output.stdout) else {
        return Err(UnavailableReason::AntigravityNoAnswer);
    };
    if below_minimum(Some(version.as_str()), Some(MIN_USAGE_VERSION)) {
        return Err(UnavailableReason::AntigravityNoAnswer);
    }
    Ok(())
}

fn verify_signed_in(
    command: &str,
    env: &LoginShellEnv,
    timeout: Duration,
) -> Result<(), UnavailableReason> {
    // Bare `agy` with `AGY_CLI_CDE_AUTH_ACTION=check` evaluates `len(os.Args) <= 1`
    // before TUI, printmode, or browser OAuth setup.
    let mut process = build_base_command(command, env);
    process.env("AGY_CLI_CDE_AUTH_ACTION", "check");
    #[cfg(test)]
    if crate::golden::record_command(&process, None, timeout) {
        return Ok(());
    }
    prepare_isolated_fork(&mut process);

    let output = run_command_with_timeout(process, timeout).map_err(|err| match err {
        CommandError::Spawn | CommandError::Timeout => UnavailableReason::AntigravityNoAnswer,
    })?;

    if output.status.success() {
        return Ok(());
    }

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.code() == Some(1) || looks_like_sign_in(&combined) {
        Err(UnavailableReason::SignIn)
    } else {
        Err(UnavailableReason::AntigravityNoAnswer)
    }
}

fn build_base_command(command: &str, env: &LoginShellEnv) -> Command {
    let mut process = Command::new(command);
    process
        .envs(&env.vars)
        .env_remove("AGY_CLI_CDE_AUTH_ACTION")
        .env("PATH", direct_chat_path(env))
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true")
        // Pipe stdin and drop the write handle right after spawn so stdin is a
        // closed pipe rather than /dev/null or NUL, which satisfy ModeCharDevice.
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(home) = runner_core::app_paths::home_dir() {
        process.current_dir(home);
    }
    process
}

#[cfg(unix)]
fn prepare_isolated_fork(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // Start a dedicated session (`setsid`) so `pgid == pid` for cleanup and
    // `/dev/tty` is detached, preventing `open("/dev/tty")` login prompts.
    // Do not combine with `process_group(0)`, which runs earlier and makes
    // `setsid()` fail with `EPERM`.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(windows)]
fn prepare_isolated_fork(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::DETACHED_PROCESS;

    // DETACHED_PROCESS prevents inheriting any parent console so CONIN$ cannot
    // open for interactive fallback. Do not combine with CREATE_NO_WINDOW,
    // which Windows ignores when DETACHED_PROCESS is set.
    command.creation_flags(DETACHED_PROCESS);
}

#[cfg(not(any(unix, windows)))]
fn prepare_isolated_fork(command: &mut Command) {
    crate::session::process::prepare_headless_fork(command);
}

#[derive(Clone, Copy)]
enum PipeStream {
    Stdout,
    Stderr,
}

fn spawn_pipe_reader(
    mut pipe: impl Read + Send + 'static,
    stream: PipeStream,
    tx: mpsc::Sender<(PipeStream, Vec<u8>)>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        let _ = tx.send((stream, buf));
    })
}

fn record_pipe_bytes(
    stream: PipeStream,
    bytes: Vec<u8>,
    stdout: &mut Option<Vec<u8>>,
    stderr: &mut Option<Vec<u8>>,
) {
    match stream {
        PipeStream::Stdout => *stdout = Some(bytes),
        PipeStream::Stderr => *stderr = Some(bytes),
    }
}

fn drain_pipes_bounded(
    rx: &mpsc::Receiver<(PipeStream, Vec<u8>)>,
    stdout: &mut Option<Vec<u8>>,
    stderr: &mut Option<Vec<u8>>,
    grace: Duration,
) {
    let deadline = Instant::now() + grace;
    while stdout.is_none() || stderr.is_none() {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        match rx.recv_timeout(deadline.saturating_duration_since(now)) {
            Ok((stream, bytes)) => record_pipe_bytes(stream, bytes, stdout, stderr),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                stdout.get_or_insert_with(Vec::new);
                stderr.get_or_insert_with(Vec::new);
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => break,
        }
    }
}

fn run_command_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<CommandOutput, CommandError> {
    let mut child = command.spawn().map_err(|_| CommandError::Spawn)?;
    // Drop stdin immediately so spawned children observe a closed pipe instead
    // of keeping write-side handles open or hitting ModeCharDevice on nul/null.
    drop(child.stdin.take());
    let tree = ProcessTree::adopt(child.id()).ok();
    let (tx, rx) = mpsc::channel();
    let stdout_handle = spawn_pipe_reader(
        child.stdout.take().expect("piped stdout"),
        PipeStream::Stdout,
        tx.clone(),
    );
    let stderr_handle = spawn_pipe_reader(
        child.stderr.take().expect("piped stderr"),
        PipeStream::Stderr,
        tx,
    );

    let deadline = Instant::now() + timeout;
    let mut exit_status: Option<ExitStatus> = None;
    let mut stdout: Option<Vec<u8>> = None;
    let mut stderr: Option<Vec<u8>> = None;
    let mut failed = false;

    loop {
        if exit_status.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => exit_status = Some(status),
                Ok(None) => {}
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        if exit_status.is_some() && stdout.is_some() && stderr.is_some() {
            break;
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let remaining = deadline.saturating_duration_since(now);
        if stdout.is_some() && stderr.is_some() {
            thread::sleep(remaining.min(POLL_INTERVAL));
            continue;
        }
        let wait = if exit_status.is_some() {
            remaining
        } else {
            remaining.min(POLL_INTERVAL)
        };
        match rx.recv_timeout(wait) {
            Ok((stream, bytes)) => {
                record_pipe_bytes(stream, bytes, &mut stdout, &mut stderr);
                while let Ok((stream, bytes)) = rx.try_recv() {
                    record_pipe_bytes(stream, bytes, &mut stdout, &mut stderr);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                stdout.get_or_insert_with(Vec::new);
                stderr.get_or_insert_with(Vec::new);
            }
        }
    }

    match (failed, exit_status, stdout, stderr) {
        (false, Some(status), Some(stdout), Some(stderr)) => {
            // Terminate any remaining process-group/job descendants and release
            // ProcessTree handles before returning on normal exit too.
            terminate_descendants(&child, tree);
            let _ = stdout_handle.join();
            let _ = stderr_handle.join();
            Ok(CommandOutput {
                status,
                stdout,
                stderr,
            })
        }
        (_, status, mut stdout, mut stderr) => {
            if status.is_none() {
                let active_tree = tree;
                shutdown_process(&mut child, active_tree.as_ref());
            } else {
                terminate_descendants(&child, tree);
            }
            drain_pipes_bounded(&rx, &mut stdout, &mut stderr, READER_JOIN_GRACE);
            if stdout.is_some() {
                let _ = stdout_handle.join();
            }
            if stderr.is_some() {
                let _ = stderr_handle.join();
            }
            Err(CommandError::Timeout)
        }
    }
}

fn terminate_descendants(child: &Child, tree: Option<ProcessTree>) {
    if let Some(tree) = tree {
        let _ = tree.terminate();
        return;
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
    #[cfg(not(unix))]
    let _ = child;
}

fn shutdown_process(child: &mut Child, tree: Option<&ProcessTree>) {
    #[cfg(unix)]
    {
        let _ = tree;
        kill_headless_fork(child);
    }
    #[cfg(windows)]
    {
        if let Some(tree) = tree {
            kill_headless_fork(child, tree);
        } else {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = tree;
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn looks_like_sign_in(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("not authenticated")
        || lower.contains("unauthenticated")
        || lower.contains("not logged in")
        || lower.contains("not signed in")
        || lower.contains("please log in")
        || lower.contains("please sign in")
        || lower.contains("agy login")
        || lower.contains("authentication required")
}

pub(crate) fn parse_antigravity_output(
    stdout: &[u8],
) -> Result<Vec<UsageWindow>, UnavailableReason> {
    if let Ok(value) = serde_json::from_slice::<Value>(stdout) {
        return parse_antigravity(&value);
    }
    let text = String::from_utf8_lossy(stdout);
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with('{') {
            if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
                return parse_antigravity(&value);
            }
        }
    }
    if looks_like_sign_in(&text) {
        return Err(UnavailableReason::SignIn);
    }
    Err(UnavailableReason::InvalidResponse)
}

pub(crate) fn parse_antigravity(value: &Value) -> Result<Vec<UsageWindow>, UnavailableReason> {
    if value.pointer("/command/name").and_then(Value::as_str) != Some("usage") {
        return Err(UnavailableReason::InvalidResponse);
    }
    if value.pointer("/status").and_then(Value::as_str) != Some("SUCCESS") {
        let error_text = value
            .get("error")
            .and_then(|err| {
                err.as_str()
                    .or_else(|| err.get("message").and_then(Value::as_str))
            })
            .unwrap_or_default();
        if looks_like_sign_in(error_text) {
            return Err(UnavailableReason::SignIn);
        }
        return Err(UnavailableReason::InvalidResponse);
    }

    let groups = value
        .pointer("/command/data/groups")
        .and_then(Value::as_array)
        .ok_or(UnavailableReason::InvalidResponse)?;

    let mut windows = Vec::new();
    for group in groups {
        let Some(group_name) = group
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let Some(buckets) = group.get("buckets").and_then(Value::as_array) else {
            continue;
        };

        // `agy -p /usage` emits `weekly` before `5h`, while Runner shows `5 hours`
        // above `Week` inside each pool. Emit `5 hours` first, then `Week`.
        let mut five_hour = None;
        let mut weekly = None;
        for bucket in buckets {
            let Some(window_type) = bucket.get("window").and_then(Value::as_str) else {
                continue;
            };
            let Some(remaining) = bucket.get("remaining_fraction").and_then(Value::as_f64) else {
                continue;
            };
            if !remaining.is_finite() || !(0.0..=1.0).contains(&remaining) {
                return Err(UnavailableReason::InvalidResponse);
            }
            let used_percent = ((1.0 - remaining) * 100.0).clamp(0.0, 100.0);
            let resets_at = parse_timestamp(bucket.get("reset_time"));
            match window_type {
                "5h" => {
                    five_hour = Some(UsageWindow {
                        name: format!("{group_name} · 5 hours used"),
                        used_percent,
                        resets_at,
                    });
                }
                "weekly" => {
                    weekly = Some(UsageWindow {
                        name: format!("{group_name} · Week used"),
                        used_percent,
                        resets_at,
                    });
                }
                _ => {}
            }
        }
        windows.extend(five_hour);
        windows.extend(weekly);
    }

    if windows.is_empty() {
        return Err(UnavailableReason::InvalidResponse);
    }
    Ok(windows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    #[test]
    fn parses_dual_pools_ordered_five_hours_before_week() {
        let payload = serde_json::json!({
            "command": {
                "name": "usage",
                "data": {
                    "groups": [
                        {
                            "name": "Gemini Models",
                            "buckets": [
                                {"window": "weekly", "remaining_fraction": 0.52, "reset_time": "2026-11-18T00:00:00Z"},
                                {"window": "5h", "remaining_fraction": 0.82, "reset_time": "2026-11-13T18:00:00Z"}
                            ]
                        },
                        {
                            "name": "Claude and GPT models",
                            "buckets": [
                                {"window": "weekly", "remaining_fraction": 0.18, "reset_time": 1770000000},
                                {"window": "5h", "remaining_fraction": 0.65, "reset_time": "2026-11-13T16:00:00Z"}
                            ]
                        }
                    ]
                }
            },
            "status": "SUCCESS"
        });
        let windows = parse_antigravity(&payload).unwrap();
        assert_eq!(windows.len(), 4);
        assert_eq!(windows[0].name, "Gemini Models · 5 hours used");
        assert!((windows[0].used_percent - 18.0).abs() < 1e-6);
        assert_eq!(
            windows[0].resets_at,
            DateTime::parse_from_rfc3339("2026-11-13T18:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc)
                .into()
        );
        assert_eq!(windows[1].name, "Gemini Models · Week used");
        assert!((windows[1].used_percent - 48.0).abs() < 1e-6);
        assert_eq!(windows[2].name, "Claude and GPT models · 5 hours used");
        assert!((windows[2].used_percent - 35.0).abs() < 1e-6);
        assert_eq!(windows[3].name, "Claude and GPT models · Week used");
        assert!((windows[3].used_percent - 82.0).abs() < 1e-6);
        assert_eq!(
            windows[3].resets_at,
            DateTime::from_timestamp(1770000000, 0)
        );
    }

    #[test]
    fn rejects_invalid_or_signed_out_usage_json() {
        assert_eq!(
            parse_antigravity(
                &serde_json::json!({"command": {"name": "other"}, "status": "SUCCESS"})
            ),
            Err(UnavailableReason::InvalidResponse)
        );
        assert_eq!(
            parse_antigravity(
                &serde_json::json!({"command": {"name": "usage"}, "status": "FAILED", "error": "Not authenticated; run agy login"})
            ),
            Err(UnavailableReason::SignIn)
        );
        assert_eq!(
            parse_antigravity(
                &serde_json::json!({"command": {"name": "usage", "data": {"groups": []}}, "status": "SUCCESS"})
            ),
            Err(UnavailableReason::InvalidResponse)
        );
        assert_eq!(
            parse_antigravity(&serde_json::json!({
                "command": {"name": "usage", "data": {"groups": [{"name": "Gemini Models", "buckets": [{"window": "weekly", "remaining_fraction": 1.5}]}]}},
                "status": "SUCCESS"
            })),
            Err(UnavailableReason::InvalidResponse)
        );
        assert_eq!(
            parse_antigravity_output(b"not json at all"),
            Err(UnavailableReason::InvalidResponse)
        );
    }

    #[test]
    fn extracts_json_after_leading_log_line() {
        let raw = b"Update available\n{\"command\":{\"name\":\"usage\",\"data\":{\"groups\":[{\"name\":\"Gemini Models\",\"buckets\":[{\"window\":\"weekly\",\"remaining_fraction\":0.25}]}]}},\"status\":\"SUCCESS\"}\n";
        let windows = parse_antigravity_output(raw).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].name, "Gemini Models · Week used");
        assert!((windows[0].used_percent - 75.0).abs() < 1e-6);
    }

    #[cfg(unix)]
    fn write_script(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn signed_out_gate_never_invokes_usage_probe() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations.log");
        let script = write_script(
            dir.path(),
            "agy",
            &format!(
                r#"echo "auth=$AGY_CLI_CDE_AUTH_ACTION auto=$AGY_CLI_DISABLE_AUTO_UPDATE args=$*" >> "{log}"
if [ "$1" = "--version" ]; then
  echo "1.5.8"
  exit 0
fi
if [ "$AGY_CLI_CDE_AUTH_ACTION" = "check" ] && [ "$#" -eq 0 ]; then
  exit 1
fi
exit 99
"#,
                log = log.display()
            ),
        );
        let env = LoginShellEnv::default();
        let result = fetch_antigravity_with_timeouts(
            script.to_str().unwrap(),
            &env,
            Duration::from_secs(10),
            Duration::from_secs(10),
        );
        assert_eq!(result, Err(UnavailableReason::SignIn));
        let recorded = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            recorded.lines().collect::<Vec<_>>(),
            [
                "auth= auto=true args=--version",
                "auth=check auto=true args=",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn preflight_timeout_kills_process_and_never_invokes_usage_probe() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations.log");
        let script = write_script(
            dir.path(),
            "agy",
            &format!(
                r#"if [ "$1" = "warmup" ]; then exit 0; fi
if [ "$1" = "--version" ]; then
  echo "1.5.8"
  exit 0
fi
echo "call args=$#" >> "{log}"
sleep 5
"#,
                log = log.display()
            ),
        );
        // Let macOS complete initial executable inspection before starting the tight timer.
        assert!(std::process::Command::new(&script)
            .arg("warmup")
            .status()
            .unwrap()
            .success());
        let env = LoginShellEnv::default();
        let result = fetch_antigravity_with_timeouts(
            script.to_str().unwrap(),
            &env,
            Duration::from_millis(300),
            Duration::from_secs(10),
        );
        assert_eq!(result, Err(UnavailableReason::AntigravityNoAnswer));
        let recorded = std::fs::read_to_string(&log).unwrap();
        assert_eq!(recorded.lines().collect::<Vec<_>>(), ["call args=0"]);
    }

    #[cfg(unix)]
    fn wait_until_process_exits(pid: i32, max_wait: Duration) -> bool {
        let deadline = Instant::now() + max_wait;
        loop {
            if !crate::session::process::process_exists(pid) {
                return true;
            }
            if Instant::now() >= deadline {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
                return false;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn signed_in_gate_detaches_tty_and_parses_pools() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations.log");
        let script = write_script(
            dir.path(),
            "agy",
            &format!(
                r#"if ( : < /dev/tty ) 2>/dev/null; then tty=yes; else tty=no; fi
if [ ! -e /dev/fd/0 ]; then stdin_char=missing; elif [ -c /dev/fd/0 ]; then stdin_char=yes; else stdin_char=no; fi
echo "auth=$AGY_CLI_CDE_AUTH_ACTION tty=$tty stdin_char=$stdin_char args=$*" >> "{log}"
if [ "$1" = "--version" ]; then
  echo "1.5.8"
  exit 0
fi
if [ "$AGY_CLI_CDE_AUTH_ACTION" = "check" ] && [ "$#" -eq 0 ]; then
  echo "Already authenticated as user@example.com."
  exit 0
fi
if [ "$1" = "-p" ] && [ "$2" = "/usage" ]; then
  printf '%s\n' '{{"command":{{"name":"usage","data":{{"groups":[{{"name":"Gemini Models","buckets":[{{"window":"weekly","remaining_fraction":0.52}},{{"window":"5h","remaining_fraction":0.82}}]}},{{"name":"Claude and GPT models","buckets":[{{"window":"weekly","remaining_fraction":0.18}},{{"window":"5h","remaining_fraction":0.65}}]}}]}}}},"status":"SUCCESS"}}'
  exit 0
fi
exit 2
"#,
                log = log.display()
            ),
        );
        let env = LoginShellEnv::default();
        let reading = fetch_antigravity_with_timeouts(
            script.to_str().unwrap(),
            &env,
            Duration::from_secs(10),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(reading.windows.len(), 4);
        assert_eq!(reading.windows[0].name, "Gemini Models · 5 hours used");
        assert!((reading.windows[3].used_percent - 82.0).abs() < 1e-6);
        let recorded = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            recorded.lines().collect::<Vec<_>>(),
            [
                "auth= tty=no stdin_char=no args=--version",
                "auth=check tty=no stdin_char=no args=",
                "auth= tty=no stdin_char=no args=-p /usage --output-format json",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn usage_timeout_maps_to_no_answer() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations.log");
        let script = write_script(
            dir.path(),
            "agy",
            &format!(
                r#"if [ "$1" = "warmup" ]; then exit 0; fi
if [ "$1" = "--version" ]; then
  echo "1.5.8"
  exit 0
fi
if [ "$AGY_CLI_CDE_AUTH_ACTION" = "check" ]; then
  echo "check" >> "{log}"
  exit 0
fi
echo "usage" >> "{log}"
sleep 5
"#,
                log = log.display()
            ),
        );
        // Let macOS complete initial executable inspection before the tight usage timer.
        assert!(std::process::Command::new(&script)
            .arg("warmup")
            .status()
            .unwrap()
            .success());
        let env = LoginShellEnv::default();
        let result = fetch_antigravity_with_timeouts(
            script.to_str().unwrap(),
            &env,
            Duration::from_secs(10),
            Duration::from_millis(200),
        );
        assert_eq!(result, Err(UnavailableReason::AntigravityNoAnswer));
        let recorded = std::fs::read_to_string(&log).unwrap();
        assert_eq!(recorded.lines().collect::<Vec<_>>(), ["check", "usage"]);
    }

    #[cfg(unix)]
    #[test]
    fn usage_auth_error_after_check_maps_to_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(
            dir.path(),
            "agy",
            r#"if [ "$1" = "--version" ]; then
  echo "1.5.8"
  exit 0
fi
if [ "$AGY_CLI_CDE_AUTH_ACTION" = "check" ]; then
  exit 0
fi
echo "Session expired, please run agy login" >&2
exit 1
"#,
        );
        let env = LoginShellEnv::default();
        let result = fetch_antigravity_with_timeouts(
            script.to_str().unwrap(),
            &env,
            Duration::from_secs(10),
            Duration::from_secs(10),
        );
        assert_eq!(result, Err(UnavailableReason::SignIn));
    }

    #[cfg(unix)]
    #[test]
    fn usage_timeout_bounds_pipe_holding_descendant_and_kills_it() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("descendant.pid");
        let script = write_script(
            dir.path(),
            "agy",
            &format!(
                r#"if [ "$1" = "warmup" ]; then exit 0; fi
if [ "$1" = "--version" ]; then
  echo "1.5.8"
  exit 0
fi
if [ "$AGY_CLI_CDE_AUTH_ACTION" = "check" ]; then
  exit 0
fi
sleep 600 &
echo "$!" > "{pid_file}"
exit 0
"#,
                pid_file = pid_file.display()
            ),
        );
        assert!(std::process::Command::new(&script)
            .arg("warmup")
            .status()
            .unwrap()
            .success());
        let env = LoginShellEnv::default();
        let started = Instant::now();
        let result = fetch_antigravity_with_timeouts(
            script.to_str().unwrap(),
            &env,
            Duration::from_secs(10),
            Duration::from_millis(200),
        );
        assert_eq!(result, Err(UnavailableReason::AntigravityNoAnswer));
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "pipe timeout exceeded bounded deadline: {:?}",
            started.elapsed()
        );
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(
            wait_until_process_exits(pid, Duration::from_secs(2)),
            "pipe-holding descendant {pid} was not killed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn normal_exit_cleans_up_detached_descendant() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("descendant.pid");
        let script = write_script(
            dir.path(),
            "agy",
            &format!(
                r#"if [ "$1" = "warmup" ]; then exit 0; fi
if [ "$1" = "--version" ]; then
  echo "1.5.8"
  exit 0
fi
if [ "$AGY_CLI_CDE_AUTH_ACTION" = "check" ]; then
  exit 0
fi
sleep 600 </dev/null >/dev/null 2>&1 &
echo "$!" > "{pid_file}"
printf '%s\n' '{{"command":{{"name":"usage","data":{{"groups":[{{"name":"Gemini Models","buckets":[{{"window":"weekly","remaining_fraction":0.52}}]}},{{"name":"Claude and GPT models","buckets":[{{"window":"weekly","remaining_fraction":0.18}}]}}]}}}},"status":"SUCCESS"}}'
exit 0
"#,
                pid_file = pid_file.display()
            ),
        );
        assert!(std::process::Command::new(&script)
            .arg("warmup")
            .status()
            .unwrap()
            .success());
        let env = LoginShellEnv::default();
        let reading = fetch_antigravity_with_timeouts(
            script.to_str().unwrap(),
            &env,
            Duration::from_secs(10),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(reading.windows.len(), 2);
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(
            wait_until_process_exits(pid, Duration::from_secs(2)),
            "background descendant {pid} survived normal command exit"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unsupported_version_never_invokes_auth_check_or_usage_probe() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations.log");
        let script = write_script(
            dir.path(),
            "agy",
            &format!(
                r#"if [ "$1" = "warmup" ]; then exit 0; fi
echo "auth=$AGY_CLI_CDE_AUTH_ACTION args=$*" >> "{log}"
if [ "$1" = "--version" ]; then
  echo "$TEST_VERSION"
  exit 0
fi
exit 99
"#,
                log = log.display()
            ),
        );
        assert!(std::process::Command::new(&script)
            .arg("warmup")
            .status()
            .unwrap()
            .success());
        for version in ["1.5.7", "1.2.9", "1.5.8-rc.1"] {
            let _ = std::fs::remove_file(&log);
            let mut env = LoginShellEnv::default();
            env.vars.insert("TEST_VERSION".into(), version.into());
            let result = fetch_antigravity_with_timeouts(
                script.to_str().unwrap(),
                &env,
                Duration::from_secs(10),
                Duration::from_secs(10),
            );
            assert_eq!(result, Err(UnavailableReason::AntigravityNoAnswer));
            let recorded = std::fs::read_to_string(&log).unwrap();
            assert_eq!(
                recorded.lines().collect::<Vec<_>>(),
                ["auth= args=--version"],
                "version {version} must not reach auth check or usage"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn unknown_or_failing_version_never_invokes_auth_check_or_usage_probe() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations.log");
        let script = write_script(
            dir.path(),
            "agy",
            &format!(
                r#"if [ "$1" = "warmup" ]; then exit 0; fi
echo "auth=$AGY_CLI_CDE_AUTH_ACTION args=$*" >> "{log}"
if [ -n "$TEST_SLEEP" ]; then
  sleep 5
fi
if [ "$1" = "--version" ]; then
  if [ -n "$TEST_STDOUT" ]; then echo "$TEST_STDOUT"; fi
  exit "${{TEST_EXIT:-0}}"
fi
exit 99
"#,
                log = log.display()
            ),
        );
        assert!(std::process::Command::new(&script)
            .arg("warmup")
            .status()
            .unwrap()
            .success());

        for (stdout, code) in [("unknown", "0"), ("", "0"), ("1.5.8", "1")] {
            let _ = std::fs::remove_file(&log);
            let mut env = LoginShellEnv::default();
            env.vars.insert("TEST_STDOUT".into(), stdout.into());
            env.vars.insert("TEST_EXIT".into(), code.into());
            let result = fetch_antigravity_with_timeouts(
                script.to_str().unwrap(),
                &env,
                Duration::from_secs(10),
                Duration::from_secs(10),
            );
            assert_eq!(result, Err(UnavailableReason::AntigravityNoAnswer));
            let recorded = std::fs::read_to_string(&log).unwrap();
            assert_eq!(
                recorded.lines().collect::<Vec<_>>(),
                ["auth= args=--version"]
            );
        }

        let _ = std::fs::remove_file(&log);
        let mut env = LoginShellEnv::default();
        env.vars.insert("TEST_SLEEP".into(), "1".into());
        let result = fetch_antigravity_with_timeouts(
            script.to_str().unwrap(),
            &env,
            Duration::from_millis(200),
            Duration::from_secs(10),
        );
        assert_eq!(result, Err(UnavailableReason::AntigravityNoAnswer));
        let recorded = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            recorded.lines().collect::<Vec<_>>(),
            ["auth= args=--version"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_probe_detaches_console_and_pipes_non_char_stdin() {
        if std::env::var_os("RUNNER_ANTIGRAVITY_CONSOLE_PROBE").is_some() {
            return;
        }
        let env = LoginShellEnv {
            vars: std::collections::BTreeMap::from([(
                "RUNNER_ANTIGRAVITY_CONSOLE_PROBE".into(),
                "1".into(),
            )]),
            ..LoginShellEnv::default()
        };
        let exe = std::env::current_exe().unwrap();
        let mut command = build_base_command(exe.to_str().unwrap(), &env);
        command.args([
            "--exact",
            "runtimes::antigravity::usage::tests::windows_console_probe_helper",
            "--nocapture",
        ]);
        prepare_isolated_fork(&mut command);

        let output = run_command_with_timeout(command, Duration::from_secs(10)).unwrap();
        assert!(
            output.status.success(),
            "helper failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("conin_denied=true"), "stdout={stdout}");
        assert!(stdout.contains("console_window=null"), "stdout={stdout}");
        assert!(stdout.contains("stdin_file_type=pipe"), "stdout={stdout}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_console_probe_helper() {
        if std::env::var_os("RUNNER_ANTIGRAVITY_CONSOLE_PROBE").is_none() {
            return;
        }
        use std::io::Read;
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileType, FILE_TYPE_CHAR, FILE_TYPE_PIPE,
        };
        use windows_sys::Win32::System::Console::{
            GetConsoleWindow, GetStdHandle, STD_INPUT_HANDLE,
        };

        assert!(
            std::fs::OpenOptions::new()
                .read(true)
                .open("CONIN$")
                .is_err(),
            "CONIN$ must not open under DETACHED_PROCESS"
        );
        let console_window = unsafe { GetConsoleWindow() };
        assert!(
            console_window.is_null(),
            "GetConsoleWindow must be null under DETACHED_PROCESS"
        );
        let stdin_handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        assert_ne!(stdin_handle, INVALID_HANDLE_VALUE);
        assert!(!stdin_handle.is_null());
        let file_type = unsafe { GetFileType(stdin_handle) };
        assert_ne!(file_type, FILE_TYPE_CHAR);
        assert_eq!(file_type, FILE_TYPE_PIPE);

        let mut buf = [0u8; 1];
        assert_eq!(std::io::stdin().read(&mut buf).unwrap(), 0);
        println!("conin_denied=true console_window=null stdin_file_type=pipe");
    }
}
