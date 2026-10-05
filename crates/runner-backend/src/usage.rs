use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::model::Runtime;
use crate::runtime_status::RuntimeCommandSource;
use crate::shell_path::LoginShellEnv;
use crate::AppCore;

pub(crate) const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const MIN_REFRESH_VISIBLE: Duration = Duration::from_millis(400);
const SCHEDULE_INTERVAL: Duration = Duration::from_secs(15 * 60);
const OPEN_INTERVAL: Duration = Duration::from_secs(5 * 60);

pub use runner_core::protocol::usage::UsageWindow;

pub use runner_core::protocol::usage::AgentUsage;

pub use runner_core::protocol::usage::UnavailableReason;

pub use runner_core::protocol::usage::UsageSnapshot;

pub use runner_core::protocol::usage::RuntimeUsage;

pub use runner_core::protocol::usage::RefreshReason;

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
        let mut results = Vec::new();
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
                if let Some(source) = crate::runtimes::adapter(status.name).usage() {
                    let denied = self.state.lock().unwrap().keychain_denied;
                    results.push((status.name, (source.fetch)(&command, &env, denied)));
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
        for (runtime, result) in results {
            if result == Err(UnavailableReason::KeychainDenied) {
                state.keychain_denied = true;
            }
            let entry = state.snapshot.runtimes.entry(runtime).or_default();
            apply_result(&mut entry.value, &mut entry.error, result, now);
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

pub(crate) fn parse_timestamp(value: Option<&Value>) -> Option<DateTime<Utc>> {
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

pub(crate) fn percent(value: Option<&Value>) -> Option<f64> {
    value?
        .as_f64()
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(0., 100.))
}

pub(crate) fn http_client(
    env: &LoginShellEnv,
) -> Result<reqwest::blocking::Client, reqwest::Error> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtimes::{
        antigravity::usage::parse_antigravity,
        claude_code::usage::{
            claude_response, keychain_account, keychain_error_reason, parse_claude,
            parse_credentials, read_claude_credentials_file,
        },
        codex::usage::parse_codex,
    };
    #[cfg(unix)]
    use crate::runtimes::{
        claude_code::usage::read_claude_credentials_with, codex::usage::fetch_codex_with_timeout,
    };
    use chrono::TimeDelta;

    #[test]
    fn http_client_accepts_captured_socks_proxy() {
        let mut env = LoginShellEnv::default();
        env.vars
            .insert("ALL_PROXY".into(), "socks5h://127.0.0.1:1080".into());
        http_client(&env).expect("a captured SOCKS proxy should build without a network request");
    }

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
    fn antigravity_remaining_fraction_becomes_used_percent() {
        let output = serde_json::json!({"status":"SUCCESS","command":{"name":"usage","data":{"groups":[
            {"name":"Gemini Models","buckets":[{"window":"weekly","remaining_fraction":0.75,"reset_time":"2026-10-05T11:56:44Z"},{"window":"5h","remaining_fraction":0.25,"reset_time":"2026-09-28T16:56:44Z"}]},
            {"name":"Claude and GPT models","buckets":[{"window":"5h","remaining_fraction":1.0}]}
        ]}}});
        let windows = parse_antigravity(&output).unwrap();
        assert_eq!(windows[0].name, "Gemini Models · Week used");
        assert_eq!(windows[0].used_percent, 25.);
        assert_eq!(windows[1].used_percent, 75.);
        assert_eq!(windows[2].used_percent, 0.);
        assert!(windows[0].resets_at.is_some());
        let mut invalid = output.clone();
        invalid["command"]["data"]["groups"][0]["buckets"][0]["remaining_fraction"] = 2.0.into();
        invalid["command"]["data"]["groups"][0]["buckets"][1]["remaining_fraction"] = (-1.0).into();
        invalid["command"]["data"]["groups"][1]["buckets"] = serde_json::json!([]);
        assert_eq!(
            parse_antigravity(&invalid),
            Err(UnavailableReason::InvalidResponse)
        );
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

#[cfg(target_os = "macos")]
#[test]
fn usage_catalog_golden() {
    let home = tempfile::tempdir().unwrap();
    let env = LoginShellEnv {
        path: Some("/golden/bin".into()),
        vars: std::collections::BTreeMap::from([
            ("GOLDEN_ENV".into(), "value".into()),
            ("HTTPS_PROXY".into(), "http://127.0.0.1:9999".into()),
        ]),
    };
    let mut rows = Vec::new();
    for runtime in Runtime::ALL {
        let commands = crate::golden::capture_commands(|| {
            if let Some(source) = crate::runtimes::adapter(runtime).usage() {
                let _ = (source.fetch)("/golden/agent", &env, false);
            }
        });
        rows.push(serde_json::json!({"runtime":runtime,"supported":crate::runtimes::adapter(runtime).usage().is_some(),"commands":commands}));
    }
    let value = crate::golden::normalize(serde_json::json!(rows), home.path());
    // macOS credentials are a Keychain command; the account comes from the process user.
    #[cfg(target_os = "macos")]
    let value = {
        let mut value = value;
        value[1]["commands"][0]["args"][4] = serde_json::json!("<ACCOUNT>");
        value
    };
    crate::golden::assert_golden(
        if cfg!(target_os = "macos") {
            "catalog-usage-macos"
        } else {
            "catalog-usage-file"
        },
        value,
    );
}
