#[cfg(any(target_os = "macos", test))]
use std::io::Read;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::model::Runtime;
use crate::runtime_status::{direct_chat_path, RuntimeCommandSource};
use crate::session::process::{prepare_headless_fork, ProcessTree};
use crate::shell_path::LoginShellEnv;
use crate::AppCore;

const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(target_os = "macos")]
const KEYCHAIN_TIMEOUT: Duration = Duration::from_secs(120);
const MIN_REFRESH_VISIBLE: Duration = Duration::from_millis(400);
const SCHEDULE_INTERVAL: Duration = Duration::from_secs(15 * 60);
const OPEN_INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug, PartialEq)]
pub struct UsageWindow {
    pub name: String,
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentUsage {
    pub windows: Vec<UsageWindow>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    SignIn,
    KeychainDenied,
    KeychainUnavailable,
    ClaudeUnreachable,
    CodexNoAnswer,
    InvalidResponse,
}

#[derive(Clone, Debug, Default)]
pub struct UsageSnapshot {
    pub claude: Option<AgentUsage>,
    pub codex: Option<AgentUsage>,
    pub claude_error: Option<UnavailableReason>,
    pub codex_error: Option<UnavailableReason>,
    pub last_fetch_at: Option<DateTime<Utc>>,
    pub refreshing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshReason {
    Launch,
    Schedule,
    Button,
    Open,
}

#[derive(Default)]
struct UsageState {
    snapshot: UsageSnapshot,
    last_attempt_at: Option<DateTime<Utc>>,
    refresh_started_at: Option<Instant>,
    keychain_denied: bool,
}

pub struct UsageService {
    state: Mutex<UsageState>,
    wake_scheduler: Condvar,
    enabled: RwLock<Vec<Runtime>>,
}

impl Default for UsageService {
    fn default() -> Self {
        Self {
            state: Mutex::new(UsageState::default()),
            wake_scheduler: Condvar::new(),
            enabled: RwLock::new(Vec::new()),
        }
    }
}

impl UsageService {
    pub fn snapshot(&self) -> UsageSnapshot {
        self.state.lock().unwrap().snapshot.clone()
    }

    pub fn set_enabled(&self, runtimes: Vec<Runtime>) -> bool {
        let mut enabled = self.enabled.write().unwrap();
        if *enabled == runtimes {
            return false;
        }
        *enabled = runtimes;
        true
    }

    pub fn enabled(&self) -> Vec<Runtime> {
        self.enabled.read().unwrap().clone()
    }

    fn begin_refresh(&self, reason: RefreshReason, now: DateTime<Utc>) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.snapshot.refreshing || !refresh_due(state.last_attempt_at, reason, now) {
            return false;
        }
        state.last_attempt_at = Some(now);
        state.refresh_started_at = Some(Instant::now());
        state.snapshot.refreshing = true;
        self.wake_scheduler.notify_one();
        true
    }

    pub fn request_refresh(self: &Arc<Self>, core: AppCore, reason: RefreshReason) {
        if !self.begin_refresh(reason, Utc::now()) {
            return;
        }
        core.events.emit("usage/updated", &());
        let service = Arc::clone(self);
        thread::spawn(move || {
            service.fetch(&core);
            core.events.emit("usage/updated", &());
        });
    }

    pub fn start_scheduler(self: &Arc<Self>, core: AppCore) {
        let service = Arc::clone(self);
        thread::spawn(move || {
            while core
                .runtime_discovery
                .read()
                .is_ok_and(|state| state.checking)
            {
                thread::sleep(Duration::from_millis(100));
            }
            service.request_refresh(core.clone(), RefreshReason::Launch);
            let mut state = service.state.lock().unwrap();
            loop {
                let wait = schedule_wait(state.last_attempt_at, Utc::now());
                if wait.is_zero() && !state.snapshot.refreshing {
                    drop(state);
                    service.request_refresh(core.clone(), RefreshReason::Schedule);
                    state = service.state.lock().unwrap();
                } else {
                    state = service
                        .wake_scheduler
                        .wait_timeout(state, wait.max(Duration::from_millis(100)))
                        .unwrap()
                        .0;
                }
            }
        });
    }

