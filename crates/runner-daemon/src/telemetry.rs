use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use runner_core::protocol::telemetry::{
    HeartbeatMeta, HeartbeatPayload, TelemetryStatus, DEFAULT_HEARTBEAT_ENDPOINT,
};
use runner_core::remote::RemoteState;
use runner_core::runtime::Runtime;
use runner_core::version;
use rusqlite::{Connection, Transaction, TransactionBehavior};
use uuid::Uuid;

use crate::db::{app_state_delete, app_state_get, app_state_set};
use crate::error::Result;
use crate::AppCore;

pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

const TELEMETRY_ENABLED_KEY: &str = "telemetry_enabled";
const TELEMETRY_INSTALL_ID_KEY: &str = "telemetry_install_id";
const TELEMETRY_LAST_SENT_AT_KEY: &str = "telemetry_last_sent_at";
const EXAMPLE_INSTALL_ID: &str = "8f3c2d10-4b7a-4e21-9c83-6a1f9e04a91e";
const DISCOVERY_WAIT_LIMIT: Duration = Duration::from_secs(5);

const ALLOWED_RUNTIMES: &[&str] = &[
    "claude-code",
    "codex",
    "antigravity",
    "pi",
    "copilot",
    "cursor",
    "opencode",
    "grok",
    "trae",
];

#[derive(Debug, Clone)]
pub enum TelemetryEnvSource {
    Process,
    Fixed(BTreeMap<String, String>),
}

impl TelemetryEnvSource {
    fn get(&self, key: &str) -> Option<String> {
        match self {
            Self::Process => std::env::var(key).ok(),
            Self::Fixed(vars) => vars.get(key).cloned(),
        }
    }

    fn override_var(&self) -> Option<String> {
        if let Some(value) = self.get("DO_NOT_TRACK") {
            let trimmed = value.trim();
            if trimmed == "1" || trimmed.eq_ignore_ascii_case("true") {
                return Some(format!("DO_NOT_TRACK={trimmed}"));
            }
        }
        if let Some(value) = self.get("RUNNER_TELEMETRY") {
            let trimmed = value.trim();
            if trimmed == "0" || trimmed.eq_ignore_ascii_case("false") {
                return Some(format!("RUNNER_TELEMETRY={trimmed}"));
            }
        }
        None
    }

    fn test_url(&self) -> Option<String> {
        if !cfg!(any(test, debug_assertions)) {
            return None;
        }
        let value = self.get("RUNNER_HEARTBEAT_TEST_URL")?;
        let trimmed = value.trim();
        let parsed = reqwest::Url::parse(trimmed).ok()?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return None;
        }
        let host = parsed.host_str()?;
        let is_loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if !is_loopback {
            return None;
        }
        Some(parsed.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub dev_build: bool,
    pub endpoint: String,
    pub version: String,
    pub build_stamp: String,
    pub channel: String,
    pub os: String,
    pub arch: String,
    pub timeout: Duration,
    pub interval: Duration,
    pub env: TelemetryEnvSource,
    pub network_enabled: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            dev_build: version::is_dev_build(),
            endpoint: DEFAULT_HEARTBEAT_ENDPOINT.to_owned(),
            version: version::CRATE_VERSION.to_owned(),
            build_stamp: version::BUILD_STAMP
                .filter(|s| !s.is_empty())
                .unwrap_or("dev")
                .to_owned(),
            channel: version::heartbeat_channel(version::RELEASE_CHANNEL).to_owned(),
            os: version::heartbeat_os().to_owned(),
            arch: version::heartbeat_arch().to_owned(),
            timeout: HEARTBEAT_TIMEOUT,
            interval: HEARTBEAT_INTERVAL,
            env: TelemetryEnvSource::Process,
            network_enabled: true,
        }
    }
}

#[derive(Default)]
struct SchedulerState {
    last_attempt_at: Option<DateTime<Utc>>,
    launch_generation: u64,
    failed_generation: Option<u64>,
    connected_app_clients: u32,
    stopped: bool,
    wake_seq: u64,
    #[cfg(test)]
    cycles: usize,
    #[cfg(test)]
    in_cycle: bool,
    #[cfg(test)]
    exited: bool,
}

