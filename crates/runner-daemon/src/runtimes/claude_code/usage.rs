use crate::shell_path::LoginShellEnv;
use crate::usage::http_client;
use crate::usage::{parse_timestamp, percent, UnavailableReason, UsageWindow};
use serde_json::Value;
#[cfg(any(target_os = "macos", all(test, unix)))]
use std::io::Read;
#[cfg(any(target_os = "macos", all(test, unix)))]
use std::process::{Command, Stdio};
#[cfg(any(target_os = "macos", all(test, unix)))]
use std::thread;
#[cfg(any(target_os = "macos", all(test, unix)))]
use std::time::{Duration, Instant};
#[cfg(target_os = "macos")]
const KEYCHAIN_TIMEOUT: Duration = Duration::from_secs(120);

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

pub(crate) fn parse_claude(value: &Value) -> Result<Vec<UsageWindow>, UnavailableReason> {
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

pub(crate) fn parse_credentials(bytes: &[u8]) -> Result<String, UnavailableReason> {
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
pub(crate) fn keychain_account(user: Option<&str>) -> &str {
    user.filter(|name| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    })
    .unwrap_or("claude-code-user")
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn keychain_error_reason(code: Option<i32>, stderr: &[u8]) -> UnavailableReason {
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

#[cfg(any(target_os = "macos", all(test, unix)))]
pub(crate) fn read_claude_credentials_with(
    command: &std::path::Path,
    account: &str,
    timeout: Duration,
) -> Result<Vec<u8>, UnavailableReason> {
    let mut planned = Command::new(command);
    planned
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-a",
            account,
            "-w",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(test)]
    if crate::golden::record_command(&planned, None, timeout) {
        return Ok(br#"{"claudeAiOauth":{"accessToken":"golden-token"}}"#.to_vec());
    }
    let mut child = planned
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
pub(crate) fn read_claude_credentials_file(
    path: &std::path::Path,
) -> Result<Vec<u8>, UnavailableReason> {
    #[cfg(test)]
    if crate::golden::record(serde_json::json!({"credentials_file": path})) {
        return Ok(br#"{"claudeAiOauth":{"accessToken":"golden-token"}}"#.to_vec());
    }
    std::fs::read(path).map_err(|_| UnavailableReason::SignIn)
}

pub(crate) fn claude_response(
    status: u16,
    value: &Value,
) -> Result<Vec<UsageWindow>, UnavailableReason> {
    if matches!(status, 401 | 403) {
        return Err(UnavailableReason::SignIn);
    }
    if !(200..300).contains(&status) {
        return Err(UnavailableReason::ClaudeUnreachable);
    }
    parse_claude(value)
}

pub(crate) fn fetch_claude(env: &LoginShellEnv) -> Result<Vec<UsageWindow>, UnavailableReason> {
    let token = parse_credentials(&read_claude_credentials()?)?;
    let client = http_client(env).map_err(|_| UnavailableReason::ClaudeUnreachable)?;
    let request = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20");
    #[cfg(test)]
    {
        let planned = request.try_clone().unwrap().build().unwrap();
        if crate::golden::record(
            serde_json::json!({"url": planned.url().as_str(), "method": planned.method().as_str(), "headers": planned.headers().iter().map(|(key, value)| (key.as_str(), value.to_str().unwrap())).collect::<std::collections::BTreeMap<_,_>>(), "proxy_env": env.vars}),
        ) {
            return Err(UnavailableReason::ClaudeUnreachable);
        }
    }
    let response = request
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
