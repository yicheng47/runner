//! Installed agent CLI versions and whether a newer one exists (#533).
//!
//! The installed version is the first semver token `<command> --version`
//! prints. "Newer" comes from the npm registry's `latest` dist-tag, which
//! every supported CLI publishes at the version it ships natively, so one
//! endpoint covers native, npm and Homebrew installs. Every failure degrades
//! to showing the version alone.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{
    effective_runtime_command, RuntimeCommandSource, SharedDiscoveryState, SharedShellEnv,
};
use crate::db::DbPool;
use crate::events::EventChannel;
use crate::model::Runtime;
use crate::router::runtime::{runtime_definition, runtime_definitions};
use crate::shell_path::LoginShellEnv;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const LATEST_TTL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, Default)]
pub(crate) struct VersionDiscovery {
    installed: HashMap<Runtime, Probe>,
    /// The newest probe started per runtime: its command and generation.
    /// Only that probe may store its result; older ones are superseded.
    probing: HashMap<Runtime, (String, u64)>,
    next_probe: u64,
    latest: HashMap<Runtime, Latest>,
    /// The newest npm check started per runtime: its tag and generation.
    /// A check for another tag supersedes it, like an override does a probe.
    checking: HashMap<Runtime, (&'static str, u64)>,
    next_check: u64,
    /// A check asked for while discovery was still resolving executables;
    /// `true` when it was forced. It runs once discovery completes.
    deferred_latest: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Probe {
    command: String,
    version: Option<String>,
}

#[derive(Debug, Clone)]
struct Latest {
    /// The npm dist-tag this answer came from.
    tag: &'static str,
    version: String,
    fetched_at: Instant,
}

impl VersionDiscovery {
    /// The probed version of `command`, if the last probe ran that command.
    pub(crate) fn installed(&self, runtime: Runtime, command: &str) -> Option<&str> {
        self.installed
            .get(&runtime)
            .filter(|probe| probe.command == command)
            .and_then(|probe| probe.version.as_deref())
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
        let running = self
            .probing
            .get(&runtime)
            .is_some_and(|(running, _)| running == command);
        let answered = self
            .installed
            .get(&runtime)
            .is_some_and(|probe| probe.command == command);
        if !force && running {
            return None;
        }
        if !force && answered {
            self.probing.remove(&runtime);
            return None;
        }
        self.next_probe += 1;
        self.probing
            .insert(runtime, (command.to_owned(), self.next_probe));
        Some(self.next_probe)
    }

    /// Stores the result of the newest probe for `runtime` and reports
    /// whether the shown version changed. A superseded probe — an override
    /// set while it ran, or a later forced probe — is discarded.
    fn finish_probe(
        &mut self,
        runtime: Runtime,
        generation: u64,
        command: String,
        version: Option<String>,
    ) -> bool {
        if self
            .probing
            .get(&runtime)
            .is_none_or(|(_, newest)| *newest != generation)
        {
            return false;
        }
        self.probing.remove(&runtime);
        let probe = Probe { command, version };
        self.installed.insert(runtime, probe.clone()) != Some(probe)
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct Version {
    core: [u64; 3],
    prerelease: bool,
}

fn parse_semver(token: &str) -> Option<Version> {
    let token = token.strip_prefix('v').unwrap_or(token);
    let token = token.split('+').next()?;
    let (core, prerelease) = match token.split_once('-') {
        Some((core, prerelease)) if !prerelease.is_empty() => (core, true),
        Some(_) => return None,
        None => (token, false),
    };
    let mut parts = core.split('.');
    let mut numbers = [0; 3];
    for number in &mut numbers {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        *number = part.parse().ok()?;
    }
    parts.next().is_none().then_some(Version {
        core: numbers,
        prerelease,
    })
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
    let (Some(latest), Some(installed)) = (parse_semver(latest), parse_semver(installed)) else {
        return false;
    };
    match latest.core.cmp(&installed.core) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => installed.prerelease && !latest.prerelease,
    }
}

fn installed_command(
    runtime: Runtime,
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
) -> Option<String> {
    effective_runtime_command(runtime, pool, shell_env, discovery)
        .ok()
        .filter(|command| command.source != RuntimeCommandSource::Catalog)
        .map(|command| command.command)
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

/// Checks npm for a newer version of every installed runtime that has an
/// updater, on background threads. Answers are cached for six hours per app
/// run; `force` (Refresh) skips the cache. While discovery is still
/// resolving executables the check waits for it, so a CLI that discovery is
/// about to find is checked too.
pub(crate) fn request_latest(
    pool: &Arc<DbPool>,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    events: &EventChannel,
    force: bool,
) {
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

fn run_deferred_latest_with(
    pool: &Arc<DbPool>,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    events: &EventChannel,
    fetch: fn(&LoginShellEnv, &str, &str) -> Option<String>,
) {
    let deferred = discovery
        .write()
        .ok()
        .and_then(|mut state| state.versions.deferred_latest.take());
    if let Some(force) = deferred {
        request_latest_with(pool, shell_env, discovery, events, force, fetch);
    }
}

fn request_latest_with(
    pool: &Arc<DbPool>,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    events: &EventChannel,
    force: bool,
    fetch: fn(&LoginShellEnv, &str, &str) -> Option<String>,
) {
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
    for definition in runtime_definitions() {
        let Some(package) = definition.npm_package else {
            continue;
        };
        if definition.update_args.is_empty()
            || installed_command(definition.name, pool, shell_env, discovery).is_none()
        {
            continue;
        }
        let runtime = definition.name;
        let shell_env = Arc::clone(shell_env);
        let discovery = Arc::clone(discovery);
        let events = events.clone();
        std::thread::spawn(move || {
            let env = shell_env.read().map(|env| env.clone()).unwrap_or_default();
            let tag = dist_tag(runtime);
            if check_latest(&discovery, runtime, tag, force, Instant::now(), || {
                fetch(&env, package, tag)
            }) {
                events.emit("runtime/changed", &());
            }
        });
    }
}

/// One cached npm check with the fetch passed in, so tests stub the network.
/// Returns whether the shown latest version changed. A check superseded by a
/// newer one, for another tag or forced, is discarded.
fn check_latest(
    discovery: &SharedDiscoveryState,
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
    let Some(generation) = generation else {
        return dropped;
    };
    let version = fetch();
    if version.is_none() {
        log::debug!("runtime latest check failed: runtime={runtime} tag={tag}");
    }
    let finished = discovery.write().is_ok_and(|mut state| {
        state
            .versions
            .finish_latest(runtime, generation, tag, version, now)
    });
    dropped || finished
}

/// The npm dist-tag a runtime's own updater follows. Claude Code follows the
/// release channel in its user settings, `autoUpdatesChannel`: `latest` by
/// default, or `stable`, which npm publishes as its own tag. The others
/// follow `latest`.
fn dist_tag(runtime: Runtime) -> &'static str {
    if runtime != Runtime::ClaudeCode {
        return "latest";
    }
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|dir| !dir.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| runner_core::app_paths::home_dir().map(|home| home.join(".claude")))
        .and_then(|dir| claude_channel(&dir.join("settings.json")))
        .unwrap_or("latest")
}

fn claude_channel(settings: &std::path::Path) -> Option<&'static str> {
    let settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(settings).ok()?).ok()?;
    match settings.get("autoUpdatesChannel")?.as_str()? {
        "stable" => Some("stable"),
        "latest" => Some("latest"),
        _ => None,
    }
}

fn fetch_latest(env: &LoginShellEnv, package: &str, tag: &str) -> Option<String> {
    let response = crate::usage::http_client(env)
        .ok()?
        .get(format!("https://registry.npmjs.org/{package}/{tag}"))
        .send()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    parse_latest(&response.json().ok()?)
}

fn parse_latest(body: &serde_json::Value) -> Option<String> {
    let version = body.get("version")?.as_str()?.trim();
    parse_semver(version).map(|_| version.to_owned())
}

/// Whether the runtime has an update subcommand Runner can run.
pub(crate) fn updatable(runtime: Runtime) -> bool {
    runtime_definition(runtime).is_some_and(|definition| {
        !definition.update_args.is_empty() && definition.npm_package.is_some()
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
            Runtime::Codex,
            "latest",
            false,
            later,
            || { fetch("0.156.0") }
        ));
        assert_eq!(fetches.get(), 1);

        assert!(check_latest(
            &discovery,
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
            Runtime::Codex,
            "latest",
            true,
            now,
            || { Some("0.155.0".into()) }
        ));
        assert_eq!(available(&discovery, "0.153.4").as_deref(), Some("0.155.0"));
        assert!(check_latest(
            &discovery,
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
                check_latest(&discovery, Runtime::ClaudeCode, "latest", true, now, || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Some("2.1.281".into())
                })
            })
        };
        entered_rx.recv().unwrap();

        assert!(check_latest(
            &discovery,
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
    static STUB_FETCHES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    #[cfg(unix)]
    fn stub_fetch(_: &LoginShellEnv, package: &str, _: &str) -> Option<String> {
        STUB_FETCHES.lock().unwrap().push(package.to_owned());
        Some("99.0.0".into())
    }

    #[cfg(unix)]
    #[test]
    fn a_check_requested_during_discovery_runs_once_it_completes() {
        use std::os::unix::fs::PermissionsExt;
        let bin = tempfile::tempdir().unwrap();
        let codex = bin.path().join("codex");
        std::fs::write(&codex, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
        let pool = Arc::new(crate::db::open_in_memory().unwrap());
        let shell_env = Arc::new(RwLock::new(LoginShellEnv {
            path: Some(bin.path().display().to_string()),
            vars: Default::default(),
        }));
        let discovery = discovery();
        let events = EventChannel::new();

        request_latest_with(&pool, &shell_env, &discovery, &events, false, stub_fetch);
        request_latest_with(&pool, &shell_env, &discovery, &events, true, stub_fetch);
        assert_eq!(
            discovery.read().unwrap().versions.deferred_latest,
            Some(true)
        );
        assert!(STUB_FETCHES.lock().unwrap().is_empty());

        discovery.write().unwrap().checking = false;
        run_deferred_latest_with(&pool, &shell_env, &discovery, &events, stub_fetch);
        let deadline = Instant::now() + Duration::from_secs(10);
        while available(&discovery, "0.153.4").is_none() {
            assert!(Instant::now() < deadline, "deferred check never ran");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(available(&discovery, "0.153.4").as_deref(), Some("99.0.0"));
        assert!(STUB_FETCHES
            .lock()
            .unwrap()
            .contains(&"@openai/codex".to_owned()));
        assert_eq!(discovery.read().unwrap().versions.deferred_latest, None);
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
        let shell_env = Arc::new(RwLock::new(LoginShellEnv::default()));
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
    fn only_runtimes_with_an_updater_and_a_package_can_update() {
        assert!(updatable(Runtime::Codex));
        assert!(updatable(Runtime::ClaudeCode));
        assert!(updatable(Runtime::Copilot));
        assert!(!updatable(Runtime::Trae));
        assert!(!updatable(Runtime::Pi));
    }
}