    fn fetch(&self, core: &AppCore) {
        let statuses = crate::runtime_status::status_list(
            &core.db,
            &core.runtime_shell_env,
            &core.runtime_discovery,
        );
        let env = core.runtime_shell_env.read().unwrap().clone();
        let enabled = self.enabled();
        let mut claude = None;
        let mut codex = None;
        if let Ok(statuses) = statuses {
            for status in statuses.runtimes {
                if !enabled.contains(&status.name)
                    || !matches!(
                        status.effective_source,
                        Some(RuntimeCommandSource::Detected | RuntimeCommandSource::Override)
                    )
                {
                    continue;
                }
                let Some(command) = status.effective_command else {
                    continue;
                };
                match status.name {
                    Runtime::ClaudeCode => {
                        let denied = self.state.lock().unwrap().keychain_denied;
                        claude = Some(if denied {
                            Err(UnavailableReason::KeychainDenied)
                        } else {
                            fetch_claude(&env)
                        });
                    }
                    Runtime::Codex => codex = Some(fetch_codex(&command, &env)),
                    _ => {}
                }
            }
        }
        let remaining = self
            .state
            .lock()
            .unwrap()
            .refresh_started_at
            .map(|started| refresh_visible_remaining(started, Instant::now()))
            .unwrap_or_default();
        if !remaining.is_zero() {
            thread::sleep(remaining);
        }
        let now = Utc::now();
        let mut state = self.state.lock().unwrap();
        if let Some(result) = claude {
            if result == Err(UnavailableReason::KeychainDenied) {
                state.keychain_denied = true;
            }
            let snapshot = &mut state.snapshot;
            apply_result(
                &mut snapshot.claude,
                &mut snapshot.claude_error,
                result,
                now,
            );
        }
        if let Some(result) = codex {
            let snapshot = &mut state.snapshot;
            apply_result(&mut snapshot.codex, &mut snapshot.codex_error, result, now);
        }
        state.snapshot.last_fetch_at = Some(now);
        state.snapshot.refreshing = false;
        state.refresh_started_at = None;
        self.wake_scheduler.notify_one();
    }
}

fn refresh_visible_remaining(started: Instant, now: Instant) -> Duration {
    MIN_REFRESH_VISIBLE.saturating_sub(now.saturating_duration_since(started))
}

fn schedule_wait(last: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Duration {
    last.and_then(|last| {
        (last + chrono::TimeDelta::seconds(SCHEDULE_INTERVAL.as_secs() as i64) - now)
            .to_std()
            .ok()
    })
    .unwrap_or(Duration::ZERO)
}

fn refresh_due(last: Option<DateTime<Utc>>, reason: RefreshReason, now: DateTime<Utc>) -> bool {
    let interval = match reason {
        RefreshReason::Button => return true,
        RefreshReason::Launch => Duration::ZERO,
        RefreshReason::Schedule => SCHEDULE_INTERVAL,
        RefreshReason::Open => OPEN_INTERVAL,
    };
    last.is_none_or(|last| {
        now.signed_duration_since(last).to_std().is_ok_and(|age| {
            if reason == RefreshReason::Schedule {
                age >= interval
            } else {
                age > interval
            }
        })
    })
}

fn apply_result(
    current: &mut Option<AgentUsage>,
    error: &mut Option<UnavailableReason>,
    result: Result<Vec<UsageWindow>, UnavailableReason>,
    now: DateTime<Utc>,
) {
    match result {
        Ok(windows) => {
            *current = Some(AgentUsage {
                windows,
                updated_at: now,
            });
            *error = None;
        }
        Err(reason) if current.is_none() => *error = Some(reason),
        Err(_) => {}
    }
}

fn parse_timestamp(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let value = value?;
    if let Some(seconds) = value
        .as_i64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
    {
        let millis = if seconds > 10_000_000_000 {
            seconds
        } else {
            seconds.checked_mul(1000)?
        };
        return DateTime::from_timestamp_millis(millis);
    }
    value.as_str()?.parse().ok()
}

fn percent(value: Option<&Value>) -> Option<f64> {
    value?
        .as_f64()
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(0., 100.))
}

fn parse_codex_window(value: &Value) -> Option<UsageWindow> {
    let duration = value.get("windowDurationMins")?.as_u64()?;
    let name = match duration {
        299..=301 => "5 hours".to_owned(),
        10079..=10081 => "Week".to_owned(),
        _ => format!("{duration} min"),
    };
    Some(UsageWindow {
        name,
        used_percent: percent(value.get("usedPercent"))?,
        resets_at: parse_timestamp(value.get("resetsAt")),
    })
}

fn parse_codex(value: &Value) -> Result<Vec<UsageWindow>, UnavailableReason> {
    let limits = value
        .get("rateLimits")
        .ok_or(UnavailableReason::InvalidResponse)?;
    let mut windows: Vec<_> = ["primary", "secondary"]
        .into_iter()
        .filter_map(|name| limits.get(name).and_then(parse_codex_window))
        .collect();
    windows.sort_by_key(|window| match window.name.as_str() {
        "5 hours" => 0,
        "Week" => 1,
        _ => 2,
    });
    if windows.is_empty() {
        Err(UnavailableReason::InvalidResponse)
    } else {
        Ok(windows)
    }
}

fn parse_claude_window(value: Option<&Value>, name: String) -> Option<UsageWindow> {
    let value = value?;
    Some(UsageWindow {
        name,
        used_percent: percent(
            value
                .get("utilization")
                .or_else(|| value.get("used_percentage"))
                .or_else(|| value.get("percent")),
        )?,
        resets_at: parse_timestamp(value.get("resets_at")),
    })
}

fn parse_claude(value: &Value) -> Result<Vec<UsageWindow>, UnavailableReason> {
    let mut windows = Vec::new();
    if let Some(window) = parse_claude_window(value.get("five_hour"), "5 hours".into()) {
        windows.push(window);
    }
    if let Some(window) = parse_claude_window(value.get("seven_day"), "Week".into()) {
        windows.push(window);
    }
    if let Some(limits) = value.get("limits").and_then(Value::as_array) {
        for limit in limits
            .iter()
            .filter(|limit| limit.get("kind").and_then(Value::as_str) == Some("weekly_scoped"))
        {
            let Some(model) = limit
                .pointer("/scope/model/display_name")
                .and_then(Value::as_str)
                .filter(|model| !model.trim().is_empty())
            else {
                continue;
            };
            if let Some(window) = parse_claude_window(Some(limit), format!("{model} · week")) {
                windows.push(window);
            }
        }
    }
    if windows.is_empty() {
        Err(UnavailableReason::InvalidResponse)
    } else {
        Ok(windows)
    }
}

fn parse_credentials(bytes: &[u8]) -> Result<String, UnavailableReason> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| UnavailableReason::SignIn)?;
    value
        .pointer("/claudeAiOauth/accessToken")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .ok_or(UnavailableReason::SignIn)
}

