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
//!    `.gemini` when the keyring entry is absent or marked unavailable.
//! 2. Reuse the stored `access_token` while unexpired; otherwise refresh it in
//!    memory against `oauth2.googleapis.com/token` using `agy`'s installed-app
//!    OAuth client (`gcp` vs `consumer`). Refreshed tokens are kept in memory
//!    only and are never written back or logged.
//! 3. POST `v1internal:loadCodeAssist` (to resolve `cloudaicompanionProject`)
//!    and `v1internal:retrieveUserQuotaSummary` on Cloud Code, mapping groups
//!    and buckets into `UsageReading` windows.
//! 4. Missing credentials and OAuth/auth rejections (`4xx != 429` on refresh,
//!    `401`/`403` after optional in-memory retry) return `UnavailableReason::SignIn`;
//!    network failures, timeouts, `429`, `5xx`, and unparseable responses
//!    return `UnavailableReason::AntigravityNoAnswer`.

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
        collect_group_windows(
            group_name,
            buckets,
            ("remaining_fraction", "remainingFraction"),
            ("reset_time", "resetTime"),
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
    fraction_keys: (&str, &str),
    reset_keys: (&str, &str),
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
        let Some(remaining) = bucket
            .get(fraction_keys.0)
            .or_else(|| bucket.get(fraction_keys.1))
            .and_then(Value::as_f64)
        else {
            continue;
        };
        if !remaining.is_finite() || !(0.0..=1.0).contains(&remaining) {
            return Err(invalid_reason);
        }
        let used_percent = ((1.0 - remaining) * 100.0).clamp(0.0, 100.0);
        let resets_at = parse_timestamp(
            bucket
                .get(reset_keys.0)
                .or_else(|| bucket.get(reset_keys.1)),
        );
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
    use crate::usage::http_client;
    use base64::Engine as _;
    use chrono::{DateTime, Duration as ChronoDuration, Utc};
    use reqwest::StatusCode;
    use serde::Deserialize;
    use serde_json::{json, Value};
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    #[cfg(windows)]
    const WINDOWS_CREDENTIAL_TARGET: &str = "gemini:antigravity";
    const KEYRING_UNAVAILABLE_MARKER: &str = "antigravity-keyring-unavailable";
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
        fetch_with_options(
            env,
            auth_timeout,
            usage_timeout,
            None,
            read_windows_default_credential,
        )
    }

    pub(super) fn fetch_with_options(
        env: &LoginShellEnv,
        auth_timeout: Duration,
        usage_timeout: Duration,
        override_endpoints: Option<EndpointConfig<'_>>,
        read_keyring: impl FnOnce() -> Result<Option<String>, ()>,
    ) -> Result<UsageReading, UnavailableReason> {
        let mut cred = load_stored_credential(env, read_keyring)?;
        let oauth = oauth_client_for_method(cred.auth_method.as_deref());
        let (oauth_token_url, cloud_code_url) = match override_endpoints {
            Some(ov) => (ov.oauth_token_url, ov.cloud_code_url.trim_end_matches('/')),
            None => (OAUTH_TOKEN_URL, oauth.default_cloud_code_url),
        };
        let client = http_client(env).map_err(|_| UnavailableReason::AntigravityNoAnswer)?;

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

    fn env_non_empty<'a>(env: &'a LoginShellEnv, name: &str) -> Option<&'a str> {
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

        let (cli_dir, gemini_dir) = match (home, cli_override) {
            (Some(home), cli) => {
                let gemini_dir = home.join(".gemini");
                let cli_dir = cli.unwrap_or_else(|| gemini_dir.join("antigravity-cli"));
                (cli_dir, gemini_dir)
            }
            (None, Some(cli_dir)) => {
                let gemini_dir = cli_dir.parent().unwrap_or(&cli_dir).to_path_buf();
                (cli_dir, gemini_dir)
            }
            (None, None) => return (Vec::new(), Vec::new()),
        };

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

    pub(super) fn load_stored_credential(
        env: &LoginShellEnv,
        read_keyring: impl FnOnce() -> Result<Option<String>, ()>,
    ) -> Result<StoredCredential, UnavailableReason> {
        let (markers, token_files) = token_storage_paths(env);
        let keyring_disabled = markers.iter().any(|path| path.exists());

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
                ("remainingFraction", "remaining_fraction"),
                ("resetTime", "reset_time"),
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
mod tests {
    use super::*;
    use chrono::DateTime;
    #[cfg(unix)]
    use std::thread;
    #[cfg(unix)]
    use std::time::Instant;

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
    fn windows_probe_spawns_no_child_process() {
        let dir = tempfile::tempdir().unwrap();
        let sentinel = dir.path().join("spawned.txt");
        let fake_agy = dir.path().join("fake-agy.cmd");
        std::fs::write(
            &fake_agy,
            format!("@echo spawned>\"{}\"\r\n", sentinel.display()),
        )
        .unwrap();

        let marker = dir
            .path()
            .join(".gemini")
            .join("antigravity-cli")
            .join("cache")
            .join("antigravity-keyring-unavailable");
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, "test-isolated").unwrap();

        let mut env = LoginShellEnv::default();
        env.vars.insert(
            "USERPROFILE".into(),
            dir.path().to_string_lossy().into_owned(),
        );
        env.vars
            .insert("HOME".into(), dir.path().to_string_lossy().into_owned());

        let result = fetch_antigravity_with_timeouts(
            fake_agy.to_str().unwrap(),
            &env,
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        assert_eq!(result, Err(UnavailableReason::SignIn));
        assert!(
            !sentinel.exists(),
            "Windows usage probe must never spawn agy or any child process"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_credential_manager_roundtrip() {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Security::Credentials::{
            CredDeleteW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
        };

        struct DeleteCredGuard(Vec<u16>);
        impl Drop for DeleteCredGuard {
            fn drop(&mut self) {
                unsafe {
                    CredDeleteW(self.0.as_ptr(), CRED_TYPE_GENERIC, 0);
                }
            }
        }

        // Always use a test-only target name; never touch `"gemini:antigravity"`.
        let target = format!("runner-test:antigravity-usage-{}", std::process::id());
        let mut wide_target: Vec<u16> = std::ffi::OsStr::new(&target)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let cleanup_guard = DeleteCredGuard(wide_target.clone());

        let secret_json = serde_json::json!({
            "token": {
                "access_token": "win-test-access",
                "refresh_token": "win-test-refresh",
                "expiry": "2099-01-01T00:00:00Z"
            },
            "auth_method": "consumer",
            "project_id": "win-proj"
        })
        .to_string();

        let write_blob = |bytes: &[u8]| {
            let mut blob = bytes.to_vec();
            let mut cred: CREDENTIALW = unsafe { std::mem::zeroed() };
            cred.Type = CRED_TYPE_GENERIC;
            cred.TargetName = wide_target.as_mut_ptr();
            cred.CredentialBlobSize = blob.len() as u32;
            cred.CredentialBlob = blob.as_mut_ptr();
            cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
            let wrote = unsafe { CredWriteW(&cred, 0) };
            assert_ne!(wrote, 0, "CredWriteW should store test credential");
        };

        // Raw UTF-8 JSON (go-keyring default on Windows)
        write_blob(secret_json.as_bytes());
        let read_back = direct_quota::read_windows_credential_target(&target)
            .expect("CredReadW should succeed")
            .expect("test credential should be present");
        let parsed = direct_quota::parse_stored_credential(&read_back)
            .expect("stored credential JSON should parse");
        assert_eq!(parsed.access_token, "win-test-access");
        assert_eq!(parsed.refresh_token, "win-test-refresh");
        assert_eq!(parsed.auth_method.as_deref(), Some("consumer"));
        assert_eq!(parsed.project_id.as_deref(), Some("win-proj"));

        // `go-keyring-base64:` encoded as UTF-16LE
        use base64::Engine as _;
        let b64_secret = format!(
            "go-keyring-base64:{}",
            base64::engine::general_purpose::STANDARD.encode(secret_json.as_bytes())
        );
        let utf16_blob: Vec<u8> = b64_secret
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        write_blob(&utf16_blob);
        let read_back_b64 = direct_quota::read_windows_credential_target(&target)
            .expect("CredReadW should succeed for UTF-16LE base64")
            .expect("UTF-16LE base64 credential should be present");
        let parsed_b64 = direct_quota::parse_stored_credential(&read_back_b64)
            .expect("UTF-16LE base64 JSON should parse");
        assert_eq!(parsed_b64.access_token, "win-test-access");

        drop(cleanup_guard);
        assert_eq!(
            direct_quota::read_windows_credential_target(&target),
            Ok(None)
        );
    }

    #[derive(Clone, Debug)]
    struct RecordedHttpRequest {
        path: String,
        headers: std::collections::BTreeMap<String, String>,
        body: String,
    }

    fn spawn_fake_http_server<F>(
        responses: usize,
        mut handler: F,
    ) -> (
        String,
        std::sync::mpsc::Receiver<RecordedHttpRequest>,
        std::thread::JoinHandle<()>,
    )
    where
        F: FnMut(usize, &RecordedHttpRequest) -> (u16, &'static str, String) + Send + 'static,
    {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            for idx in 0..responses {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let mut header_end = None;
                while header_end.is_none() {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        header_end = Some(pos + 4);
                    }
                }
                let Some(head_end) = header_end else {
                    continue;
                };
                let head_str = String::from_utf8_lossy(&buf[..head_end - 4]);
                let mut lines = head_str.lines();
                let request_line = lines.next().unwrap_or_default();
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                let mut headers = std::collections::BTreeMap::new();
                let mut content_length = 0usize;
                for line in lines {
                    if let Some((k, v)) = line.split_once(':') {
                        let key = k.trim().to_ascii_lowercase();
                        let val = v.trim().to_owned();
                        if key == "content-length" {
                            content_length = val.parse().unwrap_or(0);
                        }
                        headers.insert(key, val);
                    }
                }
                while buf.len() < head_end + content_length {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let body = String::from_utf8_lossy(
                    &buf[head_end..head_end + content_length.min(buf.len() - head_end)],
                )
                .into_owned();
                let req = RecordedHttpRequest {
                    path,
                    headers,
                    body,
                };
                let _ = tx.send(req.clone());
                let (status, reason, resp_body) = handler(idx, &req);
                let reply = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_body.len(),
                    resp_body
                );
                let _ = stream.write_all(reply.as_bytes());
                let _ = stream.flush();
            }
        });
        (format!("http://{addr}"), rx, handle)
    }

    #[test]
    fn direct_quota_decodes_keyring_blobs() {
        use base64::Engine as _;

        let json = r#"{"token":{"access_token":"abc","refresh_token":"def"}}"#;
        assert_eq!(
            direct_quota::decode_keyring_blob(json.as_bytes()).as_deref(),
            Some(json)
        );

        let mut with_bom = b"\xEF\xBB\xBF".to_vec();
        with_bom.extend_from_slice(json.as_bytes());
        assert_eq!(
            direct_quota::decode_keyring_blob(&with_bom).as_deref(),
            Some(json)
        );

        let utf16_le: Vec<u8> = json.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(
            direct_quota::decode_keyring_blob(&utf16_le).as_deref(),
            Some(json)
        );

        let mut utf16_bom = vec![0xFF, 0xFE];
        utf16_bom.extend_from_slice(&utf16_le);
        assert_eq!(
            direct_quota::decode_keyring_blob(&utf16_bom).as_deref(),
            Some(json)
        );

        let encoded = format!(
            "go-keyring-base64:{}",
            base64::engine::general_purpose::STANDARD.encode(json.as_bytes())
        );
        assert_eq!(
            direct_quota::decode_keyring_blob(encoded.as_bytes()).as_deref(),
            Some(json)
        );

        assert_eq!(direct_quota::decode_keyring_blob(b""), None);
        assert_eq!(direct_quota::decode_keyring_blob(b"   \0\0"), None);
        assert_eq!(
            direct_quota::decode_keyring_blob(b"go-keyring-base64:not-valid!"),
            None
        );
    }

    #[test]
    fn direct_quota_parses_nested_and_flat_token_formats() {
        let nested = r#"{
            "token": {
                "access_token": "tok-1",
                "token_type": "Bearer",
                "refresh_token": "ref-1",
                "expiry": "2099-01-01T00:00:00Z"
            },
            "auth_method": "gcp",
            "project_id": "gcp-proj"
        }"#;
        let parsed = direct_quota::parse_stored_credential(nested).unwrap();
        assert_eq!(parsed.access_token, "tok-1");
        assert_eq!(parsed.refresh_token, "ref-1");
        assert_eq!(parsed.auth_method.as_deref(), Some("gcp"));
        assert_eq!(parsed.project_id.as_deref(), Some("gcp-proj"));
        assert!(parsed.expiry.is_some());
        assert!(!direct_quota::is_expired_or_missing_access_token(
            &parsed,
            chrono::Utc::now()
        ));

        let flat_zero_expiry = r#"{
            "access_token": "tok-2",
            "refresh_token": "ref-2",
            "expiry": "0001-01-01T00:00:00Z"
        }"#;
        let parsed_flat = direct_quota::parse_stored_credential(flat_zero_expiry).unwrap();
        assert_eq!(parsed_flat.access_token, "tok-2");
        assert_eq!(parsed_flat.refresh_token, "ref-2");
        assert_eq!(parsed_flat.expiry, None);
        assert!(!direct_quota::is_expired_or_missing_access_token(
            &parsed_flat,
            chrono::Utc::now()
        ));

        let expired = direct_quota::parse_stored_credential(
            r#"{"access_token":"tok-3","refresh_token":"ref-3","expiry":"2020-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        assert!(direct_quota::is_expired_or_missing_access_token(
            &expired,
            chrono::Utc::now()
        ));

        assert_eq!(
            direct_quota::parse_stored_credential(
                r#"{"token":{"access_token":"","refresh_token":""}}"#
            ),
            None
        );
    }

    #[test]
    fn direct_quota_storage_precedence_and_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = LoginShellEnv::default();
        env.vars.insert(
            "USERPROFILE".into(),
            dir.path().to_string_lossy().into_owned(),
        );
        env.vars
            .insert("HOME".into(), dir.path().to_string_lossy().into_owned());

        let cli_dir = dir.path().join(".gemini").join("antigravity-cli");
        let gemini_dir = dir.path().join(".gemini");
        std::fs::create_dir_all(cli_dir.join("cache")).unwrap();

        let disk_cli = cli_dir.join("antigravity-oauth-token");
        std::fs::write(
            &disk_cli,
            r#"{"token":{"access_token":"disk-cli","refresh_token":"ref-disk"}}"#,
        )
        .unwrap();
        std::fs::write(
            gemini_dir.join("jetski-standalone-oauth-token"),
            r#"{"access_token":"disk-legacy","refresh_token":"ref-legacy"}"#,
        )
        .unwrap();

        // Keyring wins when available and no marker is present.
        let cred = direct_quota::load_stored_credential(&env, || {
            Ok(Some(
                r#"{"token":{"access_token":"keyring-tok","refresh_token":"ref-key"}}"#.into(),
            ))
        })
        .unwrap();
        assert_eq!(cred.access_token, "keyring-tok");

        // Fallback to cli disk file when keyring returns None.
        let cred = direct_quota::load_stored_credential(&env, || Ok(None)).unwrap();
        assert_eq!(cred.access_token, "disk-cli");

        // Marker skips keyring even if keyring would return a value.
        std::fs::write(
            cli_dir
                .join("cache")
                .join("antigravity-keyring-unavailable"),
            "1",
        )
        .unwrap();
        let cred = direct_quota::load_stored_credential(&env, || {
            Ok(Some(
                r#"{"token":{"access_token":"keyring-tok","refresh_token":"ref-key"}}"#.into(),
            ))
        })
        .unwrap();
        assert_eq!(cred.access_token, "disk-cli");

        // Removing primary disk file falls back to legacy standalone token file.
        std::fs::remove_file(&disk_cli).unwrap();
        let cred = direct_quota::load_stored_credential(&env, || Ok(None)).unwrap();
        assert_eq!(cred.access_token, "disk-legacy");

        // Removing all token files yields SignIn.
        std::fs::remove_file(gemini_dir.join("jetski-standalone-oauth-token")).unwrap();
        assert_eq!(
            direct_quota::load_stored_credential(&env, || Ok(None)),
            Err(UnavailableReason::SignIn)
        );
    }

    #[test]
    fn direct_quota_reuses_unexpired_token_and_maps_windows() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = LoginShellEnv::default();
        env.vars.insert(
            "USERPROFILE".into(),
            dir.path().to_string_lossy().into_owned(),
        );
        env.vars
            .insert("HOME".into(), dir.path().to_string_lossy().into_owned());

        let (base_url, rx, server) = spawn_fake_http_server(2, |_idx, req| {
            match req.path.as_str() {
            "/v1internal:loadCodeAssist" => (
                200,
                "OK",
                serde_json::json!({ "cloudaicompanionProject": "projects/live-proj" }).to_string(),
            ),
            "/v1internal:retrieveUserQuotaSummary" => (
                200,
                "OK",
                serde_json::json!({
                    "groups": [
                        {
                            "displayName": "Gemini Models",
                            "buckets": [
                                {"window": "weekly", "remainingFraction": 0.52, "resetTime": "2026-11-18T00:00:00Z"},
                                {"window": "5h", "remainingFraction": 0.82, "resetTime": "2026-11-13T18:00:00Z"},
                                {"window": "daily", "remainingAmount": "10", "disabled": false},
                                {"window": "5h", "remainingFraction": 0.0, "disabled": true}
                            ]
                        },
                        {
                            "displayName": "Claude and GPT models",
                            "buckets": [
                                {"window": "weekly", "remainingFraction": 0.18, "resetTime": "2026-11-19T00:00:00Z"},
                                {"window": "5h", "remainingFraction": 0.65, "resetTime": "2026-11-13T16:00:00Z"}
                            ]
                        }
                    ]
                })
                .to_string(),
            ),
            other => panic!("unexpected path: {other}"),
        }
        });

        let reading = direct_quota::fetch_with_options(
            &env,
            Duration::from_secs(2),
            Duration::from_secs(2),
            Some(direct_quota::EndpointConfig {
                oauth_token_url: &format!("{base_url}/oauth2/token"),
                cloud_code_url: &base_url,
            }),
            || {
                Ok(Some(
                    serde_json::json!({
                        "token": {
                            "access_token": "unexpired-secret",
                            "refresh_token": "refresh-secret",
                            "expiry": "2099-01-01T00:00:00Z"
                        },
                        "auth_method": "consumer"
                    })
                    .to_string(),
                ))
            },
        )
        .unwrap();

        server.join().unwrap();
        let reqs: Vec<_> = rx.try_iter().collect();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].path, "/v1internal:loadCodeAssist");
        assert_eq!(
            reqs[0].headers.get("authorization").map(String::as_str),
            Some("Bearer unexpired-secret")
        );
        assert!(reqs[0]
            .headers
            .get("user-agent")
            .is_some_and(|ua| ua.starts_with("Runner/")));
        assert_eq!(
            serde_json::from_str::<Value>(&reqs[0].body).unwrap(),
            serde_json::json!({"metadata": {"ideType": "ANTIGRAVITY"}})
        );
        assert_eq!(reqs[1].path, "/v1internal:retrieveUserQuotaSummary");
        assert_eq!(
            serde_json::from_str::<Value>(&reqs[1].body).unwrap(),
            serde_json::json!({"project": "projects/live-proj"})
        );

        assert_eq!(reading.windows.len(), 4);
        assert_eq!(reading.windows[0].name, "Gemini Models · 5 hours used");
        assert!((reading.windows[0].used_percent - 18.0).abs() < 1e-6);
        assert_eq!(reading.windows[1].name, "Gemini Models · Week used");
        assert!((reading.windows[1].used_percent - 48.0).abs() < 1e-6);
        assert_eq!(
            reading.windows[2].name,
            "Claude and GPT models · 5 hours used"
        );
        assert!((reading.windows[2].used_percent - 35.0).abs() < 1e-6);
        assert_eq!(reading.windows[3].name, "Claude and GPT models · Week used");
        assert!((reading.windows[3].used_percent - 82.0).abs() < 1e-6);
    }

    #[test]
    fn direct_quota_refreshes_expired_token_in_memory_without_writing_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = LoginShellEnv::default();
        env.vars.insert(
            "USERPROFILE".into(),
            dir.path().to_string_lossy().into_owned(),
        );
        env.vars
            .insert("HOME".into(), dir.path().to_string_lossy().into_owned());

        let token_path = dir
            .path()
            .join(".gemini")
            .join("antigravity-cli")
            .join("antigravity-oauth-token");
        std::fs::create_dir_all(token_path.parent().unwrap()).unwrap();
        let original_contents = serde_json::json!({
            "token": {
                "access_token": "stale-token",
                "refresh_token": "refresh-secret",
                "expiry": "2020-01-01T00:00:00Z"
            },
            "auth_method": "gcp",
            "project_id": "fallback-gcp-proj"
        })
        .to_string();
        std::fs::write(&token_path, &original_contents).unwrap();

        let (base_url, rx, server) =
            spawn_fake_http_server(3, |_idx, req| match req.path.as_str() {
                "/oauth2/token" => (
                    200,
                    "OK",
                    serde_json::json!({
                        "access_token": "fresh-in-memory-token",
                        "refresh_token": "rotated-not-saved",
                        "expires_in": 3600
                    })
                    .to_string(),
                ),
                "/v1internal:loadCodeAssist" => (200, "OK", "{}".into()),
                "/v1internal:retrieveUserQuotaSummary" => (
                    200,
                    "OK",
                    serde_json::json!({
                        "groups": [{
                            "displayName": "Gemini Models",
                            "buckets": [{"window": "5h", "remainingFraction": 0.9}]
                        }]
                    })
                    .to_string(),
                ),
                other => panic!("unexpected path: {other}"),
            });

        let gcp_creds = direct_quota::oauth_client_for_method(Some("gcp"));
        let consumer_creds = direct_quota::oauth_client_for_method(None);
        assert_ne!(gcp_creds.client_id, consumer_creds.client_id);
        assert_ne!(gcp_creds.client_secret, consumer_creds.client_secret);
        assert_eq!(
            gcp_creds.default_cloud_code_url,
            "https://cloudcode-pa.googleapis.com"
        );
        assert_eq!(
            consumer_creds.default_cloud_code_url,
            "https://daily-cloudcode-pa.googleapis.com"
        );

        let reading = direct_quota::fetch_with_options(
            &env,
            Duration::from_secs(2),
            Duration::from_secs(2),
            Some(direct_quota::EndpointConfig {
                oauth_token_url: &format!("{base_url}/oauth2/token"),
                cloud_code_url: &base_url,
            }),
            || Ok(None),
        )
        .unwrap();

        server.join().unwrap();
        let reqs: Vec<_> = rx.try_iter().collect();
        assert_eq!(reqs.len(), 3);
        assert_eq!(reqs[0].path, "/oauth2/token");
        assert!(reqs[0].body.contains("grant_type=refresh_token"));
        assert!(reqs[0].body.contains("refresh_token=refresh-secret"));
        assert!(reqs[0].body.contains("client_id=884354919052"));
        assert_eq!(
            reqs[1].headers.get("authorization").map(String::as_str),
            Some("Bearer fresh-in-memory-token")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&reqs[2].body).unwrap(),
            serde_json::json!({"project": "fallback-gcp-proj"})
        );
        assert_eq!(reading.windows.len(), 1);
        assert!((reading.windows[0].used_percent - 10.0).abs() < 1e-6);

        // Credential on disk must remain strictly unmodified.
        assert_eq!(
            std::fs::read_to_string(&token_path).unwrap(),
            original_contents
        );
    }

    #[test]
    fn direct_quota_retries_once_after_401_with_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = LoginShellEnv::default();
        env.vars
            .insert("HOME".into(), dir.path().to_string_lossy().into_owned());

        let (base_url, rx, server) =
            spawn_fake_http_server(4, |_idx, req| match req.path.as_str() {
                "/v1internal:loadCodeAssist"
                    if req.headers.get("authorization").map(String::as_str)
                        == Some("Bearer old-tok") =>
                {
                    (401, "Unauthorized", r#"{"error":{"code":401}}"#.into())
                }
                "/oauth2/token" => (
                    200,
                    "OK",
                    serde_json::json!({"access_token": "retried-tok"}).to_string(),
                ),
                "/v1internal:loadCodeAssist" => (
                    200,
                    "OK",
                    serde_json::json!({"cloudaicompanionProject": "proj-after-retry"}).to_string(),
                ),
                "/v1internal:retrieveUserQuotaSummary" => (
                    200,
                    "OK",
                    serde_json::json!({
                        "groups": [{
                            "displayName": "Gemini Models",
                            "buckets": [{"window": "weekly", "remainingFraction": 0.25}]
                        }]
                    })
                    .to_string(),
                ),
                other => panic!("unexpected path: {other}"),
            });

        let reading = direct_quota::fetch_with_options(
            &env,
            Duration::from_secs(2),
            Duration::from_secs(2),
            Some(direct_quota::EndpointConfig {
                oauth_token_url: &format!("{base_url}/oauth2/token"),
                cloud_code_url: &base_url,
            }),
            || {
                Ok(Some(
                    r#"{"access_token":"old-tok","refresh_token":"ref-tok"}"#.into(),
                ))
            },
        )
        .unwrap();

        server.join().unwrap();
        assert_eq!(rx.try_iter().count(), 4);
        assert_eq!(reading.windows[0].name, "Gemini Models · Week used");
        assert!((reading.windows[0].used_percent - 75.0).abs() < 1e-6);
    }

    #[test]
    fn direct_quota_classifies_auth_rejections_as_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = LoginShellEnv::default();
        env.vars
            .insert("HOME".into(), dir.path().to_string_lossy().into_owned());

        // Missing credential -> SignIn
        assert_eq!(
            direct_quota::fetch_with_options(
                &env,
                Duration::from_secs(1),
                Duration::from_secs(1),
                None,
                || Ok(None),
            ),
            Err(UnavailableReason::SignIn)
        );

        // OAuth invalid_grant (400) -> SignIn
        let (base_url, _rx, server) = spawn_fake_http_server(1, |_, _| {
            (400, "Bad Request", r#"{"error":"invalid_grant"}"#.into())
        });
        assert_eq!(
            direct_quota::fetch_with_options(
                &env,
                Duration::from_secs(2),
                Duration::from_secs(2),
                Some(direct_quota::EndpointConfig {
                    oauth_token_url: &format!("{base_url}/token"),
                    cloud_code_url: &base_url,
                }),
                || {
                    Ok(Some(
                        r#"{"access_token":"","refresh_token":"revoked"}"#.into(),
                    ))
                },
            ),
            Err(UnavailableReason::SignIn)
        );
        server.join().unwrap();

        // Cloud Code 401 when refresh retry also gets 401 -> SignIn
        let (base_url, _rx, server) = spawn_fake_http_server(2, |_, req| {
            if req.path == "/token" {
                (200, "OK", r#"{"access_token":"still-rejected"}"#.into())
            } else {
                (401, "Unauthorized", "{}".into())
            }
        });
        assert_eq!(
            direct_quota::fetch_with_options(
                &env,
                Duration::from_secs(2),
                Duration::from_secs(2),
                Some(direct_quota::EndpointConfig {
                    oauth_token_url: &format!("{base_url}/token"),
                    cloud_code_url: &base_url,
                }),
                || {
                    Ok(Some(
                        r#"{"access_token":"","refresh_token":"valid-ref"}"#.into(),
                    ))
                },
            ),
            Err(UnavailableReason::SignIn)
        );
        server.join().unwrap();
    }

    #[test]
    fn direct_quota_classifies_transient_and_malformed_failures_as_no_answer() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = LoginShellEnv::default();
        env.vars
            .insert("HOME".into(), dir.path().to_string_lossy().into_owned());

        for (status, reason, body) in [
            (429, "Too Many Requests", "{}"),
            (500, "Internal Server Error", "{}"),
        ] {
            let body = body.to_owned();
            let (base_url, _rx, server) =
                spawn_fake_http_server(1, move |_, _| (status, reason, body.clone()));
            assert_eq!(
                direct_quota::fetch_with_options(
                    &env,
                    Duration::from_secs(2),
                    Duration::from_secs(2),
                    Some(direct_quota::EndpointConfig {
                        oauth_token_url: &format!("{base_url}/token"),
                        cloud_code_url: &base_url,
                    }),
                    || { Ok(Some(r#"{"access_token":"","refresh_token":"ref"}"#.into(),)) },
                ),
                Err(UnavailableReason::AntigravityNoAnswer)
            );
            server.join().unwrap();
        }

        // Unparseable / empty quota response -> AntigravityNoAnswer
        for quota_body in [
            "not-json",
            "{}",
            r#"{"groups":[{"displayName":"Gemini Models","buckets":[{"window":"5h","remainingFraction":1.5}]}]}"#,
        ] {
            let quota_body = quota_body.to_owned();
            let (base_url, _rx, server) =
                spawn_fake_http_server(2, move |_, req| match req.path.as_str() {
                    "/v1internal:loadCodeAssist" => (200, "OK", "{}".into()),
                    _ => (200, "OK", quota_body.clone()),
                });
            assert_eq!(
                direct_quota::fetch_with_options(
                    &env,
                    Duration::from_secs(2),
                    Duration::from_secs(2),
                    Some(direct_quota::EndpointConfig {
                        oauth_token_url: &format!("{base_url}/token"),
                        cloud_code_url: &base_url,
                    }),
                    || {
                        Ok(Some(
                            r#"{"access_token":"tok","expiry":"2099-01-01T00:00:00Z"}"#.into(),
                        ))
                    },
                ),
                Err(UnavailableReason::AntigravityNoAnswer)
            );
            server.join().unwrap();
        }

        // Slow server exceeds usage_timeout -> AntigravityNoAnswer
        let (base_url, _rx, server) = spawn_fake_http_server(1, |_, _| {
            std::thread::sleep(Duration::from_millis(350));
            (200, "OK", "{}".into())
        });
        assert_eq!(
            direct_quota::fetch_with_options(
                &env,
                Duration::from_millis(120),
                Duration::from_millis(120),
                Some(direct_quota::EndpointConfig {
                    oauth_token_url: &format!("{base_url}/token"),
                    cloud_code_url: &base_url,
                }),
                || {
                    Ok(Some(
                        r#"{"access_token":"tok","expiry":"2099-01-01T00:00:00Z"}"#.into(),
                    ))
                },
            ),
            Err(UnavailableReason::AntigravityNoAnswer)
        );
        server.join().unwrap();

        // Cloud Code 429 / 503 -> AntigravityNoAnswer
        for (status, reason) in [(429, "Too Many Requests"), (503, "Service Unavailable")] {
            let (base_url, _rx, server) =
                spawn_fake_http_server(1, move |_, _| (status, reason, "{}".into()));
            assert_eq!(
                direct_quota::fetch_with_options(
                    &env,
                    Duration::from_secs(2),
                    Duration::from_secs(2),
                    Some(direct_quota::EndpointConfig {
                        oauth_token_url: &format!("{base_url}/token"),
                        cloud_code_url: &base_url,
                    }),
                    || {
                        Ok(Some(
                            r#"{"access_token":"tok","expiry":"2099-01-01T00:00:00Z"}"#.into(),
                        ))
                    },
                ),
                Err(UnavailableReason::AntigravityNoAnswer)
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn direct_quota_respects_antigravity_gemini_dir_override() {
        let dir = tempfile::tempdir().unwrap();
        let home_dir = dir.path().join("home");
        let custom_dir = dir.path().join("custom-cli");
        std::fs::create_dir_all(&home_dir).unwrap();
        std::fs::create_dir_all(&custom_dir).unwrap();
        std::fs::write(
            custom_dir.join("antigravity-oauth-token"),
            r#"{"access_token":"custom-tok","refresh_token":"custom-ref"}"#,
        )
        .unwrap();

        let mut env = LoginShellEnv::default();
        env.vars
            .insert("HOME".into(), home_dir.to_string_lossy().into_owned());
        env.vars.insert(
            "ANTIGRAVITY_GEMINI_DIR".into(),
            custom_dir.to_string_lossy().into_owned(),
        );

        let cred = direct_quota::load_stored_credential(&env, || Ok(None)).unwrap();
        assert_eq!(cred.access_token, "custom-tok");
        assert_eq!(cred.refresh_token, "custom-ref");
    }
}
