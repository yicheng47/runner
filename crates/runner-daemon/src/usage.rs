use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::db::UsageLkg;
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
    claude_accounts: std::collections::BTreeMap<String, RuntimeUsage>,
    claude_denied: std::collections::HashSet<String>,
    active_claude: Option<String>,
    account_refresh_pending: bool,
    lkg_seq: u64,
}

impl UsageState {
    fn show_claude_account(&mut self, id: &str) {
        let value = self.claude_accounts.get(id).cloned().unwrap_or_default();
        self.snapshot.runtimes.insert(Runtime::ClaudeCode, value);
    }

    /// Claude Code's snapshot entry only shows the selected account's cached
    /// usage, so the account cache is what is saved.
    fn lkg(&self) -> UsageLkg {
        UsageLkg {
            runtimes: self
                .snapshot
                .runtimes
                .iter()
                .filter(|(runtime, _)| **runtime != Runtime::ClaudeCode)
                .filter_map(|(runtime, usage)| Some((*runtime, usage.value.clone()?)))
                .collect(),
            claude_accounts: self
                .claude_accounts
                .iter()
                .filter_map(|(id, usage)| Some((id.clone(), usage.value.clone()?)))
                .collect(),
        }
    }
}

pub struct UsageService {
    state: Mutex<UsageState>,
    saved_seq: Mutex<u64>,
    wake_scheduler: Condvar,
    enabled: RwLock<Vec<Runtime>>,
    polling_enabled: std::sync::atomic::AtomicBool,
    clients: std::sync::atomic::AtomicUsize,
    transport_lifetime: std::sync::atomic::AtomicBool,
    discovery_started: std::sync::atomic::AtomicBool,
    isolated: std::sync::atomic::AtomicBool,
    fetch: bool,
}

impl Default for UsageService {
    fn default() -> Self {
        Self {
            state: Mutex::new(UsageState::default()),
            saved_seq: Mutex::new(0),
            wake_scheduler: Condvar::new(),
            enabled: RwLock::new(Vec::new()),
            polling_enabled: std::sync::atomic::AtomicBool::new(true),
            clients: std::sync::atomic::AtomicUsize::new(0),
            transport_lifetime: std::sync::atomic::AtomicBool::new(false),
            discovery_started: std::sync::atomic::AtomicBool::new(false),
            isolated: std::sync::atomic::AtomicBool::new(false),
            fetch: true,
        }
    }
}

impl UsageService {
    /// For in-process test cores. A fetch runs the installed agent CLIs on its
    /// own thread, whose events a deterministic UI test can neither await nor
    /// order.
    pub fn without_fetch() -> Self {
        Self {
            fetch: false,
            ..Self::default()
        }
    }
    pub fn manage_client_lifetime(&self, isolated: bool) {
        self.transport_lifetime
            .store(true, std::sync::atomic::Ordering::Release);
        self.isolated
            .store(isolated, std::sync::atomic::Ordering::Release);
        self.set_polling_enabled(false);
    }
    pub fn client_connected(self: &Arc<Self>, core: AppCore) {
        use std::sync::atomic::Ordering;
        if !self.transport_lifetime.load(Ordering::Acquire) {
            return;
        }
        if self.clients.fetch_add(1, Ordering::AcqRel) != 0 {
            return;
        }
        log::info!("runnerd client connected: UI work resumed");
        if self.isolated.load(Ordering::Acquire) {
            return;
        }
        self.set_polling_enabled(true);
        if crate::claude_accounts::load(&core.db).is_ok_and(|state| state.accounts.len() > 1) {
            crate::claude_accounts::refresh(&core);
        }
        if !self.discovery_started.swap(true, Ordering::AcqRel) {
            crate::runtime_status::start_background_discovery(
                core.events.clone(),
                core.db.clone(),
                core.runtime_shell_env.clone(),
                core.runtime_discovery.clone(),
                false,
                self.enabled(),
            );
        } else {
            let _ = crate::runtime_status::refresh_background_discovery(
                core.events.clone(),
                core.db.clone(),
                core.runtime_shell_env.clone(),
                core.runtime_discovery.clone(),
                self.enabled(),
            );
        }
    }
    pub fn client_disconnected(&self) {
        use std::sync::atomic::Ordering;
        if self.transport_lifetime.load(Ordering::Acquire)
            && self.clients.fetch_sub(1, Ordering::AcqRel) == 1
        {
            log::info!("runnerd last client disconnected: UI work paused");
            self.set_polling_enabled(false);
        }
    }
    pub fn set_polling_enabled(&self, enabled: bool) {
        self.polling_enabled
            .store(enabled, std::sync::atomic::Ordering::Release);
        self.wake_scheduler.notify_all();
    }

