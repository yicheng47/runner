//! Installed agent CLI versions and whether a newer one exists (#533).
//!
//! The installed version is the first semver token `<command> --version`
//! prints. "Newer" comes from either the npm registry's dist-tag (`latest` or
//! Claude Code's configured channel) or a runtime's HTTPS JSON release
//! manifest (Antigravity CLI). Every failure degrades to showing the version
//! alone.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{
    candidate_runtime_command, RuntimeCommandSource, SharedDiscoveryState, SharedShellEnv,
};
use crate::db::DbPool;
use crate::events::EventChannel;
use crate::model::Runtime;

use crate::shell_path::LoginShellEnv;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const LATEST_TTL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, Default)]
pub(crate) struct VersionDiscovery {
    installed: HashMap<Runtime, Probe>,
    /// The newest probe started per runtime (async or sync). Only that
    /// probe's generation may commit its result into `installed` or retire
    /// `probing[runtime]`.
    probing: HashMap<Runtime, ActiveProbe>,
    /// Currently running `EventChannel`-backed background workers per runtime.
    in_flight_async: HashMap<Runtime, ActiveProbe>,
    /// A `changed` result committed by an `events: None` sync probe while a
    /// matching background worker was still in flight; cleared once that worker
    /// or a newer channeled probe delivers `runtime/changed`.
    pending_async_notify: HashMap<Runtime, u64>,
    next_probe: u64,
    latest: HashMap<Runtime, Latest>,
    /// The newest remote check started per runtime: its source key (npm tag or
    /// manifest URL) and generation. A check for another source supersedes it,
    /// like an override does a probe.
    checking: HashMap<Runtime, (&'static str, u64)>,
    next_check: u64,
    /// A check asked for while discovery was still resolving executables;
    /// `true` when it was forced. It runs once discovery completes.
    deferred_latest: Option<bool>,
    /// Flag names parsed from `<command> --help`, keyed by `(command, version)`.
    help_flags: HashMap<HelpCacheKey, Option<HashSet<String>>>,
    #[cfg(test)]
    help_flags_overrides: HashMap<Runtime, Option<HashSet<String>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HelpCacheKey {
    command: String,
    version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutableFingerprint {
    modified_ns: Option<u128>,
    len: u64,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

pub(crate) fn executable_fingerprint(command: &str) -> Option<ExecutableFingerprint> {
    let metadata = std::fs::metadata(command).ok()?;
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_nanos());
    #[cfg(unix)]
    let (dev, ino) = {
        use std::os::unix::fs::MetadataExt;
        (metadata.dev(), metadata.ino())
    };
    Some(ExecutableFingerprint {
        modified_ns,
        len: metadata.len(),
        #[cfg(unix)]
        dev,
        #[cfg(unix)]
        ino,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Probe {
    command: String,
    fingerprint: Option<ExecutableFingerprint>,
    version: Option<String>,
}

impl Probe {
    fn matches(&self, command: &str, fingerprint: Option<&ExecutableFingerprint>) -> bool {
        self.command == command && self.fingerprint.as_ref() == fingerprint
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveProbe {
    command: String,
    fingerprint: Option<ExecutableFingerprint>,
    generation: u64,
}

#[derive(Debug, Clone)]
struct Latest {
    /// The upstream source key (npm dist-tag or manifest URL) this answer came from.
    tag: &'static str,
    version: String,
    fetched_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LatestSource {
    Npm {
        package: &'static str,
        tag: &'static str,
    },
    ManifestUrl(&'static str),
}

impl LatestSource {
    fn url(self) -> String {
        match self {
            Self::Npm { package, tag } => format!("https://registry.npmjs.org/{package}/{tag}"),
            Self::ManifestUrl(url) => url.to_owned(),
        }
    }

    fn cache_key(self) -> &'static str {
        match self {
            Self::Npm { tag, .. } => tag,
            Self::ManifestUrl(url) => url,
        }
    }
}

impl VersionDiscovery {
    #[cfg(test)]
    pub(crate) fn set_installed(&mut self, runtime: Runtime, command: &str, version: Option<&str>) {
        self.installed.insert(
            runtime,
            Probe {
                command: command.into(),
                fingerprint: executable_fingerprint(command),
                version: version.map(str::to_owned),
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn set_help_flags(&mut self, runtime: Runtime, flags: Option<HashSet<String>>) {
        self.help_flags_overrides.insert(runtime, flags);
    }

    /// The probed version of `command`, if the last probe ran that command.
    pub(crate) fn installed(&self, runtime: Runtime, command: &str) -> Option<&str> {
        let fingerprint = executable_fingerprint(command);
        self.installed
            .get(&runtime)
            .filter(|probe| probe.matches(command, fingerprint.as_ref()))
            .and_then(|probe| probe.version.as_deref())
    }

    #[cfg(test)]
    pub(crate) fn has_in_flight_async(&self, runtime: Runtime) -> bool {
        self.in_flight_async.contains_key(&runtime)
    }

    pub(crate) fn probed(&self, runtime: Runtime, command: &str) -> Option<Option<&str>> {
        let fingerprint = executable_fingerprint(command);
        self.installed
            .get(&runtime)
            .filter(|probe| probe.matches(command, fingerprint.as_ref()))
            .map(|probe| probe.version.as_deref())
    }

    fn commit_probe(&mut self, runtime: Runtime, probe: Probe) -> bool {
        let previous = self.installed.insert(runtime, probe.clone());
        let changed = previous.as_ref() != Some(&probe);
        if changed {
            let binary_replaced = previous.as_ref().is_some_and(|prev| {
                prev.command == probe.command && prev.fingerprint != probe.fingerprint
            });
            self.help_flags.retain(|key, _| {
                key.command != probe.command || (!binary_replaced && key.version == probe.version)
            });
        }
        changed
    }

    /// Records an already-validated override probe after persistence succeeds,
    /// superseding any older in-flight probe so a stale result cannot win later.
    pub(crate) fn record_override_probe(
        &mut self,
        runtime: Runtime,
        command: String,
        fingerprint: Option<ExecutableFingerprint>,
        version: Option<String>,
    ) -> bool {
        self.next_probe += 1;
        self.probing.remove(&runtime);
        self.in_flight_async.remove(&runtime);
        self.pending_async_notify.remove(&runtime);
        self.commit_probe(
            runtime,
            Probe {
                command,
                fingerprint,
                version,
            },
        )
    }

    fn begin_sync_probe(&mut self, runtime: Runtime, command: &str) -> Option<u64> {
        if self.probed(runtime, command).is_some() {
            return None;
        }
        let fingerprint = executable_fingerprint(command);
        self.next_probe += 1;
        let generation = self.next_probe;
        self.probing.insert(
            runtime,
            ActiveProbe {
                command: command.to_owned(),
                fingerprint,
                generation,
            },
        );
        Some(generation)
    }

    fn finish_sync_probe(
        &mut self,
        runtime: Runtime,
        generation: u64,
        command: String,
        version: Option<String>,
        has_events: bool,
    ) -> bool {
        let Some(active) = self.probing.get(&runtime) else {
            return false;
        };
        if active.generation != generation {
            return false;
        }
        let fingerprint = active.fingerprint.clone();
        self.probing.remove(&runtime);
        let probe = Probe {
            command,
            fingerprint,
            version,
        };
        let changed = self.commit_probe(runtime, probe.clone());
        if has_events {
            let pending = self.pending_async_notify.remove(&runtime).is_some();
            return changed || pending;
        }
        if let Some(async_probe) = self
            .in_flight_async
            .get(&runtime)
            .filter(|active| active.command == probe.command)
        {
            if changed {
                self.pending_async_notify
                    .insert(runtime, async_probe.generation);
            }
        }
        false
    }

    /// The npm `latest` version when it is newer than `installed`.
    pub(crate) fn available(&self, runtime: Runtime, installed: Option<&str>) -> Option<&str> {
        let latest = self.latest.get(&runtime)?.version.as_str();
        is_newer(latest, installed?).then_some(latest)
    }

    /// Starts a probe of `command` unless one for it is already running or,
    /// without `force`, already answered. Returns the probe's generation.
    /// Either way the runtime is now on `command`, so a probe still running
    /// for another command is superseded and its result will be discarded.
    fn begin_probe(&mut self, runtime: Runtime, command: &str, force: bool) -> Option<u64> {
        let fingerprint = executable_fingerprint(command);
        let running = self
            .in_flight_async
            .get(&runtime)
            .is_some_and(|probe| probe.command == command && probe.fingerprint == fingerprint);
        let answered = self
            .installed
            .get(&runtime)
            .is_some_and(|probe| probe.matches(command, fingerprint.as_ref()));
        if !force && running {
            return None;
        }
        if !force && answered {
            self.probing.remove(&runtime);
            self.in_flight_async.remove(&runtime);
            return None;
        }
        if force {
            self.help_flags.retain(|key, _| key.command != command);
        }
        self.next_probe += 1;
        let generation = self.next_probe;
        let probe = ActiveProbe {
            command: command.to_owned(),
            fingerprint,
            generation,
        };
        self.probing.insert(runtime, probe.clone());
        self.in_flight_async.insert(runtime, probe);
        if let Some(pending_gen) = self.pending_async_notify.get_mut(&runtime) {
            *pending_gen = generation;
        }
        Some(generation)
    }

    /// Stores the result of the newest probe for `runtime` and reports
    /// whether the shown version changed. A superseded probe — an override
    /// set while it ran, a later forced probe, or a newer sync probe — is
    /// discarded without evicting the newer probe's state.
    fn finish_probe(
        &mut self,
        runtime: Runtime,
        generation: u64,
        command: String,
        version: Option<String>,
    ) -> bool {
        if self
            .in_flight_async
            .get(&runtime)
            .is_some_and(|active| active.generation == generation)
        {
            self.in_flight_async.remove(&runtime);
        }
        let Some(active) = self.probing.get(&runtime) else {
            if self
                .pending_async_notify
                .get(&runtime)
                .is_some_and(|pending| *pending == generation)
            {
                self.pending_async_notify.remove(&runtime);
                return true;
            }
            return false;
        };
        if active.generation != generation {
            if self
                .pending_async_notify
                .get(&runtime)
                .is_some_and(|pending| *pending == generation)
            {
                self.pending_async_notify.remove(&runtime);
                return true;
            }
            return false;
        }
        let fingerprint = active.fingerprint.clone();
        self.probing.remove(&runtime);
        let pending = self.pending_async_notify.remove(&runtime).is_some();
        let changed = self.commit_probe(
            runtime,
            Probe {
                command,
                fingerprint,
                version,
            },
        );
        changed || pending
    }

    pub(crate) fn clear_runtime_probe(&mut self, runtime: Runtime) -> bool {
        self.next_probe += 1;
        let removed = self.installed.remove(&runtime);
        if let Some(probe) = &removed {
            self.help_flags
                .retain(|key, _| key.command != probe.command);
        }
        self.probing.remove(&runtime);
        self.in_flight_async.remove(&runtime);
        self.pending_async_notify.remove(&runtime);
        removed.is_some()
    }

    pub(super) fn clear_probes(&mut self) {
        self.next_probe += 1;
        self.installed.clear();
        self.probing.clear();
        self.in_flight_async.clear();
        self.pending_async_notify.clear();
        self.help_flags.clear();
    }

    pub(super) fn clear_help_flags(&mut self, command: Option<&str>) {
        if let Some(command) = command {
            self.help_flags.retain(|key, _| key.command != command);
        } else {
            self.help_flags.clear();
        }
    }

    /// Starts an npm check of `tag` unless one for it is already running or,
    /// without `force`, answered within the cache window. Returns the check's
    /// generation, and whether an answer from another tag was dropped: once
    /// a runtime asks for a new tag, the old tag's answer no longer shows.
    fn begin_latest(
        &mut self,
        runtime: Runtime,
        tag: &'static str,
        force: bool,
        now: Instant,
    ) -> (Option<u64>, bool) {
        let dropped = self
            .latest
            .get(&runtime)
            .is_some_and(|latest| latest.tag != tag)
            && self.latest.remove(&runtime).is_some();
        let running = self
            .checking
            .get(&runtime)
            .is_some_and(|(running, _)| *running == tag);
        let fresh = self
            .latest
            .get(&runtime)
            .is_some_and(|latest| now.saturating_duration_since(latest.fetched_at) < LATEST_TTL);
        if running || (!force && fresh) {
            return (None, dropped);
        }
        self.next_check += 1;
        self.checking.insert(runtime, (tag, self.next_check));
        (Some(self.next_check), dropped)
    }

    /// A failed check forgets the previous answer, so the row falls back to
    /// its version alone: no Update button, no dot, and no error.
    fn finish_latest(
        &mut self,
        runtime: Runtime,
        generation: u64,
        tag: &'static str,
        version: Option<String>,
        now: Instant,
    ) -> bool {
        if self
            .checking
            .get(&runtime)
            .is_none_or(|(_, newest)| *newest != generation)
        {
            return false;
        }
        self.checking.remove(&runtime);
        let Some(version) = version else {
            return self.latest.remove(&runtime).is_some();
        };
        let previous = self.latest.insert(
            runtime,
            Latest {
                tag,
                version: version.clone(),
                fetched_at: now,
            },
        );
        previous.is_none_or(|previous| previous.version != version)
    }
}

fn parse_semver(token: &str) -> Option<()> {
    runner_core::protocol::runtime_metadata::version_is_semver(token).then_some(())
}

pub(crate) fn below_minimum(installed: Option<&str>, minimum: Option<&str>) -> bool {
    runner_core::protocol::runtime_metadata::version_below(installed, minimum)
}

/// The first semver-looking token in `--version` output: `2.1.266 (Claude
/// Code)`, `codex-cli 0.153.4` and `GitHub Copilot CLI 1.0.83.` all yield
/// one. Output without one yields nothing rather than an error.
pub(crate) fn parse_version(output: &[u8]) -> Option<String> {
    String::from_utf8_lossy(output)
        .split_whitespace()
        .map(|token| token.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
        .find(|token| parse_semver(token).is_some())
        .map(|token| token.strip_prefix('v').unwrap_or(token).to_owned())
}

/// Whether `latest` is a newer release than `installed`. A prerelease of the
/// same version counts as older; two prereleases of it count as equal.
pub(crate) fn is_newer(latest: &str, installed: &str) -> bool {
    runner_core::protocol::runtime_metadata::version_is_newer(latest, installed)
}

pub(crate) fn installed_command(
    runtime: Runtime,
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
) -> Option<String> {
    candidate_runtime_command(runtime, pool, shell_env, discovery)
        .ok()
        .filter(|command| command.source != RuntimeCommandSource::Catalog)
        .map(|command| command.command)
}

pub(crate) fn probe_command_version(command: &str, env: &LoginShellEnv) -> Option<String> {
    super::models::command_output(command, &["--version"], env, PROBE_TIMEOUT)
        .and_then(|output| parse_version(&output))
}

pub(crate) fn ensure_probed(
    runtime: Runtime,
    command: &str,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    events: Option<&EventChannel>,
) -> Option<String> {
    let generation = {
        let Ok(mut state) = discovery.write() else {
            return None;
        };
        if let Some(cached) = state.versions.probed(runtime, command) {
            return cached.map(str::to_owned);
        }
        state.versions.begin_sync_probe(runtime, command)
    };
    let env = shell_env.read().map(|env| env.clone()).unwrap_or_default();
    let version = probe_command_version(command, &env);
    log::info!(
        "runtime version sync probe: runtime={runtime} version={}",
        version.as_deref().unwrap_or("unknown")
    );
    let changed = generation.is_some_and(|generation| {
        discovery.write().is_ok_and(|mut state| {
            state.versions.finish_sync_probe(
                runtime,
                generation,
                command.to_owned(),
                version.clone(),
                events.is_some(),
            )
        })
    });
    if changed {
        if let Some(events) = events {
            events.emit("runtime/changed", &());
        }
    }
    version
}

/// Probes the installed version of each runtime that resolves to an
/// executable, each on its own thread. Not-found runtimes never probe.
pub(crate) fn request_probes(
    pool: &Arc<DbPool>,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    events: &EventChannel,
    runtimes: &[Runtime],
    force: bool,
) {
    for &runtime in runtimes {
        let Some(command) = installed_command(runtime, pool, shell_env, discovery) else {
            continue;
        };
        let Some(generation) = discovery
            .write()
            .ok()
            .and_then(|mut state| state.versions.begin_probe(runtime, &command, force))
        else {
            continue;
        };
        let shell_env = Arc::clone(shell_env);
        let discovery = Arc::clone(discovery);
        let events = events.clone();
        std::thread::spawn(move || {
            run_probe(
                runtime, generation, command, &shell_env, &discovery, &events,
            );
        });
    }
}

/// Probes one runtime on the calling thread and returns its version. Runs
/// after an update exits, whose result footer names the new version.
pub(crate) fn probe_now(
    runtime: Runtime,
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    events: &EventChannel,
) -> Option<String> {
    let command = installed_command(runtime, pool, shell_env, discovery)?;
    let generation = discovery
        .write()
        .ok()
        .and_then(|mut state| state.versions.begin_probe(runtime, &command, true))?;
    run_probe(runtime, generation, command, shell_env, discovery, events)
}

pub(crate) fn starts_now(
    runtime: Runtime,
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
) -> bool {
    let Some(command) = installed_command(runtime, pool, shell_env, discovery) else {
        return false;
    };
    let env = shell_env.read().map(|env| env.clone()).unwrap_or_default();
    super::models::command_output(&command, &["--version"], &env, PROBE_TIMEOUT).is_some()
}

pub(crate) fn clear_help_flags(discovery: &SharedDiscoveryState, command: Option<&str>) {
    let Ok(mut state) = discovery.write() else {
        return;
    };
    state.versions.clear_help_flags(command);
}

pub(crate) fn probe_help_flags(
    runtime: Runtime,
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
) -> Option<HashSet<String>> {
    probe_help_flags_with(runtime, pool, shell_env, discovery, |command, env| {
        super::models::command_output_with_stderr(command, &["--help"], env, PROBE_TIMEOUT)
    })
}

pub(crate) fn probe_help_flags_with(
    runtime: Runtime,
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    fetch: impl FnOnce(&str, &LoginShellEnv) -> Option<Vec<u8>>,
) -> Option<HashSet<String>> {
    #[cfg(test)]
    if let Ok(state) = discovery.read() {
        if let Some(flags) = state.versions.help_flags_overrides.get(&runtime) {
            return flags.clone();
        }
    }
    let command = installed_command(runtime, pool, shell_env, discovery)?;
    let version = discovery.read().ok().and_then(|state| {
        state
            .versions
            .installed(runtime, &command)
            .map(str::to_owned)
    });
    let key = HelpCacheKey {
        command: command.clone(),
        version,
    };
    if let Ok(state) = discovery.read() {
        if let Some(cached) = state.versions.help_flags.get(&key) {
            return cached.clone();
        }
    }
    let env = shell_env.read().map(|env| env.clone()).unwrap_or_default();
    let flags = fetch(&command, &env)
        .and_then(|bytes| crate::runtimes::custom_flags::parse_help_flags(&bytes));
    if let Ok(mut state) = discovery.write() {
        state.versions.help_flags.insert(key, flags.clone());
    }
    flags
}

fn run_probe(
    runtime: Runtime,
    generation: u64,
    command: String,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    events: &EventChannel,
) -> Option<String> {
    let env = shell_env.read().map(|env| env.clone()).unwrap_or_default();
    let version = super::models::command_output(&command, &["--version"], &env, PROBE_TIMEOUT)
        .and_then(|output| parse_version(&output));
    log::info!(
        "runtime version probe: runtime={runtime} version={}",
        version.as_deref().unwrap_or("unknown")
    );
    let changed = discovery.write().is_ok_and(|mut state| {
        state
            .versions
            .finish_probe(runtime, generation, command, version.clone())
    });
    if changed {
        events.emit("runtime/changed", &());
    }
    version
}

/// Checks the remote version source (npm registry or HTTPS release manifest)
/// for a newer version of every installed runtime that supports update checks,
/// on background threads. Answers are cached for six hours per app run; `force`
/// (Refresh) skips the cache. While discovery is still resolving executables
/// the check waits for it, so a CLI that discovery is about to find is checked
/// too. Installed versions are re-probed on every request, even while remote
/// answers are cached.
pub(crate) fn request_latest(
    pool: &Arc<DbPool>,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    events: &EventChannel,
    force: bool,
) {
    #[cfg(test)]
    if super::is_path_isolated(shell_env) {
        return;
    }
    request_latest_with(pool, shell_env, discovery, events, force, fetch_latest);
}

/// Runs the check `request_latest` deferred during discovery, if any.
pub(crate) fn run_deferred_latest(
    pool: &Arc<DbPool>,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    events: &EventChannel,
) {
    run_deferred_latest_with(pool, shell_env, discovery, events, fetch_latest);
}

fn run_deferred_latest_with<F>(
    pool: &Arc<DbPool>,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    events: &EventChannel,
    fetch: F,
) where
    F: Fn(&LoginShellEnv, LatestSource) -> Option<String> + Clone + Send + 'static,
{
    let deferred = discovery
        .write()
        .ok()
        .and_then(|mut state| state.versions.deferred_latest.take());
    if let Some(force) = deferred {
        request_latest_with(pool, shell_env, discovery, events, force, fetch);
    }
}

fn latest_source(runtime: Runtime, pool: &DbPool) -> Option<LatestSource> {
    let adapter = crate::runtimes::adapter(runtime);
    let definition = adapter.catalog()?;
    if definition.update_args.is_empty() {
        return None;
    }
    if let Some(package) = definition.npm_package {
        let tag = if runtime == Runtime::ClaudeCode {
            crate::runtimes::claude_code::accounts::sharing::main_channel(pool)
                .unwrap_or_else(|| dist_tag(runtime))
        } else {
            dist_tag(runtime)
        };
        return Some(LatestSource::Npm { package, tag });
    }
    adapter.latest_manifest_url().map(LatestSource::ManifestUrl)
}

fn request_latest_with<F>(
    pool: &Arc<DbPool>,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    events: &EventChannel,
    force: bool,
    fetch: F,
) where
    F: Fn(&LoginShellEnv, LatestSource) -> Option<String> + Clone + Send + 'static,
{
    let deferred = discovery.write().is_ok_and(|mut state| {
        if !state.checking {
            return false;
        }
        let versions = &mut state.versions;
        versions.deferred_latest = Some(versions.deferred_latest.unwrap_or(false) || force);
        true
    });
    if deferred {
        return;
    }
    for definition in crate::runtimes::catalogs() {
        let runtime = definition.name;
        if !updatable(runtime) || installed_command(runtime, pool, shell_env, discovery).is_none() {
            continue;
        }
        let Some(source) = latest_source(runtime, pool) else {
            continue;
        };
        request_probes(pool, shell_env, discovery, events, &[runtime], true);
        let shell_env = Arc::clone(shell_env);
        let discovery = Arc::clone(discovery);
        let events = events.clone();
        let fetch = fetch.clone();
        std::thread::spawn(move || {
            let env = shell_env.read().map(|env| env.clone()).unwrap_or_default();
            check_latest(
                &discovery,
                &events,
                runtime,
                source.cache_key(),
                force,
                Instant::now(),
                || fetch(&env, source),
            );
        });
    }
}

/// One cached latest-version check with the fetch passed in, so tests stub the
/// network. Emits `runtime/changed` whenever the shown answer changes: at once
/// when an answer from another source/tag is dropped, before any network I/O,
/// and again when the new answer lands. Returns whether anything changed. A
/// check superseded by a newer one, for another source or forced, is discarded.
fn check_latest(
    discovery: &SharedDiscoveryState,
    events: &EventChannel,
    runtime: Runtime,
    tag: &'static str,
    force: bool,
    now: Instant,
    fetch: impl FnOnce() -> Option<String>,
) -> bool {
    let Ok((generation, dropped)) = discovery
        .write()
        .map(|mut state| state.versions.begin_latest(runtime, tag, force, now))
    else {
        return false;
    };
    if dropped {
        events.emit("runtime/changed", &());
    }
    let Some(generation) = generation else {
        return dropped;
    };
    let version = fetch();
    if version.is_none() {
        log::debug!("runtime latest check failed: runtime={runtime} source={tag}");
    }
    let finished = discovery.write().is_ok_and(|mut state| {
        state
            .versions
            .finish_latest(runtime, generation, tag, version, now)
    });
    if finished {
        events.emit("runtime/changed", &());
    }
    dropped || finished
}

/// The npm dist-tag a runtime's own updater follows. Claude Code follows the
/// release channel in its user settings, `autoUpdatesChannel`: `latest` by
/// default, or `stable`, which npm publishes as its own tag. The others
/// follow `latest`.
fn dist_tag(runtime: Runtime) -> &'static str {
    crate::runtimes::adapter(runtime).npm_dist_tag()
}

#[cfg(test)]
use crate::runtimes::claude_code::claude_channel;

fn fetch_latest(env: &LoginShellEnv, source: LatestSource) -> Option<String> {
    fetch_latest_from_url(env, &source.url())
}

fn fetch_latest_from_url(env: &LoginShellEnv, url: &str) -> Option<String> {
    let response = crate::usage::http_client(env).ok()?.get(url).send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    parse_latest(&response.json().ok()?)
}

fn parse_latest(body: &serde_json::Value) -> Option<String> {
    let version = body.get("version")?.as_str()?.trim();
    parse_semver(version).map(|_| version.to_owned())
}

/// Whether the runtime has an update subcommand and a latest-version source
/// Runner can check (an npm package or an HTTPS release manifest).
pub(crate) fn updatable(runtime: Runtime) -> bool {
    let adapter = crate::runtimes::adapter(runtime);
    adapter.catalog().is_some_and(|definition| {
        !definition.update_args.is_empty()
            && (definition.npm_package.is_some() || adapter.latest_manifest_url().is_some())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::RwLock;

    use crate::shell_path::DiscoveryState;

    #[test]
    fn parses_the_version_shapes_each_cli_prints() {
        for (output, expected) in [
            ("2.1.280 (Claude Code)\n", "2.1.280"),
            ("codex-cli 0.155.1\n", "0.155.1"),
            (
                "GitHub Copilot CLI 1.0.83.\nRun 'copilot update' to check for updates.\n",
                "1.0.83",
            ),
            ("0.85.1\n", "0.85.1"),
            // agy --version
            ("1.2.9\n", "1.2.9"),
            ("codex-cli 0.156.0-alpha.2\n", "0.156.0-alpha.2"),
            ("tool v1.2.3\n", "1.2.3"),
        ] {
            assert_eq!(
                parse_version(output.as_bytes()).as_deref(),
                Some(expected),
                "{output}"
            );
        }
    }

    #[test]
    fn garbage_output_has_no_version() {
        for output in [
            "",
            "command not found",
            "version 1.2",
            "1.2.x",
            "1.2.3.4",
            "01.a.3",
            "codex-cli",
            "\u{1b}[31merror\u{1b}[0m",
        ] {
            assert_eq!(parse_version(output.as_bytes()), None, "{output}");
        }
        assert_eq!(parse_version(&[0xff, 0xfe, b' ', b'1']), None);
    }

    #[test]
    fn compares_releases_and_prereleases() {
        assert!(is_newer("0.155.0", "0.153.4"));
        assert!(is_newer("1.0.0", "0.999.999"));
        assert!(is_newer("2.1.270", "2.1.266"));
        assert!(!is_newer("2.1.266", "2.1.266"));
        assert!(!is_newer("0.153.4", "0.155.0"));
        assert!(is_newer("0.156.0", "0.156.0-alpha.2"));
        assert!(!is_newer("0.156.0-alpha.3", "0.156.0-alpha.2"));
        assert!(!is_newer("0.155.1", "0.156.0-alpha.2"));
        assert!(!is_newer("not-a-version", "0.1.0"));
        assert!(!is_newer("0.2.0", "unknown"));
    }

    #[test]
    fn reads_the_version_field_of_a_registry_answer() {
        assert_eq!(
            parse_latest(&serde_json::json!({"name":"@openai/codex","version":"0.156.1"}))
                .as_deref(),
            Some("0.156.1")
        );
        for body in [
            serde_json::json!({}),
            serde_json::json!({"version": 156}),
            serde_json::json!({"version": "latest"}),
            serde_json::json!("0.156.1"),
        ] {
            assert_eq!(parse_latest(&body), None, "{body}");
        }
    }

    fn discovery() -> SharedDiscoveryState {
        Arc::new(RwLock::new(DiscoveryState::pending()))
    }

    fn available(discovery: &SharedDiscoveryState, installed: &str) -> Option<String> {
        discovery
            .read()
            .unwrap()
            .versions
            .available(Runtime::Codex, Some(installed))
            .map(str::to_owned)
    }

    #[test]
    fn latest_check_is_cached_for_six_hours_and_refresh_skips_the_cache() {
        let discovery = discovery();
        let start = Instant::now();
        let fetches = std::cell::Cell::new(0);
        let fetch = |version: &'static str| {
            fetches.set(fetches.get() + 1);
            Some(version.to_owned())
        };
        assert!(check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::Codex,
            "latest",
            false,
            start,
            || { fetch("0.155.0") }
        ));
        assert_eq!(available(&discovery, "0.153.4").as_deref(), Some("0.155.0"));
        assert_eq!(available(&discovery, "0.155.0"), None);

        let later = start + Duration::from_secs(5 * 60 * 60);
        assert!(!check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::Codex,
            "latest",
            false,
            later,
            || { fetch("0.156.0") }
        ));
        assert_eq!(fetches.get(), 1);

        assert!(check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::Codex,
            "latest",
            true,
            later,
            || { fetch("0.156.0") }
        ));
        assert_eq!(fetches.get(), 2);
        assert_eq!(available(&discovery, "0.155.0").as_deref(), Some("0.156.0"));

        let expired = later + LATEST_TTL;
        assert!(!check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::Codex,
            "latest",
            false,
            expired,
            || { fetch("0.156.0") }
        ));
        assert_eq!(fetches.get(), 3);
    }

    #[test]
    fn failed_checks_drop_the_update_and_keep_the_version() {
        let discovery = discovery();
        let now = Instant::now();
        assert!(!check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::Codex,
            "latest",
            false,
            now,
            || None
        ));
        assert_eq!(available(&discovery, "0.153.4"), None);
        assert_eq!(
            discovery
                .read()
                .unwrap()
                .versions
                .available(Runtime::Codex, None),
            None
        );

        {
            let versions = &mut discovery.write().unwrap().versions;
            let generation = versions
                .begin_probe(Runtime::Codex, "/a/codex", false)
                .unwrap();
            versions.finish_probe(
                Runtime::Codex,
                generation,
                "/a/codex".into(),
                Some("0.153.4".into()),
            );
        }
        assert!(check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::Codex,
            "latest",
            true,
            now,
            || { Some("0.155.0".into()) }
        ));
        assert_eq!(available(&discovery, "0.153.4").as_deref(), Some("0.155.0"));
        assert!(check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::Codex,
            "latest",
            true,
            now,
            || { None }
        ));
        assert_eq!(available(&discovery, "0.153.4"), None);
        let state = discovery.read().unwrap();
        assert_eq!(
            state.versions.installed(Runtime::Codex, "/a/codex"),
            Some("0.153.4")
        );
        assert!(!state.versions.checking.contains_key(&Runtime::Codex));
    }

    #[test]
    fn claude_follows_the_release_channel_in_its_settings() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        assert_eq!(claude_channel(&settings), None);
        for (body, channel) in [
            (r#"{"autoUpdatesChannel":"stable"}"#, Some("stable")),
            (
                r#"{"autoUpdatesChannel":"latest","model":"opus"}"#,
                Some("latest"),
            ),
            (r#"{"autoUpdatesChannel":"nightly"}"#, None),
            (r#"{"autoUpdatesChannel":1}"#, None),
            (r#"{"model":"opus"}"#, None),
            ("not json", None),
        ] {
            std::fs::write(&settings, body).unwrap();
            assert_eq!(claude_channel(&settings), channel, "{body}");
        }
        assert_eq!(dist_tag(Runtime::Codex), "latest");
        assert_eq!(dist_tag(Runtime::Copilot), "latest");
    }

    #[test]
    fn a_stable_channel_compares_against_the_stable_tag() {
        let discovery = discovery();
        let now = Instant::now();
        let available = |installed: &str| {
            discovery
                .read()
                .unwrap()
                .versions
                .available(Runtime::ClaudeCode, Some(installed))
                .map(str::to_owned)
        };
        assert!(check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::ClaudeCode,
            "latest",
            false,
            now,
            || Some("2.1.280".into())
        ));
        assert_eq!(available("2.1.267").as_deref(), Some("2.1.280"));

        let fetched = std::cell::Cell::new(false);
        assert!(check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::ClaudeCode,
            "stable",
            false,
            now + Duration::from_secs(60),
            || {
                fetched.set(true);
                Some("2.1.267".into())
            }
        ));
        assert!(fetched.get(), "a new channel skips the cached answer");
        assert_eq!(available("2.1.267"), None);
        assert!(!check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::ClaudeCode,
            "stable",
            false,
            now + Duration::from_secs(120),
            || panic!("the stable answer is cached")
        ));
    }

    #[test]
    fn a_channel_switch_supersedes_a_check_in_flight() {
        let discovery = discovery();
        let now = Instant::now();
        let available = |discovery: &SharedDiscoveryState, installed: &str| {
            discovery
                .read()
                .unwrap()
                .versions
                .available(Runtime::ClaudeCode, Some(installed))
                .map(str::to_owned)
        };
        assert!(check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::ClaudeCode,
            "latest",
            false,
            now,
            || Some("2.1.280".into())
        ));
        assert_eq!(available(&discovery, "2.1.267").as_deref(), Some("2.1.280"));

        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let in_flight = {
            let discovery = Arc::clone(&discovery);
            std::thread::spawn(move || {
                check_latest(
                    &discovery,
                    &EventChannel::new(),
                    Runtime::ClaudeCode,
                    "latest",
                    true,
                    now,
                    || {
                        entered_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        Some("2.1.281".into())
                    },
                )
            })
        };
        entered_rx.recv().unwrap();

        assert!(check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::ClaudeCode,
            "stable",
            false,
            now,
            || {
                assert_eq!(
                    available(&discovery, "2.1.267"),
                    None,
                    "the latest-tag answer hides once stable is asked for"
                );
                Some("2.1.267".into())
            }
        ));
        assert_eq!(available(&discovery, "2.1.267"), None);

        release_tx.send(()).unwrap();
        assert!(
            !in_flight.join().unwrap(),
            "the latest-tag check was superseded"
        );
        assert_eq!(available(&discovery, "2.1.267"), None);
        let state = discovery.read().unwrap();
        let latest = &state.versions.latest[&Runtime::ClaudeCode];
        assert_eq!((latest.tag, latest.version.as_str()), ("stable", "2.1.267"));
        assert!(state.versions.checking.is_empty());
    }

    #[test]
    fn dropping_the_old_channel_answer_notifies_before_the_fetch() {
        let discovery = discovery();
        let events = EventChannel::new();
        let now = Instant::now();
        assert!(check_latest(
            &discovery,
            &events,
            Runtime::ClaudeCode,
            "latest",
            false,
            now,
            || Some("2.1.280".into())
        ));
        let mut changes = events.subscribe();
        assert!(check_latest(
            &discovery,
            &events,
            Runtime::ClaudeCode,
            "stable",
            false,
            now,
            || {
                let early = changes.try_recv().expect("notified before the fetch");
                assert_eq!(early.name, "runtime/changed");
                assert_eq!(
                    discovery
                        .read()
                        .unwrap()
                        .versions
                        .available(Runtime::ClaudeCode, Some("2.1.267")),
                    None
                );
                None
            }
        ));
    }

    #[test]
    fn a_check_in_flight_is_not_started_twice() {
        let discovery = discovery();
        let now = Instant::now();
        assert!(discovery
            .write()
            .unwrap()
            .versions
            .begin_latest(Runtime::Codex, "latest", true, now)
            .0
            .is_some());
        assert!(!check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::Codex,
            "latest",
            true,
            now,
            || { panic!("a second fetch must not start") }
        ));
    }

    #[test]
    fn probe_results_apply_only_to_the_command_they_ran() {
        let mut versions = VersionDiscovery::default();
        let first = versions
            .begin_probe(Runtime::Codex, "/a/codex", false)
            .unwrap();
        assert_eq!(
            versions.begin_probe(Runtime::Codex, "/a/codex", false),
            None
        );
        assert!(versions.finish_probe(
            Runtime::Codex,
            first,
            "/a/codex".into(),
            Some("0.153.4".into())
        ));
        assert_eq!(
            versions.begin_probe(Runtime::Codex, "/a/codex", false),
            None
        );
        assert_eq!(
            versions.installed(Runtime::Codex, "/a/codex"),
            Some("0.153.4")
        );
        assert_eq!(versions.installed(Runtime::Codex, "/b/codex"), None);
        let forced = versions
            .begin_probe(Runtime::Codex, "/a/codex", true)
            .unwrap();
        assert!(versions.finish_probe(Runtime::Codex, forced, "/a/codex".into(), None));
        assert_eq!(versions.installed(Runtime::Codex, "/a/codex"), None);
    }

    #[test]
    fn an_override_set_during_a_probe_supersedes_it() {
        let mut versions = VersionDiscovery::default();
        let detected = versions
            .begin_probe(Runtime::Codex, "/detected/codex", true)
            .unwrap();
        let overridden = versions
            .begin_probe(Runtime::Codex, "/override/codex", false)
            .expect("a new command probes even while another one runs");
        assert!(versions.finish_probe(
            Runtime::Codex,
            overridden,
            "/override/codex".into(),
            Some("0.156.0".into())
        ));
        assert!(!versions.finish_probe(
            Runtime::Codex,
            detected,
            "/detected/codex".into(),
            Some("0.153.4".into())
        ));
        assert_eq!(
            versions.installed(Runtime::Codex, "/override/codex"),
            Some("0.156.0")
        );
        assert_eq!(versions.installed(Runtime::Codex, "/detected/codex"), None);

        let older = versions
            .begin_probe(Runtime::Codex, "/override/codex", true)
            .unwrap();
        let newer = versions
            .begin_probe(Runtime::Codex, "/override/codex", true)
            .unwrap();
        assert!(!versions.finish_probe(
            Runtime::Codex,
            older,
            "/override/codex".into(),
            Some("0.155.0".into())
        ));
        assert!(versions.finish_probe(
            Runtime::Codex,
            newer,
            "/override/codex".into(),
            Some("0.157.0".into())
        ));
        assert!(!versions.finish_probe(
            Runtime::Codex,
            older,
            "/override/codex".into(),
            Some("0.155.0".into())
        ));
        assert_eq!(
            versions.installed(Runtime::Codex, "/override/codex"),
            Some("0.157.0")
        );
    }

    #[test]
    fn switching_back_to_an_answered_command_supersedes_the_running_probe() {
        let mut versions = VersionDiscovery::default();
        let first = versions
            .begin_probe(Runtime::Codex, "/a/codex", true)
            .unwrap();
        versions.finish_probe(
            Runtime::Codex,
            first,
            "/a/codex".into(),
            Some("0.153.4".into()),
        );
        let overridden = versions
            .begin_probe(Runtime::Codex, "/b/codex", false)
            .unwrap();
        assert_eq!(
            versions.begin_probe(Runtime::Codex, "/a/codex", false),
            None
        );
        assert!(!versions.finish_probe(
            Runtime::Codex,
            overridden,
            "/b/codex".into(),
            Some("0.156.0".into())
        ));
        assert_eq!(
            versions.installed(Runtime::Codex, "/a/codex"),
            Some("0.153.4")
        );
        assert!(versions.probing.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_check_requested_during_discovery_runs_once_it_completes() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Mutex;
        let bin = tempfile::tempdir().unwrap();
        let codex = bin.path().join("codex");
        std::fs::write(&codex, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
        let pool = Arc::new(crate::db::open_in_memory().unwrap());
        let shell_env = Arc::new(RwLock::new(LoginShellEnv {
            path: Some(bin.path().display().to_string()),
            vars: std::collections::BTreeMap::from([(
                super::super::TEST_ISOLATE_PATH_ENV.into(),
                "1".into(),
            )]),
        }));
        let discovery = discovery();
        let events = EventChannel::new();
        let fetches = Arc::new(Mutex::new(Vec::new()));
        let stub_fetch = {
            let fetches = Arc::clone(&fetches);
            move |_: &LoginShellEnv, source: LatestSource| {
                fetches.lock().unwrap().push(source);
                Some("99.0.0".into())
            }
        };

        request_latest_with(
            &pool,
            &shell_env,
            &discovery,
            &events,
            false,
            stub_fetch.clone(),
        );
        request_latest_with(
            &pool,
            &shell_env,
            &discovery,
            &events,
            true,
            stub_fetch.clone(),
        );
        assert_eq!(
            discovery.read().unwrap().versions.deferred_latest,
            Some(true)
        );
        assert!(fetches.lock().unwrap().is_empty());

        discovery.write().unwrap().checking = false;
        run_deferred_latest_with(&pool, &shell_env, &discovery, &events, stub_fetch);
        let deadline = Instant::now() + Duration::from_secs(10);
        while available(&discovery, "0.153.4").is_none() {
            assert!(Instant::now() < deadline, "deferred check never ran");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(available(&discovery, "0.153.4").as_deref(), Some("99.0.0"));
        assert!(fetches.lock().unwrap().contains(&LatestSource::Npm {
            package: "@openai/codex",
            tag: "latest",
        }));
        assert_eq!(discovery.read().unwrap().versions.deferred_latest, None);
    }

    #[test]
    fn update_check_reprobes_installed_version_with_a_cached_latest() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;
        let bin = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let command = bin.path().join("codex");
        #[cfg(windows)]
        let command = bin.path().join("codex.cmd");
        #[cfg(unix)]
        let script = "#!/bin/sh\necho 'codex-cli 0.155.0'\n";
        #[cfg(windows)]
        let script = "@echo off\r\necho codex-cli 0.155.0\r\n";
        std::fs::write(&command, script).unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755)).unwrap();
        let command = command.to_str().unwrap();
        let pool = Arc::new(crate::db::open_in_memory().unwrap());
        for runtime in crate::runtimes::catalogs() {
            crate::db::set_runtime_override(&pool, runtime.name.key(), Some(command)).unwrap();
        }
        let shell_env = Arc::new(RwLock::new(crate::test_support::isolated_shell_env(Some(
            bin.path(),
        ))));
        let discovery = discovery();
        let events = EventChannel::new();
        {
            let mut state = discovery.write().unwrap();
            state.checking = false;
            let versions = &mut state.versions;
            let generation = versions.begin_probe(Runtime::Codex, command, true).unwrap();
            versions.finish_probe(
                Runtime::Codex,
                generation,
                command.into(),
                Some("0.153.4".into()),
            );
        }
        check_latest(
            &discovery,
            &events,
            Runtime::Codex,
            "latest",
            false,
            Instant::now(),
            || Some("0.155.0".into()),
        );
        assert_eq!(available(&discovery, "0.153.4").as_deref(), Some("0.155.0"));

        let mut changes = events.subscribe();
        request_latest_with(&pool, &shell_env, &discovery, &events, false, |_, _| None);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if changes.try_recv().is_ok()
                && discovery
                    .read()
                    .unwrap()
                    .versions
                    .installed(Runtime::Codex, command)
                    == Some("0.155.0")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "version refresh did not notify the UI"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let status = super::super::status_list(&pool, &shell_env, &discovery).unwrap();
        let codex = status
            .runtimes
            .iter()
            .find(|runtime| runtime.name == Runtime::Codex)
            .unwrap();
        assert_eq!(codex.installed_version.as_deref(), Some("0.155.0"));
        assert_eq!(codex.available_version, None);
    }

    #[cfg(unix)]
    #[test]
    fn probe_runs_the_version_flag_and_ignores_unparseable_output() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let command = temp.path().join("codex");
        let write = |script: &str| {
            std::fs::write(&command, script).unwrap();
            std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let shell_env = Arc::new(RwLock::new(crate::test_support::isolated_shell_env(None)));
        let discovery = discovery();
        let events = EventChannel::new();
        let command_path = command.display().to_string();
        let probe = || {
            let generation = discovery
                .write()
                .unwrap()
                .versions
                .begin_probe(Runtime::Codex, &command_path, true)
                .unwrap();
            run_probe(
                Runtime::Codex,
                generation,
                command_path.clone(),
                &shell_env,
                &discovery,
                &events,
            )
        };
        write("#!/bin/sh\n[ \"$1\" = --version ] || exit 2\necho 'codex-cli 0.153.4'\n");
        assert_eq!(probe().as_deref(), Some("0.153.4"));
        write("#!/bin/sh\necho 'no version here'\n");
        assert_eq!(probe(), None);
        assert_eq!(
            discovery
                .read()
                .unwrap()
                .versions
                .installed(Runtime::Codex, &command_path),
            None
        );
    }

    #[test]
    fn older_async_probe_never_evicts_sync_probe_or_overwrites_newer_sync_result() {
        let mut versions = VersionDiscovery::default();
        let command = "/p/opencode";

        // Async completes FIRST with None while newer sync is still active: sync stays active.
        let async_gen = versions
            .begin_probe(Runtime::OpenCode, command, false)
            .unwrap();
        let sync_gen = versions
            .begin_sync_probe(Runtime::OpenCode, command)
            .unwrap();
        assert!(!versions.finish_probe(Runtime::OpenCode, async_gen, command.into(), None));
        assert!(!versions.has_in_flight_async(Runtime::OpenCode));
        assert!(versions.probed(Runtime::OpenCode, command).is_none());
        assert!(!versions.probing.is_empty());

        assert!(versions.finish_sync_probe(
            Runtime::OpenCode,
            sync_gen,
            command.into(),
            Some("1.19.5".into()),
            true,
        ));
        assert_eq!(
            versions.installed(Runtime::OpenCode, command),
            Some("1.19.5")
        );
        assert!(versions.probing.is_empty());

        // Reverse order: events=None sync finishes FIRST, then older async returns None.
        let mut versions = VersionDiscovery::default();
        let async_gen = versions
            .begin_probe(Runtime::OpenCode, command, false)
            .unwrap();
        let sync_gen = versions
            .begin_sync_probe(Runtime::OpenCode, command)
            .unwrap();
        assert!(!versions.finish_sync_probe(
            Runtime::OpenCode,
            sync_gen,
            command.into(),
            Some("1.19.5".into()),
            false,
        ));
        assert_eq!(
            versions.installed(Runtime::OpenCode, command),
            Some("1.19.5")
        );

        // Older async flushes pending notification without overwriting newer sync result.
        assert!(versions.finish_probe(Runtime::OpenCode, async_gen, command.into(), None));
        assert_eq!(
            versions.installed(Runtime::OpenCode, command),
            Some("1.19.5")
        );
        assert!(versions.probing.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn executable_fingerprint_invalidates_cached_and_in_flight_same_path_probes() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let command_path = dir.path().join("opencode");
        let write_exec = |body: &str| {
            let tmp = dir.path().join("opencode.tmp");
            std::fs::write(&tmp, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::rename(&tmp, &command_path).unwrap();
        };
        let command = command_path.to_str().unwrap();

        write_exec("echo '2.0.29'");
        let mut versions = VersionDiscovery::default();
        let g1 = versions
            .begin_sync_probe(Runtime::OpenCode, command)
            .unwrap();
        assert!(versions.finish_sync_probe(
            Runtime::OpenCode,
            g1,
            command.into(),
            Some("2.0.29".into()),
            true,
        ));
        assert_eq!(
            versions.installed(Runtime::OpenCode, command),
            Some("2.0.29")
        );
        assert_eq!(
            versions.probed(Runtime::OpenCode, command),
            Some(Some("2.0.29"))
        );
        assert_eq!(
            versions.begin_probe(Runtime::OpenCode, command, false),
            None
        );

        // Atomically replace the executable on disk with v1 without calling Refresh.
        write_exec("echo '1.19.5' # replaced v1 binary");
        assert_eq!(versions.installed(Runtime::OpenCode, command), None);
        assert_eq!(versions.probed(Runtime::OpenCode, command), None);

        // An in-flight async probe captured before a second swap is superseded when the binary changes again.
        let async_v1 = versions
            .begin_probe(Runtime::OpenCode, command, false)
            .unwrap();
        write_exec("echo '2.0.30' # second replacement binary");
        let async_v2 = versions
            .begin_probe(Runtime::OpenCode, command, false)
            .expect("replaced binary must supersede in-flight probe");
        assert!(!versions.finish_probe(
            Runtime::OpenCode,
            async_v1,
            command.into(),
            Some("1.19.5".into()),
        ));
        assert!(versions.finish_probe(
            Runtime::OpenCode,
            async_v2,
            command.into(),
            Some("2.0.30".into()),
        ));
        assert_eq!(
            versions.installed(Runtime::OpenCode, command),
            Some("2.0.30")
        );

        std::fs::remove_file(&command_path).unwrap();
        assert_eq!(versions.installed(Runtime::OpenCode, command), None);
        assert_eq!(versions.probed(Runtime::OpenCode, command), None);
    }

    #[test]
    fn clear_probes_and_clear_runtime_probe_evict_cache_and_supersede_in_flight() {
        let mut versions = VersionDiscovery::default();
        let in_flight = versions
            .begin_probe(Runtime::OpenCode, "/a/opencode", true)
            .unwrap();
        let other = versions
            .begin_probe(Runtime::Codex, "/a/codex", true)
            .unwrap();
        assert!(versions.finish_probe(
            Runtime::Codex,
            other,
            "/a/codex".into(),
            Some("0.153.4".into()),
        ));

        assert!(!versions.clear_runtime_probe(Runtime::OpenCode));
        assert!(!versions.has_in_flight_async(Runtime::OpenCode));
        // Stale generation from before clear_runtime_probe cannot commit later.
        assert!(!versions.finish_probe(
            Runtime::OpenCode,
            in_flight,
            "/a/opencode".into(),
            Some("2.0.29".into()),
        ));
        assert_eq!(versions.installed(Runtime::OpenCode, "/a/opencode"), None);
        assert_eq!(
            versions.installed(Runtime::Codex, "/a/codex"),
            Some("0.153.4")
        );

        let g = versions
            .begin_sync_probe(Runtime::OpenCode, "/a/opencode")
            .unwrap();
        assert!(versions.finish_sync_probe(
            Runtime::OpenCode,
            g,
            "/a/opencode".into(),
            Some("2.0.29".into()),
            true,
        ));
        assert!(versions.clear_runtime_probe(Runtime::OpenCode));
        assert_eq!(versions.installed(Runtime::OpenCode, "/a/opencode"), None);

        versions.clear_probes();
        assert_eq!(versions.installed(Runtime::Codex, "/a/codex"), None);
        assert!(versions.probing.is_empty());
        assert!(versions.in_flight_async.is_empty());
        assert!(versions.pending_async_notify.is_empty());
        assert!(versions.help_flags.is_empty());
    }

    #[test]
    fn only_runtimes_with_an_updater_and_a_latest_source_can_update() {
        assert!(updatable(Runtime::Codex));
        assert!(updatable(Runtime::ClaudeCode));
        assert!(updatable(Runtime::Antigravity));
        assert!(updatable(Runtime::Copilot));
        assert!(updatable(Runtime::Pi));
        assert!(!updatable(Runtime::Grok));
        assert!(!updatable(Runtime::OpenCode));
        assert!(!updatable(Runtime::Trae));
        assert!(!updatable(Runtime::Cursor));
        assert!(!updatable(Runtime::Shell));
    }

    #[test]
    fn antigravity_manifest_http_fetch_parses_version_and_degrades_on_errors() {
        use std::io::{Read as _, Write as _};
        use std::net::TcpListener;

        assert_eq!(
            LatestSource::Npm {
                package: "@openai/codex",
                tag: "latest",
            }
            .url(),
            "https://registry.npmjs.org/@openai/codex/latest"
        );
        let manifest_url = crate::runtimes::antigravity::current_manifest_url().unwrap();
        assert_eq!(LatestSource::ManifestUrl(manifest_url).url(), manifest_url);

        let responses: Vec<(&'static str, &'static str)> = vec![
            (
                "200 OK",
                r#"{"version":"1.5.17","url":"https://storage.googleapis.com/antigravity-public/antigravity-cli/1.5.17/cli.tar.gz","sha512":"abc123"}"#,
            ),
            ("404 Not Found", r#"{"version":"1.5.17"}"#),
            ("500 Internal Server Error", "oops"),
            ("200 OK", "not valid json"),
            ("200 OK", r#"{"url":"https://example.com/cli.tar.gz"}"#),
            ("200 OK", r#"{"version":"nightly"}"#),
        ];
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server_responses = responses.clone();
        let server = std::thread::spawn(move || {
            for (status, body) in server_responses {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = [0u8; 4096];
                let mut total = 0;
                while total < buf.len() {
                    let Ok(n) = stream.read(&mut buf[total..]) else {
                        break;
                    };
                    if n == 0 {
                        break;
                    }
                    total += n;
                    if buf[..total].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(reply.as_bytes()).unwrap();
            }
        });

        let env = LoginShellEnv::default();
        let url = format!("http://{addr}/manifests/darwin_arm64.json");
        assert_eq!(fetch_latest_from_url(&env, &url).as_deref(), Some("1.5.17"));
        for _ in 1..responses.len() {
            assert_eq!(fetch_latest_from_url(&env, &url), None);
        }
        server.join().unwrap();

        // Listener has dropped, so a subsequent fetch fails cleanly with None.
        assert_eq!(fetch_latest_from_url(&env, &url), None);
    }

    #[test]
    fn antigravity_manifest_check_caches_refreshes_supersedes_and_degrades_silently() {
        let discovery = discovery();
        let start = Instant::now();
        let source = crate::runtimes::antigravity::current_manifest_url().unwrap();
        let available = |installed: &str| {
            discovery
                .read()
                .unwrap()
                .versions
                .available(Runtime::Antigravity, Some(installed))
                .map(str::to_owned)
        };
        {
            let versions = &mut discovery.write().unwrap().versions;
            let generation = versions
                .begin_probe(Runtime::Antigravity, "/a/agy", false)
                .unwrap();
            versions.finish_probe(
                Runtime::Antigravity,
                generation,
                "/a/agy".into(),
                Some("1.5.8".into()),
            );
        }

        let fetches = std::cell::Cell::new(0);
        assert!(check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::Antigravity,
            source,
            false,
            start,
            || {
                fetches.set(fetches.get() + 1);
                Some("1.5.17".into())
            }
        ));
        assert_eq!(available("1.5.8").as_deref(), Some("1.5.17"));
        assert_eq!(available("1.5.17"), None);

        // Cached within 6 hours without force.
        let later = start + Duration::from_secs(5 * 60 * 60);
        assert!(!check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::Antigravity,
            source,
            false,
            later,
            || {
                fetches.set(fetches.get() + 1);
                Some("1.5.18".into())
            }
        ));
        assert_eq!(fetches.get(), 1);

        // Forced Refresh bypasses the cache.
        assert!(check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::Antigravity,
            source,
            true,
            later,
            || {
                fetches.set(fetches.get() + 1);
                Some("1.5.18".into())
            }
        ));
        assert_eq!(fetches.get(), 2);
        assert_eq!(available("1.5.8").as_deref(), Some("1.5.18"));

        // Failure degrades silently: clears available_version and keeps installed_version.
        assert!(check_latest(
            &discovery,
            &EventChannel::new(),
            Runtime::Antigravity,
            source,
            true,
            later,
            || None
        ));
        assert_eq!(available("1.5.8"), None);
        assert_eq!(
            discovery
                .read()
                .unwrap()
                .versions
                .installed(Runtime::Antigravity, "/a/agy"),
            Some("1.5.8")
        );
    }

    #[cfg(unix)]
    #[test]
    fn request_latest_fetches_antigravity_manifest_alongside_npm_runtimes() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Mutex;

        let bin = tempfile::tempdir().unwrap();
        for (name, version) in [
            ("codex", "codex-cli 0.164.0"),
            ("agy", "1.5.8"),
            ("grok", "1.0.89"),
            ("opencode", "2.1.0"),
        ] {
            let path = bin.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\necho '{version}'\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let pool = Arc::new(crate::db::open_in_memory().unwrap());
        let shell_env = Arc::new(RwLock::new(LoginShellEnv {
            path: Some(bin.path().display().to_string()),
            vars: std::collections::BTreeMap::from([(
                super::super::TEST_ISOLATE_PATH_ENV.into(),
                "1".into(),
            )]),
        }));
        let discovery = discovery();
        discovery.write().unwrap().checking = false;
        let events = EventChannel::new();

        let expected_manifest = crate::runtimes::antigravity::current_manifest_url().unwrap();
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let fetch = {
            let recorded = Arc::clone(&recorded);
            move |_: &LoginShellEnv, source: LatestSource| {
                recorded.lock().unwrap().push(source);
                match source {
                    LatestSource::Npm {
                        package: "@openai/codex",
                        ..
                    } => Some("0.165.0".into()),
                    LatestSource::ManifestUrl(url) if url == expected_manifest => {
                        Some("1.5.17".into())
                    }
                    _ => None,
                }
            }
        };

        request_latest_with(&pool, &shell_env, &discovery, &events, false, fetch);

        let codex_path = bin.path().join("codex").display().to_string();
        let agy_path = bin.path().join("agy").display().to_string();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let ready = {
                let state = discovery.read().unwrap();
                state.versions.installed(Runtime::Codex, &codex_path) == Some("0.164.0")
                    && state.versions.installed(Runtime::Antigravity, &agy_path) == Some("1.5.8")
                    && state.versions.available(Runtime::Codex, Some("0.164.0")) == Some("0.165.0")
                    && state
                        .versions
                        .available(Runtime::Antigravity, Some("1.5.8"))
                        == Some("1.5.17")
            };
            if ready {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for Codex and Antigravity latest checks"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        let fetched = recorded.lock().unwrap().clone();
        assert_eq!(fetched.len(), 2, "unexpected sources fetched: {fetched:?}");
        assert!(fetched.contains(&LatestSource::Npm {
            package: "@openai/codex",
            tag: "latest",
        }));
        assert!(fetched.contains(&LatestSource::ManifestUrl(expected_manifest)));

        let status = super::super::status_list(&pool, &shell_env, &discovery).unwrap();
        let agy = status
            .runtimes
            .iter()
            .find(|row| row.name == Runtime::Antigravity)
            .unwrap();
        assert_eq!(agy.installed_version.as_deref(), Some("1.5.8"));
        assert_eq!(agy.available_version.as_deref(), Some("1.5.17"));
        assert_eq!(agy.min_version.as_deref(), Some("1.5.8"));
        assert!(!agy.below_minimum);

        let codex = status
            .runtimes
            .iter()
            .find(|row| row.name == Runtime::Codex)
            .unwrap();
        assert_eq!(codex.installed_version.as_deref(), Some("0.164.0"));
        assert_eq!(codex.available_version.as_deref(), Some("0.165.0"));

        for runtime in [Runtime::Grok, Runtime::OpenCode] {
            let row = status
                .runtimes
                .iter()
                .find(|row| row.name == runtime)
                .unwrap();
            assert_eq!(row.available_version, None, "{runtime:?}");
        }
    }
}

#[test]
fn versions_catalog_golden() {
    let home = tempfile::tempdir().unwrap();
    let mut rows = Vec::new();
    crate::runtimes::with_conversation_home(home.path(), || {
        for channel in [None, Some("latest"), Some("stable"), Some("unknown")] {
            if let Some(channel) = channel {
                let path = home.path().join(".claude/settings.json");
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(
                    path,
                    serde_json::json!({"autoUpdatesChannel":channel}).to_string(),
                )
                .unwrap();
            }
            for runtime in Runtime::ALL {
                let adapter = crate::runtimes::adapter(runtime);
                let manifest = adapter.latest_manifest_url().map(|url| {
                    let (prefix, _) = url.rsplit_once('/').unwrap();
                    format!("{prefix}/<platform>.json")
                });
                rows.push(serde_json::json!({
                    "runtime": runtime,
                    "channel": channel,
                    "package": adapter.catalog().and_then(|catalog| catalog.npm_package),
                    "dist_tag": dist_tag(runtime),
                    "manifest": manifest,
                    "updatable": updatable(runtime),
                }));
            }
        }
    });
    crate::golden::assert_golden("catalog-versions", serde_json::json!(rows));
}

#[cfg(test)]
mod minimum_tests {
    use super::*;
    #[test]
    fn minimum_requires_a_known_installed_version_and_orders_prereleases() {
        for installed in [
            None,
            Some("unknown"),
            Some("0.149.0"),
            Some("0.150.0"),
            Some("v0.149.0+build.1"),
        ] {
            assert!(!below_minimum(installed, Some("0.149.0")), "{installed:?}");
        }
        for installed in ["0.147.0", "0.148.0", "0.149.0-alpha.1"] {
            assert!(below_minimum(Some(installed), Some("0.149.0")));
        }
        assert!(!below_minimum(Some("0.147.0"), None));
    }
}
