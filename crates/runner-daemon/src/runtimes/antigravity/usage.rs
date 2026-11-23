//! Usage fetching for Antigravity CLI (`agy`).
//!
//! # Unix (`#[cfg(not(windows))]`)
//!
//! Running `agy -p /usage` without confirming authentication first can make a
//! signed-out CLI open browser OAuth (`printmode.interactiveInput` passes if
//! stdin satisfies Go's `fs.ModeCharDevice`, which includes `/dev/null` and
//! `NUL`, or falls back to controlling terminal discovery on `/dev/tty`), and
//! pre-`1.5.8` builds have not been verified to honor the bare auth-check gate.
//! To uphold the zero-login guarantee without altering user state:
//! 1. Version gate runs `agy --version` with a closed-pipe `stdin`
//!    (`!ModeCharDevice`) and session isolation (`setsid()` on Unix). Missing or
//!    unparseable output, non-zero exit, timeout, or version `< 1.5.8` fails
//!    closed (`AntigravityNoAnswer`) without invoking bare `agy`.
//! 2. Preflight runs bare `agy` with `AGY_CLI_CDE_AUTH_ACTION=check`, a
//!    closed-pipe `stdin`, and `setsid()`. This checks OS credentials and
//!    `~/.gemini` state without starting printmode or browser OAuth (`exit 0`
//!    signed in, `exit 1` signed out on macOS `1.5.8`).
//! 3. Only when version >= `1.5.8` and preflight exits `0` does Runner spawn
//!    `agy -p /usage --output-format json` with closed-pipe `stdin` and
//!    `setsid()`.
//!
//! # Windows (`#[cfg(windows)]`)
//!
//! On Windows, spawning `agy -p /usage` under `DETACHED_PROCESS` leaves the
//! process without a console, so any console-subsystem MCP server or shim that
//! `agy` starts triggers Windows Terminal / OpenConsole to allocate a visible
//! window, while giving `agy` a hidden console (`CREATE_NO_WINDOW`) weakens
//! console isolation. Instead, Windows skips child processes completely and
//! queries quota directly over HTTPS:
//! 1. Read `agy`'s saved sign-in strictly read-only from Windows Credential
//!    Manager (`CredReadW` on generic target `"gemini:antigravity"`), falling
//!    back to `agy`'s disk token files under `.gemini/antigravity-cli` and
//!    `.gemini` (or `ANTIGRAVITY_GEMINI_DIR`) when the keyring entry is absent
//!    or a `cache/antigravity-keyring-unavailable` marker is fresher than 1 hour.
//! 2. Reuse the stored `access_token` while unexpired; otherwise refresh it in
//!    memory against `oauth2.googleapis.com/token` using `agy`'s installed-app
//!    OAuth client (`gcp` vs `consumer`) with `User-Agent: Runner/<version>`.
//!    Refreshed tokens are kept in memory only and are never written back or
//!    logged.
//! 3. POST `v1internal:loadCodeAssist` (to resolve `cloudaicompanionProject`)
//!    and `v1internal:retrieveUserQuotaSummary` on Cloud Code with
//!    `User-Agent: Runner/<version> (antigravity/cli/1.5.9)` (Cloud Code omits
//!    `cloudaicompanionProject` and rejects quota queries with `403` without the
//!    `antigravity/cli/...` product token, and omits `Gemini Models` without
//!    `/cli/`), mapping groups and buckets into `UsageReading` windows.
//! 4. HTTP clients disable redirects (`Policy::none()`) so bearer or refresh
//!    secrets are never forwarded, and preserve inherited proxy environment
//!    variables (`HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`/`NO_PROXY`) as well as
//!    Windows system proxy settings.
//! 5. Missing credentials and OAuth/auth rejections (`4xx != 429` on refresh,
//!    `401`/`403` after optional in-memory retry) return `UnavailableReason::SignIn`;
//!    network failures, timeouts, `429`, `5xx`, redirects, and unparseable
//!    responses return `UnavailableReason::AntigravityNoAnswer`.

use crate::shell_path::LoginShellEnv;
use crate::usage::{parse_timestamp, UnavailableReason, UsageReading, UsageWindow};
use serde_json::Value;
use std::time::Duration;

