use crate::runtime_status::direct_chat_path;
use crate::session::process::{prepare_headless_fork, ProcessTree};
use crate::shell_path::LoginShellEnv;
use crate::usage::{parse_timestamp, percent, UnavailableReason, UsageWindow, FETCH_TIMEOUT};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

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

pub(crate) fn parse_codex(value: &Value) -> Result<Vec<UsageWindow>, UnavailableReason> {
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

pub(crate) fn fetch_codex(
    command: &str,
    env: &LoginShellEnv,
) -> Result<Vec<UsageWindow>, UnavailableReason> {
    fetch_codex_with_timeout(command, env, FETCH_TIMEOUT)
}

pub(crate) fn fetch_codex_with_timeout(
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
    #[cfg(test)]
    if crate::golden::record_command(&process, None, timeout) {
        return Err(UnavailableReason::CodexNoAnswer);
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