#[cfg(target_os = "macos")]
fn read_claude_credentials() -> Result<Vec<u8>, UnavailableReason> {
    let user = std::env::var("USER").ok();
    read_claude_credentials_with(
        std::path::Path::new("/usr/bin/security"),
        keychain_account(user.as_deref()),
        KEYCHAIN_TIMEOUT,
    )
}

#[cfg(any(target_os = "macos", test))]
fn keychain_account(user: Option<&str>) -> &str {
    user.filter(|name| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    })
    .unwrap_or("claude-code-user")
}

#[cfg(any(target_os = "macos", test))]
fn keychain_error_reason(code: Option<i32>, stderr: &[u8]) -> UnavailableReason {
    let message = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    match code {
        Some(44) => UnavailableReason::SignIn,
        Some(51 | 128) => UnavailableReason::KeychainDenied,
        _ if message.contains("user canceled")
            || message.contains("user cancelled")
            || message.contains("user denied") =>
        {
            UnavailableReason::KeychainDenied
        }
        _ => UnavailableReason::KeychainUnavailable,
    }
}

#[cfg(any(target_os = "macos", test))]
fn read_claude_credentials_with(
    command: &std::path::Path,
    account: &str,
    timeout: Duration,
) -> Result<Vec<u8>, UnavailableReason> {
    let mut child = Command::new(command)
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-a",
            account,
            "-w",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| UnavailableReason::KeychainUnavailable)?;
    let mut stdout = child.stdout.take().unwrap();
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let mut stderr = child.stderr.take().unwrap();
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let stdout = stdout_reader.join().ok().and_then(Result::ok);
    let stderr = stderr_reader.join().ok().and_then(Result::ok);
    let status = status.ok_or(UnavailableReason::KeychainUnavailable)?;
    let stdout = stdout.ok_or(UnavailableReason::KeychainUnavailable)?;
    if status.success() {
        Ok(stdout)
    } else {
        let stderr = stderr.ok_or(UnavailableReason::KeychainUnavailable)?;
        Err(keychain_error_reason(status.code(), &stderr))
    }
}