const AUTH_CHECK_TIMEOUT: Duration = Duration::from_secs(4);
const USAGE_TIMEOUT: Duration = Duration::from_secs(15);

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
    #[cfg(not(windows))]
    {
        cli_probe::fetch(command, env, auth_timeout, usage_timeout)
    }
    #[cfg(windows)]
    {
        let _ = command;
        direct_quota::fetch_windows(env, auth_timeout, usage_timeout)
    }
}

pub(crate) fn has_stored_credential(env: &LoginShellEnv) -> bool {
    #[cfg(windows)]
    {
        direct_quota::load_stored_credential(env, direct_quota::read_windows_default_credential)
            .is_ok()
    }
    #[cfg(not(windows))]
    {
        token_paths::has_token_file(env)
    }
}

mod token_paths {
    use super::LoginShellEnv;
    use serde_json::Value;
    use std::path::PathBuf;

    pub(super) const KEYRING_UNAVAILABLE_MARKER: &str = "antigravity-keyring-unavailable";

    pub(super) fn env_non_empty<'a>(env: &'a LoginShellEnv, name: &str) -> Option<&'a str> {
        env.vars
            .get(name)
            .map(String::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    pub(super) fn token_storage_paths(env: &LoginShellEnv) -> (Vec<PathBuf>, Vec<PathBuf>) {
        #[cfg(windows)]
        let overridden_home =
            env_non_empty(env, "USERPROFILE").or_else(|| env_non_empty(env, "HOME"));
        #[cfg(not(windows))]
        let overridden_home =
            env_non_empty(env, "HOME").or_else(|| env_non_empty(env, "USERPROFILE"));

        let process_cli_dir = if overridden_home.is_none() {
            std::env::var("ANTIGRAVITY_GEMINI_DIR")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        } else {
            None
        };
        let cli_override = env_non_empty(env, "ANTIGRAVITY_GEMINI_DIR")
            .map(PathBuf::from)
            .or_else(|| process_cli_dir.map(PathBuf::from));

        let home = overridden_home
            .map(PathBuf::from)
            .or_else(runner_core::app_paths::home_dir);

        let gemini_dir = match (cli_override, home) {
            (Some(override_dir), _) => override_dir,
            (None, Some(home)) => home.join(".gemini"),
            (None, None) => return (Vec::new(), Vec::new()),
        };
        let cli_dir = gemini_dir.join("antigravity-cli");

        let markers = vec![
            cli_dir.join("cache").join(KEYRING_UNAVAILABLE_MARKER),
            gemini_dir.join("cache").join(KEYRING_UNAVAILABLE_MARKER),
        ];
        let token_files = vec![
            cli_dir.join("antigravity-oauth-token"),
            gemini_dir.join("antigravity-oauth-token"),
            gemini_dir.join("jetski-standalone-oauth-token"),
        ];
        (markers, token_files)
    }

    pub(super) fn extract_tokens(raw: &str) -> Option<(String, String, Value)> {
        let value: Value = serde_json::from_str(raw.trim()).ok()?;
        let obj = value.as_object()?;
        let token_obj = obj.get("token").and_then(Value::as_object).unwrap_or(obj);

        let access_token = token_obj
            .get("access_token")
            .or_else(|| token_obj.get("accessToken"))
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_owned();
        let refresh_token = token_obj
            .get("refresh_token")
            .or_else(|| token_obj.get("refreshToken"))
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_owned();

        if access_token.is_empty() && refresh_token.is_empty() {
            return None;
        }
        Some((access_token, refresh_token, value))
    }

    #[cfg(not(windows))]
    pub(super) fn has_token_file(env: &LoginShellEnv) -> bool {
        let (_markers, token_files) = token_storage_paths(env);
        token_files.into_iter().any(|path| {
            std::fs::read_to_string(path)
                .ok()
                .and_then(|raw| extract_tokens(&raw))
                .is_some()
        })
    }
}

#[cfg(not(windows))]
mod cli_probe {
    use super::*;
    use crate::runtime_status::direct_chat_path;
    use crate::runtime_status::versions::{below_minimum, parse_version};
    use crate::session::process::{kill_headless_fork, ProcessTree};
    use std::io::Read;
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    const MIN_USAGE_VERSION: &str = "1.5.8";
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

