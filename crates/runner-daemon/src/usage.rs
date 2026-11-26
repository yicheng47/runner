use std::collections::{BTreeMap, HashMap, HashSet};
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
const OPEN_INTERVAL: Duration = Duration::from_secs(5 * 60);

pub use runner_core::protocol::usage::UsageWindow;

pub use runner_core::protocol::usage::{AgentUsage, UsageCredits};

#[derive(Debug, PartialEq)]
pub struct UsageReading {
    pub windows: Vec<UsageWindow>,
    pub credits: Option<UsageCredits>,
}

impl From<Vec<UsageWindow>> for UsageReading {
    fn from(windows: Vec<UsageWindow>) -> Self {
        Self {
            windows,
            credits: None,
        }
    }
}

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
    runtime_accounts: HashMap<Runtime, BTreeMap<String, RuntimeUsage>>,
    denied_accounts: HashMap<Runtime, HashSet<String>>,
    active_accounts: HashMap<Runtime, String>,
    account_refresh_pending: bool,
    lkg_seq: u64,
}

impl UsageState {
    fn show_account(&mut self, runtime: Runtime, id: &str) {
        let value = self
            .runtime_accounts
            .get(&runtime)
            .and_then(|accounts| accounts.get(id))
            .cloned()
            .unwrap_or_default();
        self.snapshot.runtimes.insert(runtime, value);
    }

    /// Account-backed runtimes only show the selected account's cached usage in
    /// the main snapshot, so their per-account caches are saved separately.
    fn lkg(&self) -> UsageLkg {
        let mut lkg = UsageLkg {
            runtimes: self
                .snapshot
                .runtimes
                .iter()
                .filter(|(runtime, _)| crate::runtimes::adapter(**runtime).accounts().is_none())
                .filter_map(|(runtime, usage)| Some((*runtime, usage.value.clone()?)))
                .collect(),
            ..UsageLkg::default()
        };
        for runtime in Runtime::ALL {
            let Some(accounts) = crate::runtimes::adapter(runtime).accounts() else {
                continue;
            };
            let saved = self
                .runtime_accounts
                .get(&runtime)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|(id, usage)| Some((id.clone(), usage.value.clone()?)))
                        .collect()
                })
                .unwrap_or_default();
            accounts.save_usage_lkg(&mut lkg, saved);
        }
        lkg
    }
}