#[cfg(not(target_os = "macos"))]
fn read_claude_credentials() -> Result<Vec<u8>, UnavailableReason> {
    let home = runner_core::app_paths::home_dir().ok_or(UnavailableReason::SignIn)?;
    read_claude_credentials_file(&home.join(".claude").join(".credentials.json"))
}

#[cfg(any(not(target_os = "macos"), test))]
fn read_claude_credentials_file(path: &std::path::Path) -> Result<Vec<u8>, UnavailableReason> {
    std::fs::read(path).map_err(|_| UnavailableReason::SignIn)
}

fn claude_response(status: u16, value: &Value) -> Result<Vec<UsageWindow>, UnavailableReason> {
    if matches!(status, 401 | 403) {
        return Err(UnavailableReason::SignIn);
    }
    if !(200..300).contains(&status) {
        return Err(UnavailableReason::ClaudeUnreachable);
    }
    parse_claude(value)
}

fn fetch_claude(env: &LoginShellEnv) -> Result<Vec<UsageWindow>, UnavailableReason> {
    let token = parse_credentials(&read_claude_credentials()?)?;
    let client = http_client(env).map_err(|_| UnavailableReason::ClaudeUnreachable)?;
    let response = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .map_err(|_| UnavailableReason::ClaudeUnreachable)?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return claude_response(status, &Value::Null);
    }
    let value: Value = response
        .json()
        .map_err(|_| UnavailableReason::InvalidResponse)?;
    claude_response(status, &value)
}

fn http_client(env: &LoginShellEnv) -> Result<reqwest::blocking::Client, reqwest::Error> {
    let mut builder = reqwest::blocking::Client::builder()
        .user_agent(concat!("Runner/", env!("CARGO_PKG_VERSION")))
        .no_proxy()
        .timeout(FETCH_TIMEOUT);
    let no_proxy = env
        .vars
        .get("NO_PROXY")
        .or_else(|| env.vars.get("no_proxy"))
        .and_then(|value| reqwest::NoProxy::from_string(value));
    for (scheme, names) in [
        ("http", ["HTTP_PROXY", "http_proxy"]),
        ("https", ["HTTPS_PROXY", "https_proxy"]),
    ] {
        if let Some(url) = names
            .iter()
            .find_map(|name| env.vars.get(*name))
            .or_else(|| env.vars.get("ALL_PROXY"))
            .or_else(|| env.vars.get("all_proxy"))
        {
            let proxy = if scheme == "http" {
                reqwest::Proxy::http(url)?
            } else {
                reqwest::Proxy::https(url)?
            };
            builder = builder.proxy(proxy.no_proxy(no_proxy.clone()));
        }
    }
    builder.build()
}

fn fetch_codex(command: &str, env: &LoginShellEnv) -> Result<Vec<UsageWindow>, UnavailableReason> {
    fetch_codex_with_timeout(command, env, FETCH_TIMEOUT)
}