    pub(super) fn fetch(
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

    #[cfg(not(unix))]
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
        #[cfg(not(unix))]
        {
            let _ = tree;
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(any(not(windows), test))]
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

#[cfg(any(not(windows), test))]
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

#[cfg(any(not(windows), test))]
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
        collect_group_windows(
            group_name,
            buckets,
            &["remaining_fraction"],
            &["reset_time"],
            false,
            UnavailableReason::InvalidResponse,
            &mut windows,
        )?;
    }

    if windows.is_empty() {
        return Err(UnavailableReason::InvalidResponse);
    }
    Ok(windows)
}

fn collect_group_windows(
    group_name: &str,
    buckets: &[Value],
    fraction_keys: &[&str],
    reset_keys: &[&str],
    skip_disabled: bool,
    invalid_reason: UnavailableReason,
    windows: &mut Vec<UsageWindow>,
) -> Result<(), UnavailableReason> {
    // Emit `5 hours` above `Week` inside each pool regardless of wire order.
    let mut five_hour = None;
    let mut weekly = None;
    for bucket in buckets {
        if skip_disabled && bucket.get("disabled").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(window_type) = bucket.get("window").and_then(Value::as_str) else {
            continue;
        };
        let Some(remaining) = fraction_keys
            .iter()
            .find_map(|key| bucket.get(*key))
            .and_then(Value::as_f64)
        else {
            continue;
        };
        if !remaining.is_finite() || !(0.0..=1.0).contains(&remaining) {
            return Err(invalid_reason);
        }
        let used_percent = ((1.0 - remaining) * 100.0).clamp(0.0, 100.0);
        let resets_at = parse_timestamp(reset_keys.iter().find_map(|key| bucket.get(*key)));
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
    Ok(())
}

#[cfg(any(windows, test))]
mod direct_quota {
    use super::{
        collect_group_windows, LoginShellEnv, UnavailableReason, UsageReading, UsageWindow,
    };
    use base64::Engine as _;
    use chrono::{DateTime, Duration as ChronoDuration, Utc};
    use reqwest::{Proxy, StatusCode};
    use serde::Deserialize;
    use serde_json::{json, Value};
    use std::path::Path;
    use std::time::{Duration, Instant, SystemTime};

    #[cfg(windows)]
    const WINDOWS_CREDENTIAL_TARGET: &str = "gemini:antigravity";
    const KEYRING_UNAVAILABLE_TTL: Duration = Duration::from_secs(3600);
    const RUNNER_USER_AGENT: &str = concat!("Runner/", env!("CARGO_PKG_VERSION"));
    // Measured against Cloud Code on 2026-11-15: plain `Runner/...` omits
    // `cloudaicompanionProject` and returns `403 PERMISSION_DENIED` on
    // `retrieveUserQuotaSummary`, while `(antigravity/<ver>)` without `/cli/`
    // returns only Claude/GPT and omits `Gemini Models`.
    const CLOUD_CODE_USER_AGENT: &str = concat!(
        "Runner/",
        env!("CARGO_PKG_VERSION"),
        " (antigravity/cli/1.5.9)"
    );
    const OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
    // Antigravity's installed-app OAuth clients, shipped in every `agy` binary.
    // Per Google's installed-app OAuth guidance, such a client secret is not
    // treated as a secret; user credentials stay in the local credential store.
    const CONSUMER_CLIENT_ID: &str =
        "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
    const CONSUMER_CLIENT_SECRET: &str = "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";
    const CONSUMER_CLOUD_CODE_URL: &str = "https://daily-cloudcode-pa.googleapis.com";
    const GCP_CLIENT_ID: &str =
        "884354919052-36trc1jjb3tguiac32ov6cod268c5blh.apps.googleusercontent.com";
    const GCP_CLIENT_SECRET: &str = "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";
    const GCP_CLOUD_CODE_URL: &str = "https://cloudcode-pa.googleapis.com";
    const EXPIRY_LEEWAY_SECS: i64 = 30;
    const GO_KEYRING_BASE64_PREFIX: &str = "go-keyring-base64:";

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) struct StoredCredential {
        pub access_token: String,
        pub refresh_token: String,
        pub expiry: Option<DateTime<Utc>>,
        pub auth_method: Option<String>,
        pub project_id: Option<String>,
    }

    #[derive(Clone, Copy, Debug)]
    pub(super) struct EndpointConfig<'a> {
        pub oauth_token_url: &'a str,
        pub cloud_code_url: &'a str,
    }

    #[derive(Clone, Copy, Debug)]
    pub(super) struct OAuthClientCredentials {
        pub(super) client_id: &'static str,
        pub(super) client_secret: &'static str,
        pub(super) default_cloud_code_url: &'static str,
    }

    #[derive(Deserialize)]
    struct OAuthRefreshResponse {
        #[serde(default)]
        access_token: String,
        #[serde(default)]
        error: Option<String>,
    }

    #[cfg(windows)]
    pub(super) fn fetch_windows(
        env: &LoginShellEnv,
        auth_timeout: Duration,
        usage_timeout: Duration,
    ) -> Result<UsageReading, UnavailableReason> {
        fetch_with_options_with_proxy(
            env,
            auth_timeout,
            usage_timeout,
            None,
            &|name| std::env::var(name).ok(),
            true,
            read_windows_default_credential,
        )
    }

    #[cfg(test)]
    pub(super) fn fetch_with_options(
        env: &LoginShellEnv,
        auth_timeout: Duration,
        usage_timeout: Duration,
        override_endpoints: Option<EndpointConfig<'_>>,
        read_keyring: impl FnOnce() -> Result<Option<String>, ()>,
    ) -> Result<UsageReading, UnavailableReason> {
        fetch_with_options_with_proxy(
            env,
            auth_timeout,
            usage_timeout,
            override_endpoints,
            &|_| None,
            false,
            read_keyring,
        )
    }

    pub(super) fn fetch_with_options_with_proxy(
        env: &LoginShellEnv,
        auth_timeout: Duration,
        usage_timeout: Duration,
        override_endpoints: Option<EndpointConfig<'_>>,
        process_var: &dyn Fn(&str) -> Option<String>,
        allow_system_proxy: bool,
        read_keyring: impl FnOnce() -> Result<Option<String>, ()>,
    ) -> Result<UsageReading, UnavailableReason> {
        let mut cred = load_stored_credential(env, read_keyring)?;
        let oauth = oauth_client_for_method(cred.auth_method.as_deref());
        let (oauth_token_url, cloud_code_url) = match override_endpoints {
            Some(ov) => (ov.oauth_token_url, ov.cloud_code_url.trim_end_matches('/')),
            None => (OAUTH_TOKEN_URL, oauth.default_cloud_code_url),
        };
        let client = build_http_client_with(env, process_var, allow_system_proxy)
            .map_err(|_| UnavailableReason::AntigravityNoAnswer)?;

        let mut refreshed = false;
        if is_expired_or_missing_access_token(&cred, Utc::now()) {
            cred.access_token = refresh_access_token(
                &client,
                oauth_token_url,
                &cred.refresh_token,
                oauth,
                auth_timeout,
            )?;
            refreshed = true;
        }

        let deadline = Instant::now() + usage_timeout;
        let load_url = format!("{cloud_code_url}/v1internal:loadCodeAssist");
        let load_body = json!({
            "metadata": {
                "ideType": "ANTIGRAVITY",
            }
        });
        let load_json = post_cloud_code_endpoint(
            &client,
            &load_url,
            &load_body,
            &mut cred,
            &mut refreshed,
            oauth_token_url,
            oauth,
            auth_timeout,
            deadline,
        )?;
        let project_id = extract_project_id(&load_json, cred.project_id.as_deref());

        let quota_url = format!("{cloud_code_url}/v1internal:retrieveUserQuotaSummary");
        let quota_body = match project_id {
            Some(project) => json!({ "project": project }),
            None => json!({}),
        };
        let quota_json = post_cloud_code_endpoint(
            &client,
            &quota_url,
            &quota_body,
            &mut cred,
            &mut refreshed,
            oauth_token_url,
            oauth,
            auth_timeout,
            deadline,
        )?;

        map_quota_summary_response(&quota_json).map(Into::into)
    }

    pub(super) fn oauth_client_for_method(auth_method: Option<&str>) -> OAuthClientCredentials {
        match auth_method.map(str::trim) {
            Some(method) if method.eq_ignore_ascii_case("gcp") => OAuthClientCredentials {
                client_id: GCP_CLIENT_ID,
                client_secret: GCP_CLIENT_SECRET,
                default_cloud_code_url: GCP_CLOUD_CODE_URL,
            },
            _ => OAuthClientCredentials {
                client_id: CONSUMER_CLIENT_ID,
                client_secret: CONSUMER_CLIENT_SECRET,
                default_cloud_code_url: CONSUMER_CLOUD_CODE_URL,
            },
        }
    }

    use super::token_paths::{extract_tokens, token_storage_paths};

    fn effective_env_value(
        env: &LoginShellEnv,
        process_var: &dyn Fn(&str) -> Option<String>,
        names: &[&str],
    ) -> Option<String> {
        for name in names {
            if let Some(raw) = env.vars.get(*name) {
                let trimmed = raw.trim();
                return (!trimmed.is_empty()).then(|| trimmed.to_owned());
            }
        }
        for name in names {
            if let Some(raw) = process_var(name) {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_owned());
                }
            }
        }
        None
    }

    fn build_http_client_with(
        env: &LoginShellEnv,
        process_var: &dyn Fn(&str) -> Option<String>,
        allow_system_proxy: bool,
    ) -> Result<reqwest::blocking::Client, reqwest::Error> {
        let mut builder = reqwest::blocking::Client::builder()
            .user_agent(RUNNER_USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(crate::usage::FETCH_TIMEOUT);

        let all_proxy = effective_env_value(env, process_var, &["ALL_PROXY", "all_proxy"]);
        let http_url = effective_env_value(env, process_var, &["HTTP_PROXY", "http_proxy"])
            .or_else(|| all_proxy.clone());
        let https_url =
            effective_env_value(env, process_var, &["HTTPS_PROXY", "https_proxy"]).or(all_proxy);

        if http_url.is_some() || https_url.is_some() {
            builder = builder.no_proxy();
            let no_proxy = effective_env_value(env, process_var, &["NO_PROXY", "no_proxy"])
                .and_then(|value| reqwest::NoProxy::from_string(&value));
            if let Some(url) = http_url {
                builder = builder.proxy(Proxy::http(url)?.no_proxy(no_proxy.clone()));
            }
            if let Some(url) = https_url {
                builder = builder.proxy(Proxy::https(url)?.no_proxy(no_proxy));
            }
        } else if !allow_system_proxy {
            builder = builder.no_proxy();
        }

        builder.build()
    }

    fn marker_is_recent(path: &Path) -> bool {
        let Ok(metadata) = std::fs::metadata(path) else {
            return false;
        };
        let Ok(modified) = metadata.modified() else {
            return true;
        };
        match SystemTime::now().duration_since(modified) {
            Ok(age) => age < KEYRING_UNAVAILABLE_TTL,
            Err(_) => true,
        }
    }

    pub(super) fn load_stored_credential(
        env: &LoginShellEnv,
        read_keyring: impl FnOnce() -> Result<Option<String>, ()>,
    ) -> Result<StoredCredential, UnavailableReason> {
        let (markers, token_files) = token_storage_paths(env);
        let keyring_disabled = markers.iter().any(|path| marker_is_recent(path));

        if !keyring_disabled {
            if let Ok(Some(raw)) = read_keyring() {
                if let Some(cred) = parse_stored_credential(&raw) {
                    return Ok(cred);
                }
            }
        }

        for path in &token_files {
            if let Some(cred) = read_token_file(path) {
                return Ok(cred);
            }
        }

        Err(UnavailableReason::SignIn)
    }

    fn read_token_file(path: &Path) -> Option<StoredCredential> {
        let contents = std::fs::read_to_string(path).ok()?;
        parse_stored_credential(&contents)
    }

    pub(super) fn decode_keyring_blob(raw: &[u8]) -> Option<String> {
        if raw.is_empty() {
            return None;
        }
        let text = if let Some(rest) = raw.strip_prefix(&[0xFF, 0xFE]) {
            decode_utf16_le(rest)?
        } else if raw.len().is_multiple_of(2)
            && raw.chunks_exact(2).any(|chunk| chunk[1] == 0)
            && !raw.starts_with(&[0xEF, 0xBB, 0xBF])
        {
            decode_utf16_le(raw)?
        } else {
            let utf8 = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw);
            std::str::from_utf8(utf8).ok()?.to_owned()
        };

        let trimmed = text.trim_matches('\0').trim();
        let decoded = if let Some(b64) = trimmed.strip_prefix(GO_KEYRING_BASE64_PREFIX) {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(b64.trim()))
                .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(b64.trim()))
                .ok()?;
            String::from_utf8(bytes).ok()?
        } else {
            trimmed.to_owned()
        };

        let cleaned = decoded.trim_matches('\0').trim().to_owned();
        (!cleaned.is_empty()).then_some(cleaned)
    }

    fn decode_utf16_le(bytes: &[u8]) -> Option<String> {
        if !bytes.len().is_multiple_of(2) {
            return None;
        }
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16(&units).ok()
    }

    pub(super) fn parse_stored_credential(raw: &str) -> Option<StoredCredential> {
        let (access_token, refresh_token, value) = extract_tokens(raw)?;
        let obj = value.as_object()?;
        let token_obj = obj.get("token").and_then(Value::as_object).unwrap_or(obj);

        let expiry = parse_expiry(token_obj.get("expiry").and_then(Value::as_str));
        let auth_method = obj
            .get("auth_method")
            .or_else(|| obj.get("authMethod"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let project_id = obj
            .get("project_id")
            .or_else(|| obj.get("projectId"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        Some(StoredCredential {
            access_token,
            refresh_token,
            expiry,
            auth_method,
            project_id,
        })
    }

    fn parse_expiry(raw: Option<&str>) -> Option<DateTime<Utc>> {
        let trimmed = raw.map(str::trim).filter(|s| !s.is_empty())?;
        let parsed = DateTime::parse_from_rfc3339(trimmed)
            .ok()?
            .with_timezone(&Utc);
        // Go encodes unset `time.Time{}` as `0001-01-01T00:00:00Z`.
        (parsed > DateTime::UNIX_EPOCH).then_some(parsed)
    }

    pub(super) fn is_expired_or_missing_access_token(
        cred: &StoredCredential,
        now: DateTime<Utc>,
    ) -> bool {
        if cred.access_token.trim().is_empty() {
            return true;
        }
        match cred.expiry {
            Some(expiry) => expiry <= now + ChronoDuration::seconds(EXPIRY_LEEWAY_SECS),
            None => false,
        }
    }

    fn refresh_access_token(
        client: &reqwest::blocking::Client,
        oauth_token_url: &str,
        refresh_token: &str,
        oauth: OAuthClientCredentials,
        timeout: Duration,
    ) -> Result<String, UnavailableReason> {
        if refresh_token.trim().is_empty() {
            return Err(UnavailableReason::SignIn);
        }
        if timeout.is_zero() {
            return Err(UnavailableReason::AntigravityNoAnswer);
        }

        let response = client
            .post(oauth_token_url)
            .timeout(timeout)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", oauth.client_id),
                ("client_secret", oauth.client_secret),
            ])
            .send()
            .map_err(|_| UnavailableReason::AntigravityNoAnswer)?;

        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            return Err(UnavailableReason::AntigravityNoAnswer);
        }
        if status.is_client_error() {
            return Err(UnavailableReason::SignIn);
        }
        if !status.is_success() {
            return Err(UnavailableReason::AntigravityNoAnswer);
        }

        let parsed: OAuthRefreshResponse = response
            .json()
            .map_err(|_| UnavailableReason::AntigravityNoAnswer)?;
        let access_token = parsed.access_token.trim();
        if !access_token.is_empty() {
            return Ok(access_token.to_owned());
        }
        if matches!(
            parsed.error.as_deref().map(str::trim),
            Some("invalid_grant" | "invalid_client" | "unauthorized_client")
        ) {
            return Err(UnavailableReason::SignIn);
        }
        Err(UnavailableReason::AntigravityNoAnswer)
    }

    #[allow(clippy::too_many_arguments)]
    fn post_cloud_code_endpoint(
        client: &reqwest::blocking::Client,
        url: &str,
        body: &Value,
        cred: &mut StoredCredential,
        refreshed: &mut bool,
        oauth_token_url: &str,
        oauth: OAuthClientCredentials,
        auth_timeout: Duration,
        deadline: Instant,
    ) -> Result<Value, UnavailableReason> {
        loop {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(UnavailableReason::AntigravityNoAnswer);
            };
            if remaining.is_zero() {
                return Err(UnavailableReason::AntigravityNoAnswer);
            }

            let response = client
                .post(url)
                .timeout(remaining)
                .header(reqwest::header::USER_AGENT, CLOUD_CODE_USER_AGENT)
                .bearer_auth(&cred.access_token)
                .json(body)
                .send()
                .map_err(|_| UnavailableReason::AntigravityNoAnswer)?;

            let status = response.status();
            if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                if !*refreshed && !cred.refresh_token.trim().is_empty() {
                    let refresh_budget = deadline
                        .saturating_duration_since(Instant::now())
                        .min(auth_timeout);
                    cred.access_token = refresh_access_token(
                        client,
                        oauth_token_url,
                        &cred.refresh_token,
                        oauth,
                        refresh_budget,
                    )?;
                    *refreshed = true;
                    continue;
                }
                return Err(UnavailableReason::SignIn);
            }

            if !status.is_success() {
                return Err(UnavailableReason::AntigravityNoAnswer);
            }

            return response
                .json::<Value>()
                .map_err(|_| UnavailableReason::AntigravityNoAnswer);
        }
    }

    fn extract_project_id(value: &Value, fallback: Option<&str>) -> Option<String> {
        let from_response = value
            .get("cloudaicompanionProject")
            .or_else(|| value.get("cloudaicompanion_project"))
            .and_then(|proj| {
                proj.as_str().or_else(|| {
                    proj.get("id")
                        .or_else(|| proj.get("projectId"))
                        .or_else(|| proj.get("project"))
                        .and_then(Value::as_str)
                })
            })
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        from_response.or_else(|| {
            fallback
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
    }

    pub(super) fn map_quota_summary_response(
        value: &Value,
    ) -> Result<Vec<UsageWindow>, UnavailableReason> {
        let groups = value
            .get("groups")
            .and_then(Value::as_array)
            .ok_or(UnavailableReason::AntigravityNoAnswer)?;

        let mut windows = Vec::new();
        for group in groups {
            let Some(group_name) = group
                .get("displayName")
                .or_else(|| group.get("display_name"))
                .or_else(|| group.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            let Some(buckets) = group.get("buckets").and_then(Value::as_array) else {
                continue;
            };
            collect_group_windows(
                group_name,
                buckets,
                &["remainingFraction", "remaining_fraction"],
                &["resetTime", "reset_time"],
                true,
                UnavailableReason::AntigravityNoAnswer,
                &mut windows,
            )?;
        }

        if windows.is_empty() {
            return Err(UnavailableReason::AntigravityNoAnswer);
        }
        Ok(windows)
    }

    #[cfg(windows)]
    pub(super) fn read_windows_default_credential() -> Result<Option<String>, ()> {
        read_windows_credential_target(WINDOWS_CREDENTIAL_TARGET)
    }

    #[cfg(windows)]
    pub(super) fn read_windows_credential_target(target: &str) -> Result<Option<String>, ()> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
        use windows_sys::Win32::Security::Credentials::{
            CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
        };

        struct CredGuard(*mut CREDENTIALW);
        impl Drop for CredGuard {
            fn drop(&mut self) {
                if !self.0.is_null() {
                    // SAFETY: pointer was allocated by CredReadW and is freed once.
                    unsafe { CredFree(self.0.cast()) };
                }
            }
        }

        let wide_target: Vec<u16> = std::ffi::OsStr::new(target)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut cred_ptr: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: wide_target is NUL-terminated and cred_ptr receives a CredFree-managed pointer.
        let ok = unsafe { CredReadW(wide_target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred_ptr) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            return if err == ERROR_NOT_FOUND {
                Ok(None)
            } else {
                Err(())
            };
        }
        let _guard = CredGuard(cred_ptr);
        if cred_ptr.is_null() {
            return Ok(None);
        }
        // SAFETY: CredReadW succeeded with a non-null CREDENTIALW whose blob spans CredentialBlobSize bytes.
        let cred = unsafe { &*cred_ptr };
        if cred.CredentialBlob.is_null() || cred.CredentialBlobSize == 0 {
            return Ok(None);
        }
        // SAFETY: CredentialBlob is valid for CredentialBlobSize bytes while CredGuard lives.
        let bytes = unsafe {
            std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize)
        };
        Ok(decode_keyring_blob(bytes))
    }
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