    pub fn snapshot(&self) -> UsageSnapshot {
        self.state.lock().unwrap().snapshot.clone()
    }

    pub fn claude_account_usage(&self) -> std::collections::BTreeMap<String, RuntimeUsage> {
        self.state.lock().unwrap().claude_accounts.clone()
    }

    pub fn select_claude_account(&self, id: &str, multiple: bool) {
        let mut state = self.state.lock().unwrap();
        state.active_claude = Some(id.to_owned());
        if !multiple && state.claude_denied.contains(id) {
            state.keychain_denied = true;
        }
        state.show_claude_account(id);
    }

    pub fn restore(&self, pool: &crate::db::DbPool) {
        let lkg = match crate::db::usage_lkg(pool) {
            Ok(Some(lkg)) => lkg,
            Ok(None) => return,
            Err(error) => {
                log::warn!("saved agent usage unreadable, starting empty: {error}");
                return;
            }
        };
        let active = crate::claude_accounts::load(pool)
            .ok()
            .map(|state| state.active_id);
        self.restore_lkg(lkg, active.as_deref(), Utc::now());
    }

    fn restore_lkg(&self, lkg: UsageLkg, active_claude: Option<&str>, now: DateTime<Utc>) {
        let restored = |usage| RuntimeUsage {
            value: Some(usage),
            error: None,
        };
        let mut state = self.state.lock().unwrap();
        for (runtime, usage) in lkg.runtimes {
            if let Some(usage) = unexpired(usage, now) {
                state.snapshot.runtimes.insert(runtime, restored(usage));
            }
        }
        for (id, usage) in lkg.claude_accounts {
            if let Some(usage) = unexpired(usage, now) {
                state.claude_accounts.insert(id, restored(usage));
            }
        }
        if let Some(id) = active_claude {
            state.show_claude_account(id);
        }
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

    pub(crate) fn request_account_refresh(self: &Arc<Self>, core: AppCore) {
        let mut state = self.state.lock().unwrap();
        if state.snapshot.refreshing {
            state.account_refresh_pending = true;
            return;
        }
        drop(state);
        self.request_refresh(core, RefreshReason::Button);
    }

    pub fn request_refresh(self: &Arc<Self>, core: AppCore, reason: RefreshReason) {
        if !self.fetch || !self.begin_refresh(reason, Utc::now()) {
            return;
        }
        core.events.emit("usage/updated", &());
        let service = Arc::clone(self);
        thread::spawn(move || {
            service.fetch(&core);
            core.events.emit("usage/updated", &());
            let pending =
                std::mem::take(&mut service.state.lock().unwrap().account_refresh_pending);
            if pending {
                service.request_refresh(core, RefreshReason::Button);
            }
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
            while !service
                .polling_enabled
                .load(std::sync::atomic::Ordering::Acquire)
            {
                let state = service.state.lock().unwrap();
                drop(
                    service
                        .wake_scheduler
                        .wait_timeout(state, Duration::from_secs(1))
                        .unwrap(),
                );
            }
            if crate::claude_accounts::load(&core.db).is_ok_and(|state| state.accounts.len() > 1) {
                crate::claude_accounts::refresh(&core);
            }
            service.request_refresh(core.clone(), RefreshReason::Launch);
            let mut state = service.state.lock().unwrap();
            loop {
                if !service
                    .polling_enabled
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    state = service
                        .wake_scheduler
                        .wait_timeout(state, Duration::from_secs(1))
                        .unwrap()
                        .0;
                    continue;
                }
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
        let account_state = crate::claude_accounts::load(&core.db).ok();
        let accounts = account_state
            .as_ref()
            .filter(|state| state.accounts.len() > 1);
        let mut account_results = Vec::new();
        if accounts.is_some() {
            crate::claude_accounts::refresh_for_usage(core);
        }
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
                if status.name == Runtime::ClaudeCode {
                    if let Some(accounts) = accounts {
                        let statuses = crate::claude_accounts::snapshot(core).ok();
                        for account in &accounts.accounts {
                            if !statuses
                                .as_ref()
                                .and_then(|snapshot| snapshot.statuses.get(&account.id))
                                .is_some_and(|status| status.logged_in)
                            {
                                continue;
                            }
                            let state = self.state.lock().unwrap();
                            let denied = state.claude_denied.contains(&account.id)
                                || (account.native && state.keychain_denied);
                            drop(state);
                            account_results.push((
                                account.id.clone(),
                                crate::runtimes::claude_code::usage::fetch_account(
                                    &env, account, denied,
                                ),
                            ));
                        }
                        continue;
                    }
                }
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
        let active = account_state.as_ref().map(|state| state.active_id.as_str());
        if let Some((seq, lkg)) = self.finish_fetch(results, account_results, active, Utc::now()) {
            self.save_lkg(&core.db, seq, &lkg);
        }
    }

    /// Writes outside the state lock; a write overtaken by a newer fetch's
    /// write is dropped, so the saved usage never goes back in time.
    fn save_lkg(&self, pool: &crate::db::DbPool, seq: u64, lkg: &UsageLkg) {
        let mut saved = self.saved_seq.lock().unwrap();
        if seq <= *saved {
            return;
        }
        match crate::db::set_usage_lkg(pool, lkg) {
            Ok(()) => *saved = seq,
            Err(error) => log::warn!("saving agent usage failed: {error}"),
        }
    }

    fn finish_fetch(
        &self,
        results: Vec<(Runtime, Result<Vec<UsageWindow>, UnavailableReason>)>,
        account_results: Vec<(String, Result<Vec<UsageWindow>, UnavailableReason>)>,
        fetched_claude: Option<&str>,
        now: DateTime<Utc>,
    ) -> Option<(u64, UsageLkg)> {
        let good = results.iter().any(|(_, result)| result.is_ok())
            || account_results.iter().any(|(_, result)| result.is_ok());
        let mut state = self.state.lock().unwrap();
        for (runtime, result) in results {
            if result == Err(UnavailableReason::KeychainDenied) {
                state.keychain_denied = true;
            }
            let entry = match fetched_claude {
                Some(id) if runtime == Runtime::ClaudeCode => {
                    state.claude_accounts.entry(id.to_owned()).or_default()
                }
                _ => state.snapshot.runtimes.entry(runtime).or_default(),
            };
            apply_result(&mut entry.value, &mut entry.error, result, now);
        }
        for (id, result) in account_results {
            if result == Err(UnavailableReason::KeychainDenied) {
                state.claude_denied.insert(id.clone());
            }
            let entry = state.claude_accounts.entry(id).or_default();
            apply_result(&mut entry.value, &mut entry.error, result, now);
        }
        if let Some(id) = state
            .active_claude
            .clone()
            .or_else(|| fetched_claude.map(str::to_owned))
        {
            state.show_claude_account(&id);
        }
        state.snapshot.last_fetch_at = Some(now);
        state.snapshot.refreshing = false;
        state.refresh_started_at = None;
        self.wake_scheduler.notify_one();
        good.then(|| {
            state.lkg_seq += 1;
            (state.lkg_seq, state.lkg())
        })
    }
}

fn unexpired(mut usage: AgentUsage, now: DateTime<Utc>) -> Option<AgentUsage> {
    usage
        .windows
        .retain(|window| window.resets_at.is_none_or(|resets_at| resets_at > now));
    (!usage.windows.is_empty()).then_some(usage)
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

#[cfg(test)]
mod account_cache_tests {
    use super::*;

    #[test]
    fn renaming_the_single_account_preserves_its_existing_usage() {
        let mut core = crate::test_support::test_core();
        core.usage = Arc::new(UsageService::without_fetch());
        let usage = RuntimeUsage {
            value: Some(AgentUsage {
                windows: vec![UsageWindow {
                    name: "Week".into(),
                    used_percent: 31.,
                    resets_at: None,
                }],
                updated_at: Utc::now(),
            }),
            error: None,
        };
        core.usage
            .state
            .lock()
            .unwrap()
            .snapshot
            .runtimes
            .insert(Runtime::ClaudeCode, usage);
        crate::claude_accounts::action(
            &core,
            runner_core::protocol::ClaudeAccountAction::Rename {
                id: "native".into(),
                name: "Home".into(),
            },
        )
        .unwrap();
        assert_eq!(
            core.usage
                .snapshot()
                .get(Runtime::ClaudeCode)
                .unwrap()
                .windows[0]
                .used_percent,
            31.
        );
    }

    #[test]
    fn selecting_an_account_uses_only_its_cached_usage_and_errors() {
        let service = UsageService::without_fetch();
        let usage = |percent| RuntimeUsage {
            value: Some(AgentUsage {
                windows: vec![UsageWindow {
                    name: "Week".into(),
                    used_percent: percent,
                    resets_at: None,
                }],
                updated_at: Utc::now(),
            }),
            error: None,
        };
        {
            let mut state = service.state.lock().unwrap();
            state.claude_accounts.insert("native".into(), usage(82.));
            state.claude_accounts.insert("work".into(), usage(24.));
            state.claude_denied.insert("work".into());
        }
        service.select_claude_account("work", true);
        assert_eq!(
            service.snapshot().get(Runtime::ClaudeCode).unwrap().windows[0].used_percent,
            24.
        );
        service.select_claude_account("native", true);
        assert_eq!(
            service.snapshot().get(Runtime::ClaudeCode).unwrap().windows[0].used_percent,
            82.
        );
        assert!(!service
            .state
            .lock()
            .unwrap()
            .claude_denied
            .contains("native"));
        service.select_claude_account("unknown", true);
        assert_eq!(service.snapshot().get(Runtime::ClaudeCode), None);
        assert_eq!(service.claude_account_usage().len(), 2);
        service.select_claude_account("work", false);
        assert!(service.state.lock().unwrap().keychain_denied);
    }
}

#[cfg(test)]
mod lkg_tests {
    use super::*;
    use chrono::TimeDelta;

    fn window(name: &str, percent: f64, resets_at: Option<DateTime<Utc>>) -> UsageWindow {
        UsageWindow {
            name: name.into(),
            used_percent: percent,
            resets_at,
        }
    }

    fn usage(windows: Vec<UsageWindow>, updated_at: DateTime<Utc>) -> AgentUsage {
        AgentUsage {
            windows,
            updated_at,
        }
    }

    #[test]
    fn restore_reads_saved_usage_and_drops_expired_windows_and_empty_agents() {
        let core = crate::test_support::test_core();
        let now = Utc::now();
        let updated = now - TimeDelta::minutes(40);
        let lkg = UsageLkg {
            runtimes: [(
                Runtime::Codex,
                usage(
                    vec![
                        window("5 hours", 90., Some(now - TimeDelta::minutes(1))),
                        window("Week", 27., Some(now + TimeDelta::days(3))),
                        window("Credits", 5., None),
                    ],
                    updated,
                ),
            )]
            .into(),
            claude_accounts: [(
                "native".into(),
                usage(
                    vec![window("Week", 70., Some(now - TimeDelta::hours(1)))],
                    updated,
                ),
            )]
            .into(),
        };
        crate::db::set_usage_lkg(&core.db, &lkg).unwrap();
        let service = UsageService::without_fetch();
        service.restore(&core.db);
        let snapshot = service.snapshot();
        let codex = snapshot.get(Runtime::Codex).unwrap();
        assert_eq!(codex.updated_at, updated);
        assert_eq!(
            codex
                .windows
                .iter()
                .map(|window| window.name.as_str())
                .collect::<Vec<_>>(),
            ["Week", "Credits"]
        );
        assert_eq!(snapshot.get(Runtime::ClaudeCode), None);
        assert_eq!(snapshot.last_fetch_at, None);
        assert!(service.state.lock().unwrap().last_attempt_at.is_none());
    }

    #[test]
    fn unreadable_saved_usage_starts_empty() {
        let core = crate::test_support::test_core();
        crate::db::app_state_set(&core.db.get().unwrap(), "usage_lkg", "{not json").unwrap();
        let service = UsageService::without_fetch();
        service.restore(&core.db);
        assert!(service.snapshot().runtimes.is_empty());
    }

    fn week(percent: f64, updated_at: DateTime<Utc>) -> AgentUsage {
        usage(vec![window("Week", percent, None)], updated_at)
    }

    fn claude_percent(service: &UsageService) -> Option<f64> {
        Some(service.snapshot().get(Runtime::ClaudeCode)?.windows[0].used_percent)
    }

    fn save_accounts(core: &AppCore, ids: &[&str], active: &str) {
        let accounts: Vec<_> = ids
            .iter()
            .map(|id| serde_json::json!({"id": id, "name": id, "folder": format!("/tmp/{id}"), "native": *id == "native"}))
            .collect();
        let state =
            serde_json::json!({"accounts": accounts, "main_id": "native", "active_id": active});
        crate::db::app_state_set(
            &core.db.get().unwrap(),
            "claude_accounts",
            &state.to_string(),
        )
        .unwrap();
    }

    fn fetch_and_save(
        core: &AppCore,
        service: &UsageService,
        results: Vec<(Runtime, Result<Vec<UsageWindow>, UnavailableReason>)>,
        account_results: Vec<(String, Result<Vec<UsageWindow>, UnavailableReason>)>,
        now: DateTime<Utc>,
    ) {
        let accounts = crate::claude_accounts::load(&core.db).unwrap();
        let (seq, lkg) = service
            .finish_fetch(results, account_results, Some(&accounts.active_id), now)
            .unwrap();
        service.save_lkg(&core.db, seq, &lkg);
    }

    fn restarted(core: &AppCore) -> UsageService {
        let service = UsageService::without_fetch();
        service.restore(&core.db);
        service
    }

    #[test]
    fn failed_first_fetch_keeps_restored_numbers() {
        let core = crate::test_support::test_core();
        let now = Utc::now();
        let restored_at = now - TimeDelta::minutes(30);
        let windows = |percent| vec![window("Week", percent, None)];
        fetch_and_save(
            &core,
            &UsageService::without_fetch(),
            vec![
                (Runtime::Codex, Ok(windows(27.))),
                (Runtime::ClaudeCode, Ok(windows(55.))),
            ],
            Vec::new(),
            restored_at,
        );
        let service = restarted(&core);
        assert_eq!(claude_percent(&service), Some(55.));
        assert_eq!(
            service.finish_fetch(
                vec![
                    (Runtime::Codex, Err(UnavailableReason::CodexNoAnswer)),
                    (
                        Runtime::ClaudeCode,
                        Err(UnavailableReason::ClaudeUnreachable)
                    ),
                ],
                Vec::new(),
                Some("native"),
                now,
            ),
            None
        );
        let snapshot = service.snapshot();
        assert_eq!(
            snapshot.get(Runtime::Codex).unwrap().updated_at,
            restored_at
        );
        assert_eq!(snapshot.error(Runtime::Codex), None);
        assert_eq!(snapshot.error(Runtime::ClaudeCode), None);

        fetch_and_save(
            &core,
            &service,
            vec![
                (Runtime::Codex, Ok(windows(31.))),
                (
                    Runtime::ClaudeCode,
                    Err(UnavailableReason::ClaudeUnreachable),
                ),
            ],
            Vec::new(),
            now,
        );
        let lkg = crate::db::usage_lkg(&core.db).unwrap().unwrap();
        assert_eq!(lkg.runtimes[&Runtime::Codex], week(31., now));
        assert_eq!(lkg.claude_accounts["native"], week(55., restored_at));
    }

    #[test]
    fn restored_account_usage_follows_the_active_account() {
        let core = crate::test_support::test_core();
        let now = Utc::now();
        save_accounts(&core, &["native", "work"], "work");
        let windows = |percent| vec![window("Week", percent, None)];
        fetch_and_save(
            &core,
            &UsageService::without_fetch(),
            Vec::new(),
            vec![
                ("native".into(), Ok(windows(82.))),
                ("work".into(), Ok(windows(24.))),
            ],
            now,
        );
        let service = restarted(&core);
        assert_eq!(claude_percent(&service), Some(24.));
        assert_eq!(service.claude_account_usage().len(), 2);
        service.finish_fetch(
            Vec::new(),
            vec![("work".into(), Err(UnavailableReason::ClaudeUnreachable))],
            Some("work"),
            now,
        );
        assert_eq!(claude_percent(&service), Some(24.));
        service.select_claude_account("native", true);
        assert_eq!(claude_percent(&service), Some(82.));
    }

    #[test]
    fn removed_account_usage_is_not_restored_for_the_remaining_account() {
        let core = crate::test_support::test_core();
        let now = Utc::now();
        let earlier = now - TimeDelta::minutes(20);
        save_accounts(&core, &["native", "work"], "work");
        let service = UsageService::without_fetch();
        let resets = Some(now + TimeDelta::minutes(5));
        fetch_and_save(
            &core,
            &service,
            Vec::new(),
            vec![
                ("native".into(), Ok(vec![window("5 hours", 82., resets)])),
                ("work".into(), Ok(vec![window("5 hours", 24., None)])),
            ],
            earlier,
        );
        service.select_claude_account("native", true);
        save_accounts(&core, &["native"], "native");
        service.select_claude_account("native", false);

        assert_eq!(claude_percent(&restarted(&core)), Some(82.));
        assert_eq!(
            claude_percent(&{
                let service = UsageService::without_fetch();
                service.restore_lkg(
                    crate::db::usage_lkg(&core.db).unwrap().unwrap(),
                    Some("native"),
                    now + TimeDelta::minutes(6),
                );
                service
            }),
            None
        );

        fetch_and_save(
            &core,
            &service,
            vec![(Runtime::ClaudeCode, Ok(vec![window("Week", 50., None)]))],
            Vec::new(),
            now,
        );
        assert_eq!(claude_percent(&restarted(&core)), Some(50.));
    }

    #[test]
    fn single_account_results_update_that_accounts_cache() {
        let core = crate::test_support::test_core();
        let now = Utc::now();
        let earlier = now - TimeDelta::minutes(20);
        let windows = |percent| vec![window("Week", percent, None)];
        fetch_and_save(
            &core,
            &UsageService::without_fetch(),
            vec![(Runtime::ClaudeCode, Ok(windows(82.)))],
            Vec::new(),
            earlier - TimeDelta::minutes(20),
        );
        let service = restarted(&core);
        assert_eq!(claude_percent(&service), Some(82.));
        fetch_and_save(
            &core,
            &service,
            vec![(Runtime::ClaudeCode, Ok(windows(50.)))],
            Vec::new(),
            earlier,
        );
        save_accounts(&core, &["native", "work"], "native");
        service.select_claude_account("native", true);
        assert_eq!(claude_percent(&service), Some(50.));
        fetch_and_save(
            &core,
            &service,
            vec![(Runtime::Codex, Ok(windows(10.)))],
            vec![
                ("native".into(), Err(UnavailableReason::ClaudeUnreachable)),
                ("work".into(), Ok(windows(24.))),
            ],
            now,
        );
        let restored = restarted(&core);
        assert_eq!(claude_percent(&restored), Some(50.));
        assert_eq!(
            restored.claude_account_usage()["native"].value,
            Some(week(50., earlier))
        );
    }

    #[test]
    fn a_result_finishing_after_an_account_switch_stays_with_its_account() {
        let service = UsageService::without_fetch();
        let now = Utc::now();
        let windows = |percent| vec![window("Week", percent, None)];
        service.select_claude_account("work", true);
        let (_, lkg) = service
            .finish_fetch(
                vec![(Runtime::ClaudeCode, Ok(windows(50.)))],
                Vec::new(),
                Some("native"),
                now,
            )
            .unwrap();
        assert_eq!(claude_percent(&service), None);
        assert_eq!(lkg.claude_accounts.keys().collect::<Vec<_>>(), ["native"]);
        assert!(!lkg.runtimes.contains_key(&Runtime::ClaudeCode));
        service.finish_fetch(
            Vec::new(),
            vec![("work".into(), Err(UnavailableReason::ClaudeUnreachable))],
            Some("native"),
            now,
        );
        assert_eq!(claude_percent(&service), None);
        assert_eq!(
            service.snapshot().error(Runtime::ClaudeCode),
            Some(UnavailableReason::ClaudeUnreachable)
        );
        service.select_claude_account("native", true);
        assert_eq!(claude_percent(&service), Some(50.));
    }

    #[test]
    fn an_overtaken_save_does_not_replace_newer_usage() {
        let core = crate::test_support::test_core();
        let service = UsageService::without_fetch();
        let now = Utc::now();
        let fetch = |percent, at| {
            service
                .finish_fetch(
                    vec![(Runtime::Codex, Ok(vec![window("Week", percent, None)]))],
                    Vec::new(),
                    Some("native"),
                    at,
                )
                .unwrap()
        };
        let (first_seq, first) = fetch(20., now - TimeDelta::minutes(1));
        let (second_seq, second) = fetch(30., now);
        service.save_lkg(&core.db, second_seq, &second);
        service.save_lkg(&core.db, first_seq, &first);
        assert_eq!(
            crate::db::usage_lkg(&core.db).unwrap().unwrap().runtimes[&Runtime::Codex],
            week(30., now)
        );
    }
}