fn fetch_codex_with_timeout(
    command: &str,
    env: &LoginShellEnv,
    timeout: Duration,
) -> Result<Vec<UsageWindow>, UnavailableReason> {
    let mut process = Command::new(command);
    process
        .args([
            "-c",
            "approval_policy=never",
            "-c",
            "features.plugins=false",
            "-s",
            "read-only",
            "-a",
            "never",
            "app-server",
        ])
        .envs(&env.vars)
        .env("PATH", direct_chat_path(env))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(home) = runner_core::app_paths::home_dir() {
        process.current_dir(home);
    }
    prepare_headless_fork(&mut process);
    let mut child = process
        .spawn()
        .map_err(|_| UnavailableReason::CodexNoAnswer)?;
    let tree = ProcessTree::adopt(child.id()).map_err(|_| {
        let _ = child.kill();
        let _ = child.wait();
        UnavailableReason::CodexNoAnswer
    })?;
    let result = (|| {
        let mut stdin = child.stdin.take().ok_or(UnavailableReason::CodexNoAnswer)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(UnavailableReason::CodexNoAnswer)?;
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        write_rpc(
            &mut stdin,
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"Runner","version":env!("CARGO_PKG_VERSION")}}}),
        )?;
        let deadline = std::time::Instant::now() + timeout;
        let mut initialized = false;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let line = rx
                .recv_timeout(remaining)
                .map_err(|_| UnavailableReason::CodexNoAnswer)?
                .map_err(|_| UnavailableReason::CodexNoAnswer)?;
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value.get("method").is_some() {
                continue;
            }
            match value.get("id").and_then(Value::as_u64) {
                Some(1) if !initialized => {
                    if value.get("error").is_some() {
                        return Err(UnavailableReason::CodexNoAnswer);
                    }
                    write_rpc(
                        &mut stdin,
                        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
                    )?;
                    write_rpc(
                        &mut stdin,
                        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"account/rateLimits/read","params":{}}),
                    )?;
                    initialized = true;
                }
                Some(2) if initialized => {
                    if value.get("error").is_some() {
                        return Err(UnavailableReason::CodexNoAnswer);
                    }
                    return parse_codex(
                        value
                            .get("result")
                            .ok_or(UnavailableReason::InvalidResponse)?,
                    );
                }
                _ => {}
            }
        }
    })();
    shutdown_codex(&mut child, &tree);
    result
}