impl SchedulerState {
    fn is_failed_for_current_launch(&self) -> bool {
        self.failed_generation == Some(self.launch_generation)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendOutcome {
    Sent,
    SkippedDisabled,
    SkippedEnvOverride,
    SkippedDevBuild,
    SkippedOnboardingIncomplete,
    SkippedThrottled,
    SkippedFailedThisLaunch,
    SkippedNetworkDisabled,
    FailedSilently,
}

pub struct TelemetryService {
    config: TelemetryConfig,
    /// Serializes local SQLite transitions, status snapshots, and worker dispatch
    /// admission so consent toggles and install_id writes are atomic without ever
    /// blocking on outbound network I/O.
    local_op: Mutex<()>,
    state: Mutex<SchedulerState>,
    wake: Condvar,
    scheduler_started: AtomicBool,
}

impl Default for TelemetryService {
    fn default() -> Self {
        Self::with_config(TelemetryConfig::default())
    }
}

impl TelemetryService {
    pub fn with_config(config: TelemetryConfig) -> Self {
        Self {
            config,
            local_op: Mutex::new(()),
            state: Mutex::new(SchedulerState::default()),
            wake: Condvar::new(),
            scheduler_started: AtomicBool::new(false),
        }
    }

    pub fn without_network() -> Self {
        Self::with_config(TelemetryConfig {
            dev_build: true,
            env: TelemetryEnvSource::Fixed(BTreeMap::new()),
            network_enabled: false,
            ..TelemetryConfig::default()
        })
    }

    fn effective_endpoint(&self) -> String {
        self.config
            .env
            .test_url()
            .unwrap_or_else(|| self.config.endpoint.clone())
    }

    fn send_target(&self) -> Option<String> {
        if !self.config.network_enabled {
            return None;
        }
        if self.config.env.override_var().is_some() {
            return None;
        }
        if let Some(test_url) = self.config.env.test_url() {
            return Some(test_url);
        }
        if self.config.dev_build {
            return None;
        }
        Some(self.config.endpoint.clone())
    }

    pub fn status(&self, core: &AppCore) -> Result<TelemetryStatus> {
        self.status_at(core, Utc::now())
    }

    pub fn status_at(&self, core: &AppCore, now: DateTime<Utc>) -> Result<TelemetryStatus> {
        let _op = self.local_op.lock().unwrap();
        let mut conn = core.db.get()?;
        self.read_status_locked(core, &mut conn, now)
    }

    fn read_status_locked(
        &self,
        core: &AppCore,
        conn: &mut Connection,
        now: DateTime<Utc>,
    ) -> Result<TelemetryStatus> {
        let env_override = self.config.env.override_var();
        let stored_enabled = read_stored_enabled(conn)?;
        let enabled = stored_enabled && env_override.is_none();
        let install_id = if enabled {
            if let Some(existing) = read_valid_install_id(conn)? {
                Some(existing)
            } else {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let still_enabled = read_stored_enabled(&tx)?;
                let id = if still_enabled {
                    Some(get_or_create_install_id_tx(&tx)?)
                } else {
                    None
                };
                tx.commit()?;
                id
            }
        } else {
            None
        };
        let last_sent_at = app_state_get(conn, TELEMETRY_LAST_SENT_AT_KEY)?;
        let meta = if enabled {
            compute_meta(conn, now)
        } else {
            HeartbeatMeta::default()
        };

        let payload = self.build_payload(
            install_id.as_deref().unwrap_or(EXAMPLE_INSTALL_ID),
            is_remote_enabled(core),
            meta,
        );
        let payload_preview = serde_json::to_string_pretty(&payload)?;
        Ok(TelemetryStatus {
            enabled: enabled && install_id.is_some(),
            stored_enabled,
            env_override,
            dev_build: self.config.dev_build,
            install_id,
            payload_preview,
            endpoint: self.effective_endpoint(),
            last_sent_at,
        })
    }

    pub fn set_enabled(&self, core: &AppCore, enabled: bool) -> Result<TelemetryStatus> {
        let env_override = self.config.env.override_var();
        let status = {
            let _op = self.local_op.lock().unwrap();
            let mut conn = core.db.get()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            app_state_set(
                &tx,
                TELEMETRY_ENABLED_KEY,
                if enabled { "true" } else { "false" },
            )?;
            if !enabled {
                app_state_delete(&tx, TELEMETRY_INSTALL_ID_KEY)?;
            } else if env_override.is_none() {
                let _ = get_or_create_install_id_tx(&tx)?;
            }
            tx.commit()?;
            self.read_status_locked(core, &mut conn, Utc::now())?
        };
        core.events.emit("telemetry/changed", &());
        if enabled && env_override.is_none() {
            self.wake_scheduler();
        }
        Ok(status)
    }

    pub fn reset_id(&self, core: &AppCore) -> Result<TelemetryStatus> {
        let env_override = self.config.env.override_var();
        let (rotated, status) = {
            let _op = self.local_op.lock().unwrap();
            let mut conn = core.db.get()?;
            let rotated = if env_override.is_none() {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let changed = if read_stored_enabled(&tx)? {
                    let current = app_state_get(&tx, TELEMETRY_INSTALL_ID_KEY)?;
                    let mut next = generate_install_id();
                    while current.as_deref() == Some(next.as_str()) {
                        next = generate_install_id();
                    }
                    app_state_set(&tx, TELEMETRY_INSTALL_ID_KEY, &next)?;
                    true
                } else {
                    false
                };
                tx.commit()?;
                changed
            } else {
                false
            };
            let status = self.read_status_locked(core, &mut conn, Utc::now())?;
            (rotated, status)
        };
        if rotated {
            core.events.emit("telemetry/changed", &());
        }
        Ok(status)
    }

    pub fn notify_onboarding_completed(&self) {
        self.wake_scheduler();
    }

    pub fn app_client_connected(&self) {
        let mut state = self.state.lock().unwrap();
        let was_zero = state.connected_app_clients == 0;
        state.connected_app_clients = state.connected_app_clients.saturating_add(1);
        if was_zero {
            // Every 0 -> 1 desktop-client transition starts a new desktop launch
            // generation (covering both headless daemon startup and Keep-running
            // relaunches). Late failures from an earlier generation cannot latch
            // this launch, while SQLite `telemetry_last_sent_at` preserves the 24h
            // post-2xx throttle.
            state.launch_generation = state.launch_generation.wrapping_add(1);
            state.last_attempt_at = None;
            state.wake_seq = state.wake_seq.wrapping_add(1);
            self.wake.notify_all();
        }
    }

    pub fn app_client_disconnected(&self) {
        let mut state = self.state.lock().unwrap();
        state.connected_app_clients = state.connected_app_clients.saturating_sub(1);
    }

    pub fn stop(&self) {
        let mut state = self.state.lock().unwrap();
        state.stopped = true;
        state.wake_seq = state.wake_seq.wrapping_add(1);
        self.wake.notify_all();
    }

    #[cfg(test)]
    pub(crate) fn wait_for_cycles(&self, target: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap();
        while state.cycles < target {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            let (guard, _) = self.wake.wait_timeout(state, remaining).unwrap();
            state = guard;
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn wait_until_in_cycle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap();
        while !state.in_cycle {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            let (guard, _) = self.wake.wait_timeout(state, remaining).unwrap();
            state = guard;
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn wait_for_exit(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap();
        while !state.exited {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            let (guard, _) = self.wake.wait_timeout(state, remaining).unwrap();
            state = guard;
        }
        true
    }

    fn wake_scheduler(&self) {
        let mut state = self.state.lock().unwrap();
        state.wake_seq = state.wake_seq.wrapping_add(1);
        self.wake.notify_all();
    }

    pub fn start_scheduler(self: &Arc<Self>, core: AppCore) {
        if !self.config.network_enabled {
            return;
        }
        if self.scheduler_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let service = Arc::clone(self);
        thread::Builder::new()
            .name("runner-telemetry".into())
            .spawn(move || {
                service.run_scheduler(core);
            })
            .ok();
    }

    fn mark_scheduler_exited(&self, _state: &mut SchedulerState) {
        #[cfg(test)]
        {
            _state.exited = true;
            self.wake.notify_all();
        }
    }

    fn mark_scheduler_in_cycle(&self, _state: &mut SchedulerState, _in_cycle: bool) {
        #[cfg(test)]
        {
            _state.in_cycle = _in_cycle;
            if !_in_cycle {
                _state.cycles += 1;
            }
            self.wake.notify_all();
        }
    }

    fn run_scheduler(&self, core: AppCore) {
        // Wait briefly for initial login-shell discovery so proxy settings are available,
        // bounded so discovery stalls never block telemetry or shutdown.
        let discovery_deadline = Instant::now() + DISCOVERY_WAIT_LIMIT;
        while Instant::now() < discovery_deadline {
            {
                let mut state = self.state.lock().unwrap();
                if state.stopped {
                    self.mark_scheduler_exited(&mut state);
                    return;
                }
            }
            let checking = core
                .runtime_discovery
                .read()
                .is_ok_and(|discovery| discovery.checking);
            if !checking {
                break;
            }
            let mut state = self.state.lock().unwrap();
            if state.stopped {
                self.mark_scheduler_exited(&mut state);
                return;
            }
            let _ = self.wake.wait_timeout(state, Duration::from_millis(100));
        }

        loop {
            let cycle_seq = {
                let mut state = self.state.lock().unwrap();
                if state.stopped {
                    self.mark_scheduler_exited(&mut state);
                    return;
                }
                self.mark_scheduler_in_cycle(&mut state, true);
                state.wake_seq
            };

            let now = Utc::now();
            let outcome = self.try_send_once(&core, now);

            let mut state = self.state.lock().unwrap();
            self.mark_scheduler_in_cycle(&mut state, false);
            if state.stopped {
                self.mark_scheduler_exited(&mut state);
                return;
            }
            if state.is_failed_for_current_launch() {
                // Silent failure contract: no retry inside the same desktop launch;
                // a new 0 -> 1 desktop connection advances launch_generation.
                while !state.stopped && state.is_failed_for_current_launch() {
                    state = self.wake.wait(state).unwrap();
                }
                if state.stopped {
                    self.mark_scheduler_exited(&mut state);
                    return;
                }
                continue;
            }
            if state.wake_seq != cycle_seq {
                continue;
            }
            let wait = self.next_wait(&core, &state, outcome, Utc::now());
            let (guard, _) = self.wake.wait_timeout(state, wait).unwrap();
            state = guard;
            if state.stopped {
                self.mark_scheduler_exited(&mut state);
                return;
            }
        }
    }

    fn next_wait(
        &self,
        core: &AppCore,
        state: &SchedulerState,
        outcome: SendOutcome,
        now: DateTime<Utc>,
    ) -> Duration {
        let min_wait = Duration::from_secs(60);
        match outcome {
            SendOutcome::SkippedNetworkDisabled
            | SendOutcome::SkippedEnvOverride
            | SendOutcome::SkippedDevBuild
            | SendOutcome::SkippedDisabled
            | SendOutcome::SkippedOnboardingIncomplete
            | SendOutcome::SkippedFailedThisLaunch
            | SendOutcome::FailedSilently => self.config.interval,
            SendOutcome::Sent | SendOutcome::SkippedThrottled => {
                let last_db = core
                    .db
                    .get()
                    .ok()
                    .and_then(|conn| read_last_sent_at(&conn).ok().flatten());
                let anchor = match (last_db, state.last_attempt_at) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (Some(a), None) => Some(a),
                    (None, Some(b)) => Some(b),
                    (None, None) => None,
                };
                match anchor {
                    Some(last) => {
                        let next_at =
                            last + TimeDelta::seconds(self.config.interval.as_secs() as i64);
                        next_at
                            .signed_duration_since(now)
                            .to_std()
                            .unwrap_or(Duration::ZERO)
                            .max(min_wait)
                    }
                    None => self.config.interval,
                }
            }
        }
    }

    pub fn try_send_once(&self, core: &AppCore, now: DateTime<Utc>) -> SendOutcome {
        if !self.config.network_enabled {
            return SendOutcome::SkippedNetworkDisabled;
        }
        if self.config.env.override_var().is_some() {
            return SendOutcome::SkippedEnvOverride;
        }
        let Some(endpoint) = self.send_target() else {
            return SendOutcome::SkippedDevBuild;
        };

        let (payload, attempt_generation) = {
            let _op = self.local_op.lock().unwrap();
            let attempt_generation = {
                let state = self.state.lock().unwrap();
                if state.stopped {
                    return SendOutcome::SkippedNetworkDisabled;
                }
                state.launch_generation
            };
            let Ok(mut conn) = core.db.get() else {
                self.latch_failure(attempt_generation);
                return SendOutcome::FailedSilently;
            };
            if crate::db::onboarding_completed_at_conn(&conn)
                .ok()
                .flatten()
                .is_none()
            {
                return SendOutcome::SkippedOnboardingIncomplete;
            }
            if !read_stored_enabled(&conn).unwrap_or(false) {
                return SendOutcome::SkippedDisabled;
            }
            let last_sent_at = read_last_sent_at(&conn).ok().flatten();
            if !is_due(last_sent_at, now, self.config.interval) {
                return SendOutcome::SkippedThrottled;
            }
            {
                let state = self.state.lock().unwrap();
                if state.stopped {
                    return SendOutcome::SkippedNetworkDisabled;
                }
                if state.is_failed_for_current_launch() {
                    return SendOutcome::SkippedFailedThisLaunch;
                }
                if !is_due(state.last_attempt_at, now, self.config.interval) {
                    return SendOutcome::SkippedThrottled;
                }
            }

            let install_id = match conn.transaction_with_behavior(TransactionBehavior::Immediate) {
                Ok(tx) => {
                    if !read_stored_enabled(&tx).unwrap_or(false) {
                        return SendOutcome::SkippedDisabled;
                    }
                    let Ok(id) = get_or_create_install_id_tx(&tx) else {
                        self.latch_failure(attempt_generation);
                        return SendOutcome::FailedSilently;
                    };
                    if tx.commit().is_err() {
                        self.latch_failure(attempt_generation);
                        return SendOutcome::FailedSilently;
                    }
                    id
                }
                Err(_) => {
                    self.latch_failure(attempt_generation);
                    return SendOutcome::FailedSilently;
                }
            };
            {
                let mut state = self.state.lock().unwrap();
                if state.stopped {
                    return SendOutcome::SkippedNetworkDisabled;
                }
                if state.launch_generation == attempt_generation {
                    state.last_attempt_at = Some(now);
                }
            }
            let meta = compute_meta(&conn, now);
            (
                self.build_payload(&install_id, is_remote_enabled(core), meta),
                attempt_generation,
            )
        };

        let shell_env = core
            .runtime_shell_env
            .read()
            .map(|env| env.clone())
            .unwrap_or_default();
        let client = match crate::usage::http_client_no_redirect(&shell_env, self.config.timeout) {
            Ok(client) => client,
            Err(error) => {
                log::debug!("telemetry http client init failed: {error}");
                self.latch_failure(attempt_generation);
                return SendOutcome::FailedSilently;
            }
        };

        match client.post(&endpoint).json(&payload).send() {
            Ok(response) if response.status().is_success() => {
                let _op = self.local_op.lock().unwrap();
                if let Ok(mut conn) = core.db.get() {
                    if let Ok(tx) = conn.transaction_with_behavior(TransactionBehavior::Immediate) {
                        if read_stored_enabled(&tx).unwrap_or(false) {
                            if let Err(error) =
                                app_state_set(&tx, TELEMETRY_LAST_SENT_AT_KEY, &now.to_rfc3339())
                            {
                                log::debug!("telemetry last_sent_at persist failed: {error}");
                            }
                        }
                        let _ = tx.commit();
                    }
                }
                log::debug!("telemetry heartbeat sent: status={}", response.status());
                SendOutcome::Sent
            }
            Ok(response) => {
                log::debug!(
                    "telemetry heartbeat rejected silently: status={}",
                    response.status()
                );
                self.latch_failure(attempt_generation);
                SendOutcome::FailedSilently
            }
            Err(error) => {
                log::debug!("telemetry heartbeat failed silently: {error}");
                self.latch_failure(attempt_generation);
                SendOutcome::FailedSilently
            }
        }
    }

    fn latch_failure(&self, attempt_generation: u64) {
        let mut state = self.state.lock().unwrap();
        if state.launch_generation == attempt_generation {
            state.failed_generation = Some(attempt_generation);
        }
    }

    fn build_payload(
        &self,
        install_id: &str,
        remote_enabled: bool,
        meta: HeartbeatMeta,
    ) -> HeartbeatPayload {
        HeartbeatPayload {
            install_id: install_id.to_owned(),
            version: self.config.version.clone(),
            build_stamp: self.config.build_stamp.clone(),
            channel: self.config.channel.clone(),
            os: self.config.os.clone(),
            arch: self.config.arch.clone(),
            remote_enabled,
            meta,
        }
    }
}

fn is_remote_enabled(core: &AppCore) -> bool {
    matches!(
        core.remote.status().state,
        RemoteState::Connecting { .. }
            | RemoteState::Connected { .. }
            | RemoteState::Reconnecting { .. }
    )
}

fn read_stored_enabled(conn: &Connection) -> Result<bool> {
    Ok(match app_state_get(conn, TELEMETRY_ENABLED_KEY)? {
        Some(value) => !value.trim().eq_ignore_ascii_case("false") && value.trim() != "0",
        None => true,
    })
}

fn read_last_sent_at(conn: &Connection) -> Result<Option<DateTime<Utc>>> {
    let Some(raw) = app_state_get(conn, TELEMETRY_LAST_SENT_AT_KEY)? else {
        return Ok(None);
    };
    Ok(raw.parse::<DateTime<Utc>>().ok())
}

fn is_valid_install_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().all(|byte| !byte.is_ascii_uppercase())
        && Uuid::parse_str(id).is_ok_and(|uuid| uuid.get_version_num() == 4)
}

fn generate_install_id() -> String {
    Uuid::new_v4().hyphenated().to_string()
}

fn read_valid_install_id(conn: &Connection) -> Result<Option<String>> {
    Ok(app_state_get(conn, TELEMETRY_INSTALL_ID_KEY)?.filter(|id| is_valid_install_id(id)))
}

fn get_or_create_install_id_tx(tx: &Transaction<'_>) -> Result<String> {
    if let Some(existing) = read_valid_install_id(tx)? {
        return Ok(existing);
    }
    let id = generate_install_id();
    app_state_set(tx, TELEMETRY_INSTALL_ID_KEY, &id)?;
    Ok(id)
}

fn is_due(last: Option<DateTime<Utc>>, now: DateTime<Utc>, interval: Duration) -> bool {
    last.is_none_or(|last| {
        now.signed_duration_since(last)
            .to_std()
            .is_ok_and(|age| age >= interval)
    })
}

pub(crate) fn compute_meta(conn: &Connection, now: DateTime<Utc>) -> HeartbeatMeta {
    let cutoff = (now - TimeDelta::days(7)).to_rfc3339();
    let mut found = HashSet::new();

    if let Ok(mut stmt) = conn.prepare(
        "SELECT DISTINCT COALESCE(s.agent_runtime, r.runtime)
           FROM sessions s
           LEFT JOIN roles r ON r.id = s.role_id
          WHERE COALESCE(s.agent_runtime, r.runtime) IN (
                  'claude-code', 'codex', 'antigravity', 'pi',
                  'copilot', 'cursor', 'opencode', 'grok', 'trae'
                )
            AND s.started_at IS NOT NULL
            AND (
                  julianday(s.started_at) >= julianday(?1)
               OR (s.stopped_at IS NOT NULL AND julianday(s.stopped_at) >= julianday(?1))
               OR (s.status = 'running' AND s.archived_at IS NULL)
                )",
    ) {
        if let Ok(rows) = stmt.query_map([cutoff.as_str()], |row| row.get::<_, String>(0)) {
            for runtime in rows.flatten() {
                if ALLOWED_RUNTIMES.contains(&runtime.as_str()) {
                    found.insert(runtime);
                }
            }
        }
    }

    let active_runtimes = Runtime::SETUP_ORDER
        .into_iter()
        .map(Runtime::key)
        .filter(|key| ALLOWED_RUNTIMES.contains(key) && found.contains(*key))
        .map(str::to_owned)
        .take(10)
        .collect();

    let used_mission_7d = conn
        .query_row(
            "SELECT EXISTS(
               SELECT 1
                 FROM missions m
                 JOIN sessions s ON s.mission_id = m.id
                 LEFT JOIN roles r ON r.id = s.role_id
                WHERE COALESCE(s.agent_runtime, r.runtime) IS NOT NULL
                  AND COALESCE(s.agent_runtime, r.runtime) != 'shell'
                  AND s.started_at IS NOT NULL
                  AND (
                        julianday(s.started_at) >= julianday(?1)
                     OR (s.stopped_at IS NOT NULL AND julianday(s.stopped_at) >= julianday(?1))
                     OR (s.status = 'running' AND s.archived_at IS NULL)
                      )
                GROUP BY m.id
               HAVING COUNT(DISTINCT COALESCE(s.slot_id, s.id)) >= 2
             )",
            [cutoff.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map(|exists| exists != 0)
        .unwrap_or(false);

    HeartbeatMeta {
        active_runtimes,
        used_mission_7d,
    }
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
