use super::*;
#[cfg(unix)]
use crate::runtimes::{
    claude_code::usage::read_claude_credentials_with, codex::usage::fetch_codex_with_timeout,
};
use crate::runtimes::{
    claude_code::usage::{
        claude_response, keychain_account, keychain_error_reason, parse_claude, parse_credentials,
        read_claude_credentials_file,
    },
    codex::usage::parse_codex,
};
use chrono::TimeDelta;

const SCHEDULE_INTERVAL: Duration = Duration::from_secs(15 * 60);

fn schedule_wait(last: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Duration {
    schedule_wait_with_interval(last, now, SCHEDULE_INTERVAL)
}

fn refresh_due(last: Option<DateTime<Utc>>, reason: RefreshReason, now: DateTime<Utc>) -> bool {
    refresh_due_with_interval(last, reason, now, SCHEDULE_INTERVAL)
}

#[test]
fn managed_clients_pause_and_resume_polling_at_zero() {
    use std::sync::atomic::Ordering;
    let core = crate::test_support::test_core();
    // Keep an existing discovery pass in flight so this test starts no probes.
    core.runtime_discovery.write().unwrap().checking = true;
    core.usage.discovery_started.store(true, Ordering::Release);
    core.usage.manage_client_lifetime(false);
    assert!(!core.usage.polling_enabled.load(Ordering::Acquire));
    core.usage.client_connected(core.clone());
    core.usage.client_connected(core.clone());
    assert!(core.usage.polling_enabled.load(Ordering::Acquire));
    core.usage.client_disconnected();
    assert!(core.usage.polling_enabled.load(Ordering::Acquire));
    core.usage.client_disconnected();
    assert!(!core.usage.polling_enabled.load(Ordering::Acquire));
    core.usage.client_connected(core.clone());
    assert!(core.usage.polling_enabled.load(Ordering::Acquire));
    core.usage.client_disconnected();
    assert!(!core.usage.polling_enabled.load(Ordering::Acquire));
}

#[test]
fn http_client_accepts_captured_socks_proxy() {
    let mut env = LoginShellEnv::default();
    env.vars
        .insert("ALL_PROXY".into(), "socks5h://127.0.0.1:1080".into());
    http_client(&env).expect("a captured SOCKS proxy should build without a network request");
}

#[test]
fn parses_recorded_answers_by_duration_and_scope() {
    let codex = serde_json::json!({"rateLimits":{"primary":{"usedPercent":27,"windowDurationMins":10080,"resetsAt":1770000000},"secondary":{"usedPercent":4,"windowDurationMins":300,"resetsAt":1770000000},"credits":{"balance":"1250.75","hasCredits":true,"unlimited":false}}});
    let parsed = parse_codex(&codex).unwrap();
    assert_eq!(parsed.credits.unwrap().balance.as_deref(), Some("1250.75"));
    let windows = parsed.windows;
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
    assert_eq!(parse_codex(&answer).unwrap().windows[0].name, "60 min");
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
fn codex_credit_snapshots_include_credits_only_and_unknown_balances() {
    for (balance, has_credits, unlimited) in [
        (Some("1250"), true, false),
        (Some("1250.750012345678901234"), true, false),
        (Some("0"), false, false),
        (None, false, false),
        (Some("1250"), false, true),
        (None, true, false),
    ] {
        let credits = UsageCredits {
            balance: balance.map(str::to_owned),
            has_credits,
            unlimited,
        };
        let parsed = parse_codex(&serde_json::json!({"rateLimits": {"credits": credits}})).unwrap();
        assert!(parsed.windows.is_empty());
        assert_eq!(parsed.credits, Some(credits));
    }
    let missing_balance = parse_codex(
        &serde_json::json!({"rateLimits": {"credits": {"hasCredits": true, "unlimited": false}}}),
    )
    .unwrap();
    assert_eq!(missing_balance.credits.unwrap().balance, None);
    for credits in [Value::Null, serde_json::json!({"balance": "invalid"})] {
        assert_eq!(
            parse_codex(&serde_json::json!({"rateLimits": {"credits": credits}})),
            Err(UnavailableReason::InvalidResponse)
        );
    }
    for credits in [None, Some(Value::Null)] {
        let mut limits =
            serde_json::json!({"primary": {"usedPercent": 27, "windowDurationMins": 10080}});
        if let Some(credits) = credits {
            limits["credits"] = credits;
        }
        let parsed = parse_codex(&serde_json::json!({"rateLimits": limits})).unwrap();
        assert_eq!(parsed.windows.len(), 1);
        assert_eq!(parsed.credits, None);
    }
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
    assert_eq!(windows.windows[0].used_percent, 23.);
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
        }]
        .into()),
        now,
    );
    apply_result(
        &mut good,
        &mut error,
        Err(UnavailableReason::ClaudeUnreachable),
        now + TimeDelta::minutes(1),
    );
    assert_eq!(good.unwrap().updated_at, now);
    assert_eq!(error, Some(UnavailableReason::ClaudeUnreachable));
}