fn shutdown_codex(child: &mut std::process::Child, tree: &ProcessTree) {
    drop(child.stdin.take());
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let exited = child.try_wait().is_ok_and(|status| status.is_some());
        #[cfg(unix)]
        let drained = exited && unsafe { libc::kill(-(child.id() as i32), 0) } != 0;
        #[cfg(windows)]
        let drained = exited;
        if drained {
            break;
        }
        if std::time::Instant::now() >= deadline {
            let _ = tree.terminate();
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let _ = child.wait();
}

fn write_rpc(writer: &mut impl Write, value: Value) -> Result<(), UnavailableReason> {
    serde_json::to_writer(&mut *writer, &value).map_err(|_| UnavailableReason::CodexNoAnswer)?;
    writer
        .write_all(b"\n")
        .map_err(|_| UnavailableReason::CodexNoAnswer)?;
    writer.flush().map_err(|_| UnavailableReason::CodexNoAnswer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    #[test]
    fn parses_recorded_answers_by_duration_and_scope() {
        let codex = serde_json::json!({"rateLimits":{"primary":{"usedPercent":27,"windowDurationMins":10080,"resetsAt":1770000000},"secondary":{"usedPercent":4,"windowDurationMins":300,"resetsAt":1770000000}}});
        let windows = parse_codex(&codex).unwrap();
        assert_eq!(windows[0].name, "5 hours");
        assert_eq!(windows[1].name, "Week");
        let claude = serde_json::json!({"five_hour":{"utilization":4,"resets_at":"2026-09-23T12:00:00Z"},"seven_day":{"utilization":27},"limits":[{"kind":"weekly_scoped","percent":55,"scope":null},{"kind":"weekly_scoped","percent":80,"scope":{"model":{"display_name":"Fable"}}}]});
        let windows = parse_claude(&claude).unwrap();
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[2].name, "Fable · week");
        assert_eq!(windows[2].used_percent, 80.);
    }

    #[test]
    fn missing_fields_and_unknown_duration() {
        let answer = serde_json::json!({"rateLimits":{"primary":{"usedPercent":14,"windowDurationMins":60},"secondary":{"windowDurationMins":300}}});
        assert_eq!(parse_codex(&answer).unwrap()[0].name, "60 min");
        assert_eq!(
            parse_codex(&serde_json::json!({"rateLimits":{"primary":{}}})),
            Err(UnavailableReason::InvalidResponse)
        );
        assert_eq!(
            parse_claude(&serde_json::json!({"five_hour":{}})),
            Err(UnavailableReason::InvalidResponse)
        );
    }

    #[test]
    fn rejected_token_and_read_only_credentials() {
        assert_eq!(keychain_account(Some("jason.wang-1")), "jason.wang-1");
        assert_eq!(keychain_account(Some("bad account")), "claude-code-user");
        assert_eq!(keychain_account(None), "claude-code-user");
        assert_eq!(
            keychain_error_reason(Some(44), b""),
            UnavailableReason::SignIn
        );
        assert_eq!(
            keychain_error_reason(Some(128), b""),
            UnavailableReason::KeychainDenied
        );
        assert_eq!(
            keychain_error_reason(Some(51), b""),
            UnavailableReason::KeychainDenied
        );
        assert_eq!(
            keychain_error_reason(Some(1), b"User canceled the request"),
            UnavailableReason::KeychainDenied
        );
        assert_eq!(
            keychain_error_reason(Some(1), b"unexpected error"),
            UnavailableReason::KeychainUnavailable
        );
        assert_eq!(
            claude_response(401, &Value::Null),
            Err(UnavailableReason::SignIn)
        );
        assert_eq!(
            claude_response(503, &Value::Null),
            Err(UnavailableReason::ClaudeUnreachable)
        );
        assert_eq!(
            parse_credentials(br#"{"claudeAiOauth":{"accessToken":"secret"}}"#).unwrap(),
            "secret"
        );
        assert_eq!(
            parse_credentials(br#"{"claudeAiOauth":{}}"#),
            Err(UnavailableReason::SignIn)
        );
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".credentials.json");
        std::fs::write(&path, br#"{"claudeAiOauth":{"accessToken":"secret"}}"#).unwrap();
        let before = read_claude_credentials_file(&path).unwrap();
        parse_credentials(&before).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn keychain_command_reads_only_and_times_out() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let command = temp.path().join("fake-security");
        std::fs::write(
            &command,
            b"#!/bin/sh\n[ \"$#\" -eq 6 ] && [ \"$1\" = find-generic-password ] && [ \"$2\" = -s ] && [ \"$3\" = 'Claude Code-credentials' ] && [ \"$4\" = -a ] && [ \"$5\" = jason ] && [ \"$6\" = -w ] || exit 2\nprintf '{\"claudeAiOauth\":{\"accessToken\":\"fixture-token\"}}\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o700)).unwrap();
        let credentials =
            read_claude_credentials_with(&command, "jason", Duration::from_secs(10)).unwrap();
        assert_eq!(parse_credentials(&credentials).unwrap(), "fixture-token");

        std::fs::write(&command, b"#!/bin/sh\nexit 44\n").unwrap();
        assert_eq!(
            read_claude_credentials_with(&command, "jason", Duration::from_secs(10)),
            Err(UnavailableReason::SignIn)
        );
        std::fs::write(&command, b"#!/bin/sh\nprintf 'User canceled' >&2\nexit 1\n").unwrap();
        assert_eq!(
            read_claude_credentials_with(&command, "jason", Duration::from_secs(10)),
            Err(UnavailableReason::KeychainDenied)
        );
        std::fs::write(&command, b"#!/bin/sh\nexit 51\n").unwrap();
        assert_eq!(
            read_claude_credentials_with(&command, "jason", Duration::from_secs(10)),
            Err(UnavailableReason::KeychainDenied)
        );
        std::fs::write(
            &command,
            b"#!/bin/sh\ndd if=/dev/zero bs=70000 count=1 2>/dev/null\ndd if=/dev/zero bs=70000 count=1 1>&2 2>/dev/null\n",
        )
        .unwrap();
        assert_eq!(
            read_claude_credentials_with(&command, "jason", Duration::from_secs(10))
                .unwrap()
                .len(),
            70000
        );
        std::fs::write(&command, b"#!/bin/sh\nexec sleep 1\n").unwrap();
        assert_eq!(
            read_claude_credentials_with(&command, "jason", Duration::from_millis(10)),
            Err(UnavailableReason::KeychainUnavailable)
        );
    }

    #[cfg(unix)]
    #[test]
    fn codex_rpc_handshake_and_timeout() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("fake-codex");
        let marker = temp.path().join("terminated");
        let script = format!("#!/bin/sh\ntrap 'printf term > {} ; exit 0' TERM\nread line\nprintf '%s\\n' '{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{}}}}'\nread line\nread line\nprintf '%s\\n' '{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"rateLimits\":{{\"primary\":{{\"usedPercent\":23,\"windowDurationMins\":300}}}}}}}}'\nwhile :; do sleep 1; done\n", marker.display());
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let windows = fetch_codex_with_timeout(
            path.to_str().unwrap(),
            &LoginShellEnv::default(),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(windows[0].used_percent, 23.);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "term");
        std::fs::write(&path, b"#!/bin/sh\nread line\nexec sleep 1\n").unwrap();
        assert_eq!(
            fetch_codex_with_timeout(
                path.to_str().unwrap(),
                &LoginShellEnv::default(),
                Duration::from_millis(10)
            ),
            Err(UnavailableReason::CodexNoAnswer)
        );
    }

    #[test]
    fn refresh_clock_and_failed_fetch_keep_good_numbers() {
        let now = Utc::now();
        let started = Instant::now();
        assert_eq!(
            refresh_visible_remaining(started, started),
            Duration::from_millis(400)
        );
        assert_eq!(
            refresh_visible_remaining(started, started + Duration::from_millis(250)),
            Duration::from_millis(150)
        );
        assert_eq!(
            refresh_visible_remaining(started, started + Duration::from_millis(400)),
            Duration::ZERO
        );
        assert!(refresh_due(None, RefreshReason::Launch, now));
        assert!(!refresh_due(
            Some(now),
            RefreshReason::Schedule,
            now + TimeDelta::minutes(14)
        ));
        assert!(refresh_due(
            Some(now),
            RefreshReason::Schedule,
            now + TimeDelta::minutes(16)
        ));
        assert!(!refresh_due(
            Some(now),
            RefreshReason::Open,
            now + TimeDelta::minutes(4)
        ));
        assert!(refresh_due(
            Some(now),
            RefreshReason::Open,
            now + TimeDelta::minutes(6)
        ));
        assert!(refresh_due(Some(now), RefreshReason::Button, now));
        assert_eq!(
            schedule_wait(
                Some(now + TimeDelta::minutes(10)),
                now + TimeDelta::minutes(15)
            ),
            Duration::from_secs(10 * 60)
        );
        assert_eq!(
            schedule_wait(
                Some(now + TimeDelta::minutes(10)),
                now + TimeDelta::minutes(25)
            ),
            Duration::ZERO
        );
        let mut good = None;
        let mut error = None;
        apply_result(
            &mut good,
            &mut error,
            Ok(vec![UsageWindow {
                name: "Week".into(),
                used_percent: 30.,
                resets_at: None,
            }]),
            now,
        );
        apply_result(
            &mut good,
            &mut error,
            Err(UnavailableReason::ClaudeUnreachable),
            now + TimeDelta::minutes(1),
        );
        assert_eq!(good.unwrap().updated_at, now);
        assert_eq!(error, None);
    }
}
