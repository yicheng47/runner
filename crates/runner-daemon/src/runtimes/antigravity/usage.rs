use crate::shell_path::LoginShellEnv;
use crate::usage::{parse_timestamp, UnavailableReason, UsageWindow};
use serde_json::Value;
use std::time::Duration;

pub(crate) fn parse_antigravity(value: &Value) -> Result<Vec<UsageWindow>, UnavailableReason> {
    if value.get("status").and_then(Value::as_str) != Some("SUCCESS")
        || value.pointer("/command/name").and_then(Value::as_str) != Some("usage")
    {
        return Err(UnavailableReason::InvalidResponse);
    }
    let groups = value
        .pointer("/command/data/groups")
        .and_then(Value::as_array)
        .ok_or(UnavailableReason::InvalidResponse)?;
    let mut windows = Vec::new();
    for group in groups {
        let Some(name) = group.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(buckets) = group.get("buckets").and_then(Value::as_array) else {
            continue;
        };
        for bucket in buckets {
            let Some(window) = bucket.get("window").and_then(Value::as_str) else {
                continue;
            };
            let Some(remaining) = bucket
                .get("remaining_fraction")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && (0. ..=1.).contains(value))
            else {
                continue;
            };
            let label = match window {
                "5h" => "5 hours",
                "weekly" => "Week",
                _ => continue,
            };
            windows.push(UsageWindow {
                name: format!("{name} · {label} used"),
                used_percent: (1. - remaining) * 100.,
                resets_at: parse_timestamp(bucket.get("reset_time")),
            });
        }
    }
    if windows.is_empty() {
        Err(UnavailableReason::InvalidResponse)
    } else {
        Ok(windows)
    }
}

pub(crate) fn fetch_antigravity(
    command: &str,
    env: &LoginShellEnv,
) -> Result<Vec<UsageWindow>, UnavailableReason> {
    let output = crate::runtime_status::models::command_output(
        command,
        &["-p", "/usage", "--output-format", "json"],
        env,
        Duration::from_secs(15),
    )
    .ok_or(UnavailableReason::AntigravityNoAnswer)?;
    let value: Value =
        serde_json::from_slice(&output).map_err(|_| UnavailableReason::InvalidResponse)?;
    parse_antigravity(&value)
}