pub struct UsageService {
    state: Mutex<UsageState>,
    saved_seq: Mutex<u64>,
    interval: std::sync::atomic::AtomicU64,
    wake_scheduler: Condvar,
    enabled: RwLock<Vec<Runtime>>,
    checked: RwLock<Option<Vec<Runtime>>>,
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
            interval: std::sync::atomic::AtomicU64::new(900),
            wake_scheduler: Condvar::new(),
            enabled: RwLock::new(Vec::new()),
            checked: RwLock::new(None),
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
        for runtime in Runtime::ALL {
            if let Some(accounts) = crate::runtimes::adapter(runtime).accounts() {
                accounts.refresh_if_multiple(&core);
            }
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

    pub fn interval(&self) -> u64 {
        self.interval.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn set_interval(&self, pool: &crate::db::DbPool, seconds: u64) -> crate::error::Result<()> {
        if ![300, 900, 1800, 3600].contains(&seconds) {
            return Err(crate::error::Error::msg("invalid usage interval"));
        }
        let conn = pool.get()?;
        crate::db::app_state_set(&conn, "usage_interval", &seconds.to_string())?;
        self.interval
            .store(seconds, std::sync::atomic::Ordering::Release);
        self.wake_scheduler.notify_all();
        Ok(())
    }

    pub fn snapshot(&self) -> UsageSnapshot {
        self.state.lock().unwrap().snapshot.clone()
    }

    pub fn account_usage(&self, runtime: Runtime) -> BTreeMap<String, RuntimeUsage> {
        self.state
            .lock()
            .unwrap()
            .runtime_accounts
            .get(&runtime)
            .cloned()
            .unwrap_or_default()
    }

    pub fn select_account(&self, runtime: Runtime, id: &str, multiple: bool) {
        let mut state = self.state.lock().unwrap();
        state.active_accounts.insert(runtime, id.to_owned());
        if !multiple
            && state
                .denied_accounts
                .get(&runtime)
                .is_some_and(|denied| denied.contains(id))
        {
            state.keychain_denied = true;
        }
        state.show_account(runtime, id);
    }

    pub fn restore(&self, pool: &crate::db::DbPool) {
        if let Ok(conn) = pool.get() {
            if let Ok(Some(value)) = crate::db::app_state_get(&conn, "usage_interval") {
                if let Ok(seconds) = value.parse::<u64>() {
                    if [300, 900, 1800, 3600].contains(&seconds) {
                        self.interval
                            .store(seconds, std::sync::atomic::Ordering::Release);
                    }
                }
            }
        }

        let lkg = match crate::db::usage_lkg(pool) {
            Ok(Some(lkg)) => lkg,
            Ok(None) => return,
            Err(error) => {
                log::warn!("saved agent usage unreadable, starting empty: {error}");
                return;
            }
        };
        let active: HashMap<Runtime, String> = Runtime::ALL
            .into_iter()
            .filter_map(|runtime| {
                let id = crate::runtimes::adapter(runtime)
                    .accounts()?
                    .active_account_id(pool)?;
                Some((runtime, id))
            })
            .collect();
        self.restore_lkg(lkg, &active, Utc::now());
    }

    fn restore_lkg(
        &self,
        lkg: UsageLkg,
        active_accounts: &HashMap<Runtime, String>,
        now: DateTime<Utc>,
    ) {
        let restored = |usage| RuntimeUsage {
            value: Some(usage),
            error: None,
        };
        let mut state = self.state.lock().unwrap();
        for (runtime, usage) in &lkg.runtimes {
            if let Some(usage) = unexpired(usage.clone(), now) {
                state.snapshot.runtimes.insert(*runtime, restored(usage));
            }
        }
        for runtime in Runtime::ALL {
            let Some(accounts) = crate::runtimes::adapter(runtime).accounts() else {
                continue;
            };
            let entries = state.runtime_accounts.entry(runtime).or_default();
            for (id, usage) in accounts.load_usage_lkg(&lkg) {
                if let Some(usage) = unexpired(usage, now) {
                    entries.insert(id, restored(usage));
                }
            }
            if let Some(id) = active_accounts.get(&runtime) {
                state.show_account(runtime, id);
            }
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

    pub fn set_checked(&self, runtimes: Vec<Runtime>) {
        let mut checked = self.checked.write().unwrap();
        if checked.as_ref() == Some(&runtimes) {
            return;
        }
        *checked = Some(runtimes);
        self.wake_scheduler.notify_all();
    }

    pub fn enabled(&self) -> Vec<Runtime> {
        self.enabled.read().unwrap().clone()
    }

    fn begin_refresh(&self, reason: RefreshReason, now: DateTime<Utc>) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.snapshot.refreshing
            || !refresh_due_with_interval(
                state.last_attempt_at,
                reason,
                now,
                Duration::from_secs(self.interval()),
            )
        {
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
        let force = matches!(reason, RefreshReason::Open | RefreshReason::Button);
        if (!force && self.fetch_runtimes(false).is_empty())
            || !self.fetch
            || !self.begin_refresh(reason, Utc::now())
        {
            return;
        }
        core.events.emit("usage/updated", &());
        let service = Arc::clone(self);
        thread::spawn(move || {
            service.fetch(&core, force);
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
            for runtime in service.fetch_runtimes(false) {
                if let Some(accounts) = crate::runtimes::adapter(runtime).accounts() {
                    accounts.refresh_if_multiple(&core);
                }
            }
            service.request_refresh(core.clone(), RefreshReason::Launch);
            let mut state = service.state.lock().unwrap();
            loop {
                if !service
                    .polling_enabled
                    .load(std::sync::atomic::Ordering::Acquire)
                    || service.fetch_runtimes(false).is_empty()
                {
                    state = service
                        .wake_scheduler
                        .wait_timeout(state, Duration::from_secs(1))
                        .unwrap()
                        .0;
                    continue;
                }
                let wait = schedule_wait_with_interval(
                    state.last_attempt_at,
                    Utc::now(),
                    Duration::from_secs(service.interval()),
                );
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

    fn fetch_runtimes(&self, force: bool) -> Vec<Runtime> {
        let mut enabled = self.enabled();
        if !force {
            if let Some(checked) = self.checked.read().unwrap().as_ref() {
                enabled.retain(|runtime| checked.contains(runtime));
            }
        }
        enabled
    }

    fn fetch(&self, core: &AppCore, force: bool) {
        let statuses = crate::ops::runtime::runtime_status_list(core);
        let env = core.runtime_shell_env.read().unwrap().clone();
        let enabled = self.fetch_runtimes(force);
        let mut results = Vec::new();
        let account_snapshots: HashMap<Runtime, Box<dyn crate::runtimes::UsageAccountsSnapshot>> =
            Runtime::ALL
                .into_iter()
                .filter_map(|runtime| {
                    let snapshot = crate::runtimes::adapter(runtime)
                        .accounts()?
                        .usage_snapshot(&core.db)?;
                    Some((runtime, snapshot))
                })
                .collect();
        let mut account_results = Vec::new();
        for &runtime in &enabled {
            if let Some(snapshot) = account_snapshots.get(&runtime) {
                snapshot.prepare_refresh(core);
            }
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
                if let Some(snapshot) = account_snapshots.get(&status.name) {
                    let is_denied = |id: &str, native: bool| {
                        let state = self.state.lock().unwrap();
                        state
                            .denied_accounts
                            .get(&status.name)
                            .is_some_and(|denied| denied.contains(id))
                            || (native && state.keychain_denied)
                    };
                    if let Some(fetched) = snapshot.fetch_usage(core, &env, &is_denied) {
                        account_results.extend(
                            fetched
                                .into_iter()
                                .map(|(id, result)| (status.name, id, result)),
                        );
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
        let active_accounts: HashMap<Runtime, String> = account_snapshots
            .iter()
            .map(|(&runtime, snapshot)| (runtime, snapshot.active_account_id().to_owned()))
            .collect();
        if let Some((seq, lkg)) =
            self.finish_fetch(results, account_results, &active_accounts, Utc::now())
        {
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
        results: Vec<(Runtime, Result<UsageReading, UnavailableReason>)>,
        account_results: Vec<(Runtime, String, Result<UsageReading, UnavailableReason>)>,
        fetched_accounts: &HashMap<Runtime, String>,
        now: DateTime<Utc>,
    ) -> Option<(u64, UsageLkg)> {
        let good = results.iter().any(|(_, result)| result.is_ok())
            || account_results.iter().any(|(_, _, result)| result.is_ok());
        let mut state = self.state.lock().unwrap();
        for (runtime, result) in results {
            if result == Err(UnavailableReason::KeychainDenied) {
                state.keychain_denied = true;
            }
            let entry = match fetched_accounts.get(&runtime) {
                Some(id) => state
                    .runtime_accounts
                    .entry(runtime)
                    .or_default()
                    .entry(id.clone())
                    .or_default(),
                None => state.snapshot.runtimes.entry(runtime).or_default(),
            };
            apply_result(&mut entry.value, &mut entry.error, result, now);
        }
        for (runtime, id, result) in account_results {
            if result == Err(UnavailableReason::KeychainDenied) {
                state
                    .denied_accounts
                    .entry(runtime)
                    .or_default()
                    .insert(id.clone());
            }
            let entry = state
                .runtime_accounts
                .entry(runtime)
                .or_default()
                .entry(id)
                .or_default();
            apply_result(&mut entry.value, &mut entry.error, result, now);
        }
        let mut show_runtimes: Vec<Runtime> = state.active_accounts.keys().copied().collect();
        for &runtime in fetched_accounts.keys() {
            if !show_runtimes.contains(&runtime) {
                show_runtimes.push(runtime);
            }
        }
        for runtime in show_runtimes {
            if let Some(id) = state
                .active_accounts
                .get(&runtime)
                .cloned()
                .or_else(|| fetched_accounts.get(&runtime).cloned())
            {
                state.show_account(runtime, &id);
            }
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
    (!usage.windows.is_empty() || usage.credits.is_some()).then_some(usage)
}

fn refresh_visible_remaining(started: Instant, now: Instant) -> Duration {
    MIN_REFRESH_VISIBLE.saturating_sub(now.saturating_duration_since(started))
}

fn schedule_wait_with_interval(
    last: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    interval: Duration,
) -> Duration {
    last.and_then(|last| {
        (last + chrono::TimeDelta::seconds(interval.as_secs() as i64) - now)
            .to_std()
            .ok()
    })
    .unwrap_or(Duration::ZERO)
}

fn refresh_due_with_interval(
    last: Option<DateTime<Utc>>,
    reason: RefreshReason,
    now: DateTime<Utc>,
    scheduled: Duration,
) -> bool {
    let interval = match reason {
        RefreshReason::Button => return true,
        RefreshReason::Launch => Duration::ZERO,
        RefreshReason::Schedule => scheduled,
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
    result: Result<UsageReading, UnavailableReason>,
    now: DateTime<Utc>,
) {
    match result {
        Ok(reading) => {
            *current = Some(AgentUsage {
                windows: reading.windows,
                credits: reading.credits,
                updated_at: now,
            });
            *error = None;
        }
        Err(reason) => *error = Some(reason),
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
#[path = "usage_tests.rs"]
mod tests;

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
                credits: None,
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
        crate::runtimes::claude_code::accounts::action(
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
                credits: None,
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
            let accounts = state
                .runtime_accounts
                .entry(Runtime::ClaudeCode)
                .or_default();
            accounts.insert("native".into(), usage(82.));
            accounts.insert("work".into(), usage(24.));
            state
                .denied_accounts
                .entry(Runtime::ClaudeCode)
                .or_default()
                .insert("work".into());
        }
        service.select_account(Runtime::ClaudeCode, "work", true);
        assert_eq!(
            service.snapshot().get(Runtime::ClaudeCode).unwrap().windows[0].used_percent,
            24.
        );
        service.select_account(Runtime::ClaudeCode, "native", true);
        assert_eq!(
            service.snapshot().get(Runtime::ClaudeCode).unwrap().windows[0].used_percent,
            82.
        );
        assert!(!service
            .state
            .lock()
            .unwrap()
            .denied_accounts
            .get(&Runtime::ClaudeCode)
            .is_some_and(|denied| denied.contains("native")));
        service.select_account(Runtime::ClaudeCode, "unknown", true);
        assert_eq!(service.snapshot().get(Runtime::ClaudeCode), None);
        assert_eq!(service.account_usage(Runtime::ClaudeCode).len(), 2);
        service.select_account(Runtime::ClaudeCode, "work", false);
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
            credits: None,
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
    fn credit_cache_survives_expired_windows_failure_and_successful_replacement() {
        let core = crate::test_support::test_core();
        let service = UsageService::without_fetch();
        let now = Utc::now();
        let original = now - TimeDelta::minutes(30);
        let credits = UsageCredits {
            balance: Some("1250.750012345678901234".into()),
            has_credits: true,
            unlimited: false,
        };
        let (seq, lkg) = service
            .finish_fetch(
                vec![(
                    Runtime::Codex,
                    Ok(UsageReading {
                        windows: vec![window("Week", 27., Some(now - TimeDelta::minutes(1)))],
                        credits: Some(credits.clone()),
                    }),
                )],
                Vec::new(),
                &HashMap::new(),
                original,
            )
            .unwrap();
        service.save_lkg(&core.db, seq, &lkg);
        let service = restarted(&core);
        let cached = service.snapshot().get(Runtime::Codex).unwrap().clone();
        assert!(cached.windows.is_empty());
        assert_eq!(cached.credits, Some(credits));
        assert_eq!(cached.updated_at, original);
        assert!(service
            .finish_fetch(
                vec![(Runtime::Codex, Err(UnavailableReason::CodexNoAnswer))],
                Vec::new(),
                &HashMap::new(),
                now
            )
            .is_none());
        let failed = service.snapshot();
        assert_eq!(failed.get(Runtime::Codex), Some(&cached));
        assert_eq!(
            failed.error(Runtime::Codex),
            Some(UnavailableReason::CodexNoAnswer)
        );
        let (seq, lkg) = service
            .finish_fetch(
                vec![(Runtime::Codex, Ok(vec![window("Week", 31., None)].into()))],
                Vec::new(),
                &HashMap::new(),
                now,
            )
            .unwrap();
        service.save_lkg(&core.db, seq, &lkg);
        let recovered = service.snapshot();
        assert_eq!(recovered.get(Runtime::Codex).unwrap().credits, None);
        assert_eq!(recovered.get(Runtime::Codex).unwrap().updated_at, now);
        assert_eq!(recovered.error(Runtime::Codex), None);
        assert_eq!(
            restarted(&core).snapshot().get(Runtime::Codex),
            recovered.get(Runtime::Codex)
        );
        assert!(recovered.get(Runtime::ClaudeCode).is_none());
    }

    #[test]
    fn unreadable_saved_usage_starts_empty() {
        let core = crate::test_support::test_core();
        crate::db::app_state_set(&core.db.get().unwrap(), "usage_lkg", "{not json").unwrap();
        let service = UsageService::without_fetch();
        service.restore(&core.db);
        assert!(service.snapshot().runtimes.is_empty());
    }

    #[test]
    fn antigravity_preserves_lkg_on_error_and_drops_only_expired_pool_windows() {
        let core = crate::test_support::test_core();
        let service = UsageService::without_fetch();
        let now = Utc::now();
        let original = now - TimeDelta::minutes(20);
        let (seq, lkg) = service
            .finish_fetch(
                vec![(
                    Runtime::Antigravity,
                    Ok(vec![
                        window(
                            "Gemini Models · 5 hours used",
                            18.,
                            Some(now - TimeDelta::minutes(1)),
                        ),
                        window(
                            "Gemini Models · Week used",
                            48.,
                            Some(now + TimeDelta::days(5)),
                        ),
                        window(
                            "Claude and GPT models · Week used",
                            82.,
                            Some(now + TimeDelta::days(5)),
                        ),
                    ]
                    .into()),
                )],
                Vec::new(),
                &HashMap::new(),
                original,
            )
            .unwrap();
        service.save_lkg(&core.db, seq, &lkg);

        assert!(service
            .finish_fetch(
                vec![(Runtime::Antigravity, Err(UnavailableReason::SignIn))],
                Vec::new(),
                &HashMap::new(),
                now,
            )
            .is_none());
        let after_error = service.snapshot();
        let cached = after_error.get(Runtime::Antigravity).unwrap();
        assert_eq!(cached.windows.len(), 3);
        assert_eq!(cached.updated_at, original);
        assert_eq!(
            after_error.error(Runtime::Antigravity),
            Some(UnavailableReason::SignIn)
        );

        let restarted = restarted(&core).snapshot();
        let kept = restarted.get(Runtime::Antigravity).unwrap();
        assert_eq!(
            kept.windows
                .iter()
                .map(|w| w.name.as_str())
                .collect::<Vec<_>>(),
            [
                "Gemini Models · Week used",
                "Claude and GPT models · Week used",
            ]
        );
        assert_eq!(kept.updated_at, original);
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

    fn claude_active(id: &str) -> HashMap<Runtime, String> {
        HashMap::from([(Runtime::ClaudeCode, id.into())])
    }

    fn fetch_and_save(
        core: &AppCore,
        service: &UsageService,
        results: Vec<(Runtime, Result<Vec<UsageWindow>, UnavailableReason>)>,
        account_results: Vec<(String, Result<Vec<UsageWindow>, UnavailableReason>)>,
        now: DateTime<Utc>,
    ) {
        let active: HashMap<Runtime, String> = Runtime::ALL
            .into_iter()
            .filter_map(|runtime| {
                let id = crate::runtimes::adapter(runtime)
                    .accounts()?
                    .active_account_id(&core.db)?;
                Some((runtime, id))
            })
            .collect();
        let (seq, lkg) = service
            .finish_fetch(
                results
                    .into_iter()
                    .map(|(runtime, result)| (runtime, result.map(Into::into)))
                    .collect(),
                account_results
                    .into_iter()
                    .map(|(id, result)| (Runtime::ClaudeCode, id, result.map(Into::into)))
                    .collect(),
                &active,
                now,
            )
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
                &claude_active("native"),
                now,
            ),
            None
        );
        let snapshot = service.snapshot();
        assert_eq!(
            snapshot.get(Runtime::Codex).unwrap().updated_at,
            restored_at
        );
        assert_eq!(
            snapshot.error(Runtime::Codex),
            Some(UnavailableReason::CodexNoAnswer)
        );
        assert_eq!(
            snapshot.error(Runtime::ClaudeCode),
            Some(UnavailableReason::ClaudeUnreachable)
        );

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
        assert_eq!(service.account_usage(Runtime::ClaudeCode).len(), 2);
        service.finish_fetch(
            Vec::new(),
            vec![(
                Runtime::ClaudeCode,
                "work".into(),
                Err(UnavailableReason::ClaudeUnreachable),
            )],
            &claude_active("work"),
            now,
        );
        assert_eq!(claude_percent(&service), Some(24.));
        service.select_account(Runtime::ClaudeCode, "native", true);
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
        service.select_account(Runtime::ClaudeCode, "native", true);
        save_accounts(&core, &["native"], "native");
        service.select_account(Runtime::ClaudeCode, "native", false);

        assert_eq!(claude_percent(&restarted(&core)), Some(82.));
        assert_eq!(
            claude_percent(&{
                let service = UsageService::without_fetch();
                service.restore_lkg(
                    crate::db::usage_lkg(&core.db).unwrap().unwrap(),
                    &claude_active("native"),
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
        service.select_account(Runtime::ClaudeCode, "native", true);
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
            restored.account_usage(Runtime::ClaudeCode)["native"].value,
            Some(week(50., earlier))
        );
    }

    #[test]
    fn a_result_finishing_after_an_account_switch_stays_with_its_account() {
        let service = UsageService::without_fetch();
        let now = Utc::now();
        let windows = |percent| vec![window("Week", percent, None)];
        service.select_account(Runtime::ClaudeCode, "work", true);
        let (_, lkg) = service
            .finish_fetch(
                vec![(Runtime::ClaudeCode, Ok(windows(50.).into()))],
                Vec::new(),
                &claude_active("native"),
                now,
            )
            .unwrap();
        assert_eq!(claude_percent(&service), None);
        assert_eq!(lkg.claude_accounts.keys().collect::<Vec<_>>(), ["native"]);
        assert!(!lkg.runtimes.contains_key(&Runtime::ClaudeCode));
        service.finish_fetch(
            Vec::new(),
            vec![(
                Runtime::ClaudeCode,
                "work".into(),
                Err(UnavailableReason::ClaudeUnreachable),
            )],
            &claude_active("native"),
            now,
        );
        assert_eq!(claude_percent(&service), None);
        assert_eq!(
            service.snapshot().error(Runtime::ClaudeCode),
            Some(UnavailableReason::ClaudeUnreachable)
        );
        service.select_account(Runtime::ClaudeCode, "native", true);
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
                    vec![(
                        Runtime::Codex,
                        Ok(vec![window("Week", percent, None)].into()),
                    )],
                    Vec::new(),
                    &claude_active("native"),
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

#[test]
fn configured_interval_is_persisted_and_drives_scheduled_refresh() {
    let pool = crate::db::open_in_memory().unwrap();
    let service = UsageService::without_fetch();
    assert_eq!(service.interval(), 900);
    for seconds in [300, 900, 1800, 3600] {
        service.set_interval(&pool, seconds).unwrap();
        let restored = UsageService::without_fetch();
        restored.restore(&pool);
        assert_eq!(restored.interval(), seconds);
        let now = Utc::now();
        let interval = Duration::from_secs(seconds);
        assert_eq!(
            schedule_wait_with_interval(Some(now), now, interval),
            interval
        );
        assert!(!refresh_due_with_interval(
            Some(now),
            RefreshReason::Schedule,
            now + chrono::TimeDelta::seconds(seconds as i64 - 1),
            interval
        ));
        assert!(refresh_due_with_interval(
            Some(now),
            RefreshReason::Schedule,
            now + chrono::TimeDelta::seconds(seconds as i64),
            interval
        ));
        assert!(refresh_due_with_interval(
            Some(now),
            RefreshReason::Open,
            now + chrono::TimeDelta::minutes(6),
            interval
        ));
    }
    assert!(service.set_interval(&pool, 1).is_err());
    assert_eq!(service.interval(), 3600);
}

#[test]
fn scheduled_usage_selection_preserves_agent_discovery_and_open_loads_enabled_agents() {
    let service = Arc::new(UsageService::default());
    let agents = vec![Runtime::Codex, Runtime::ClaudeCode, Runtime::Pi];
    service.set_enabled(agents.clone());
    service.set_checked(vec![Runtime::ClaudeCode]);
    assert_eq!(service.enabled(), agents);
    assert_eq!(service.fetch_runtimes(false), [Runtime::ClaudeCode]);
    service.set_checked(Vec::new());
    assert!(service.fetch_runtimes(false).is_empty());
    assert_eq!(service.fetch_runtimes(true), agents);
    let temp = tempfile::tempdir().unwrap();
    let core = crate::test_support::test_core_in(temp.path().join("data"));
    for reason in [RefreshReason::Launch, RefreshReason::Schedule] {
        service.request_refresh(core.clone(), reason);
        let state = service.state.lock().unwrap();
        assert!(!state.snapshot.refreshing);
        assert!(state.last_attempt_at.is_none());
        assert!(state.snapshot.last_fetch_at.is_none());
    }
}
