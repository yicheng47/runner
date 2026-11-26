use crate::model::Runtime;
#[cfg(test)]
use crate::shell_path::DiscoveryOutcome;
use std::path::Path;
#[cfg(all(test, unix))]
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::db::{self, DbPool, LoginShellEnvLkg};
use crate::error::{Error, Result};
use crate::events::EventChannel;

use crate::session::launch;
use crate::shell_path::{DiscoveryResult, DiscoveryState, LoginShellEnv};

pub(crate) mod models;
pub(crate) mod versions;

pub type SharedShellEnv = Arc<RwLock<LoginShellEnv>>;
pub type SharedDiscoveryState = Arc<RwLock<DiscoveryState>>;

pub use runner_core::protocol::runtime::RuntimeCommandSource;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveRuntimeCommand {
    pub command: String,
    pub source: RuntimeCommandSource,
}

pub use runner_core::protocol::runtime::RuntimeRowState;

pub use runner_core::protocol::runtime::ShellDiscoveryStatus;

pub use runner_core::protocol::runtime::RuntimeExecutableStatus;

pub use runner_core::protocol::runtime::RuntimeStatusResponse;

pub use runner_core::protocol::runtime::OverrideValidationError;

pub fn withholds_below_minimum(runtime: &crate::runtimes::RuntimeCatalog) -> bool {
    runner_core::protocol::runtime_metadata::withhold_below_version(runtime.name).is_some()
}

pub fn is_withheld_version(
    runtime: &crate::runtimes::RuntimeCatalog,
    installed: Option<&str>,
) -> bool {
    let Some(limit) = runner_core::protocol::runtime_metadata::withhold_below_version(runtime.name)
    else {
        return false;
    };
    versions::below_minimum(installed, Some(limit))
}

pub(crate) fn unsupported_version_message(
    runtime: &crate::runtimes::RuntimeCatalog,
    installed: &str,
) -> String {
    let detected = if runtime.name == Runtime::OpenCode {
        "OpenCode"
    } else {
        runtime.display_name
    };
    let minimum = runtime.min_version.unwrap_or_default();
    format!(
        "{detected} {installed} was found. Runner only supports {} ({minimum} or newer).",
        runtime.display_name
    )
}

pub fn status_list(
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
) -> Result<RuntimeStatusResponse> {
    status_list_with_events(pool, shell_env, discovery, None)
}

pub(crate) fn status_list_with_events(
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    events: Option<&crate::events::EventChannel>,
) -> Result<RuntimeStatusResponse> {
    let shell_env_arc = shell_env;
    let discovery_arc = discovery;
    let shell_env = shell_env
        .read()
        .map_err(|_| Error::msg("runtime shell environment lock poisoned"))?
        .clone();
    let discovery = discovery
        .read()
        .map_err(|_| Error::msg("runtime discovery lock poisoned"))?
        .clone();
    let config_env =
        crate::runtimes::claude_code::accounts::sharing::config_env(pool, shell_env.vars.clone());
    let overrides = db::runtime_overrides(pool)?;
    let launch_options = db::runtime_launch_options(pool)?;
    let path = direct_chat_path(&shell_env);
    let npm_available = find_executable("npm", &path).is_some();
    let home = runner_core::app_paths::home_dir();
    let result = discovery.result.as_ref();
    let failed = result.is_some_and(|result| !result.outcome.is_success());

    let runtimes = crate::runtimes::catalogs()
        .iter()
        .map(|runtime| {
            let defaults = home
                .as_deref()
                .map(|home| {
                    crate::runtimes::adapter(runtime.name)
                        .native_defaults_with_env(home, &config_env)
                })
                .unwrap_or_default();
            let detected_path =
                find_executable(runtime.command, &path).map(|path| path.display().to_string());
            let override_path = overrides.get(runtime.name.key()).cloned();
            let mut invalid_reason = override_path
                .as_deref()
                .and_then(|path| validate_executable_path(Path::new(path)).err())
                .map(|error| error.message);
            let mut valid_override = override_path
                .as_deref()
                .filter(|_| invalid_reason.is_none())
                .map(str::to_string);
            let mut override_version = None;
            let mut override_below_minimum = false;
            if let Some(path) = valid_override.as_deref() {
                override_version = match discovery.versions.probed(runtime.name, path) {
                    Some(version) => version.map(str::to_owned),
                    None if withholds_below_minimum(runtime) => versions::ensure_probed(
                        runtime.name,
                        path,
                        shell_env_arc,
                        discovery_arc,
                        events,
                    ),
                    None => None,
                };
                if is_withheld_version(runtime, override_version.as_deref()) {
                    invalid_reason = Some(unsupported_version_message(
                        runtime,
                        override_version.as_deref().unwrap(),
                    ));
                    valid_override = None;
                    override_below_minimum = true;
                }
            }
            let detected_version = override_path
                .is_none()
                .then(|| {
                    detected_path
                        .as_deref()
                        .and_then(|command| discovery.versions.installed(runtime.name, command))
                        .map(str::to_owned)
                })
                .flatten();
            let detected_below_minimum = override_path.is_none()
                && is_withheld_version(runtime, detected_version.as_deref());
            let (effective_command, effective_source) =
                if override_below_minimum || detected_below_minimum {
                    (None, None)
                } else if let Some(path) = valid_override {
                    (Some(path), Some(RuntimeCommandSource::Override))
                } else if let Some(path) = detected_path.clone() {
                    (Some(path), Some(RuntimeCommandSource::Detected))
                } else if discovery.checking {
                    (
                        Some(runtime.command.to_string()),
                        Some(RuntimeCommandSource::Catalog),
                    )
                } else {
                    (None, None)
                };
            let state = if invalid_reason.is_some() {
                RuntimeRowState::InvalidOverride
            } else if override_path.is_some() {
                RuntimeRowState::Override
            } else if detected_below_minimum {
                RuntimeRowState::NotFound
            } else if discovery.checking {
                RuntimeRowState::Checking
            } else if failed {
                RuntimeRowState::ProbeTimedOut
            } else if detected_path.is_some() {
                RuntimeRowState::Detected
            } else {
                RuntimeRowState::NotFound
            };
            let installed_version = if override_below_minimum {
                override_version
            } else if detected_below_minimum {
                detected_version
            } else {
                effective_command
                    .as_deref()
                    .filter(|_| effective_source != Some(RuntimeCommandSource::Catalog))
                    .and_then(|command| {
                        override_version
                            .as_deref()
                            .or_else(|| discovery.versions.installed(runtime.name, command))
                    })
                    .map(str::to_owned)
            };
            let available_version = versions::updatable(runtime.name)
                .then(|| {
                    discovery
                        .versions
                        .available(runtime.name, installed_version.as_deref())
                })
                .flatten()
                .map(str::to_owned);
            RuntimeExecutableStatus {
                launch_options: launch_options
                    .get(runtime.name.key())
                    .cloned()
                    .unwrap_or_default(),
                name: runtime.name,
                display_name: runtime.display_name.to_string(),
                command: runtime.command.to_string(),
                default_model: defaults.model,
                default_effort: defaults.effort,
                detected_path,
                override_path,
                effective_command,
                effective_source,
                state,
                invalid_reason,
                below_minimum: versions::below_minimum(
                    installed_version.as_deref(),
                    runtime.min_version,
                ),
                min_version: runtime.min_version.map(str::to_owned),
                installed_version,
                available_version,
                sign_in: runner_core::protocol::RuntimeSignInStatus::Unknown,
            }
        })
        .collect();

    Ok(RuntimeStatusResponse {
        npm_available,
        shell: ShellDiscoveryStatus {
            shell: result
                .and_then(|result| result.shell.clone())
                .or(discovery.seeded_shell),
            outcome: result.map(|result| result.outcome),
            duration_ms: result.map(|result| result.duration_ms),
            checking: discovery.checking,
            using_last_known_good: discovery.last_known_good_captured_at.is_some()
                && (discovery.checking || failed),
            last_known_good_captured_at: discovery.last_known_good_captured_at,
        },
        runtimes,
    })
}

pub(crate) fn candidate_runtime_command(
    runtime: Runtime,
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
) -> Result<EffectiveRuntimeCommand> {
    let overrides = db::runtime_overrides(pool)?;
    let shell_env = shell_env
        .read()
        .map_err(|_| Error::msg("runtime shell environment lock poisoned"))?
        .clone();
    let checking = discovery
        .read()
        .map_err(|_| Error::msg("runtime discovery lock poisoned"))?
        .checking;
    effective_runtime_command_on_path(runtime, &overrides, &direct_chat_path(&shell_env), checking)
}

pub fn effective_runtime_command(
    runtime: Runtime,
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
) -> Result<EffectiveRuntimeCommand> {
    effective_runtime_command_with_events(runtime, pool, shell_env, discovery, None)
}

pub(crate) fn effective_runtime_command_with_events(
    runtime: Runtime,
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    events: Option<&crate::events::EventChannel>,
) -> Result<EffectiveRuntimeCommand> {
    let candidate = candidate_runtime_command(runtime, pool, shell_env, discovery)?;
    let Some(definition) = crate::runtimes::adapter(runtime).catalog() else {
        return Ok(candidate);
    };
    if candidate.source != RuntimeCommandSource::Catalog && withholds_below_minimum(&definition) {
        let installed =
            versions::ensure_probed(runtime, &candidate.command, shell_env, discovery, events);
        if is_withheld_version(&definition, installed.as_deref()) {
            return Err(runner_core::protocol::runtime::RuntimeVersionGate {
                runtime,
                installed_version: installed.unwrap(),
                min_version: definition.min_version.unwrap().into(),
            }
            .into());
        }
    }
    Ok(candidate)
}

fn effective_runtime_command_on_path(
    runtime: Runtime,
    overrides: &std::collections::BTreeMap<String, String>,
    path: &str,
    checking: bool,
) -> Result<EffectiveRuntimeCommand> {
    let definition = crate::runtimes::adapter(runtime)
        .catalog()
        .ok_or_else(|| Error::msg(format!("unknown runtime: {runtime}")))?;
    if let Some(path) = overrides.get(runtime.key()) {
        if validate_executable_path(Path::new(path)).is_ok() {
            log::info!(
                "runtime executable: runtime={} source=override command={}",
                runtime,
                path
            );
            return Ok(EffectiveRuntimeCommand {
                command: path.clone(),
                source: RuntimeCommandSource::Override,
            });
        }
    }

    if let Some(path) = find_executable(definition.command, path) {
        let command = path.display().to_string();
        log::info!(
            "runtime executable: runtime={} source=detected command={}",
            runtime,
            command
        );
        return Ok(EffectiveRuntimeCommand {
            command,
            source: RuntimeCommandSource::Detected,
        });
    }

    if checking {
        return Ok(EffectiveRuntimeCommand {
            command: definition.command.to_string(),
            source: RuntimeCommandSource::Catalog,
        });
    }

    log::warn!(
        "runtime executable not found: runtime={} catalog_command={}",
        runtime,
        definition.command
    );
    Err(runtime_not_found_error(runtime))
}

pub fn runtime_not_found_error(runtime: Runtime) -> Error {
    let name = crate::runtimes::adapter(runtime)
        .catalog()
        .map(|definition| definition.display_name)
        .unwrap_or(runtime.key());
    Error::msg(format!(
        "{name} executable was not found. Open Settings → Agents to refresh discovery or set an override."
    ))
}

pub fn validate_override(
    runtime: Runtime,
    path: &str,
) -> std::result::Result<(), OverrideValidationError> {
    validate_override_with_env(runtime, path, &LoginShellEnv::default())
}

pub(crate) fn validate_override_with_env(
    runtime: Runtime,
    path: &str,
    env: &LoginShellEnv,
) -> std::result::Result<(), OverrideValidationError> {
    validate_and_probe_override(runtime, path, env).map(|_| ())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedOverrideProbe {
    pub(crate) version: Option<String>,
    pub(crate) fingerprint: Option<versions::ExecutableFingerprint>,
}

pub(crate) fn validate_and_probe_override(
    runtime: Runtime,
    path: &str,
    env: &LoginShellEnv,
) -> std::result::Result<Option<ValidatedOverrideProbe>, OverrideValidationError> {
    let Some(definition) = crate::runtimes::adapter(runtime).catalog() else {
        return Err(validation_error(
            "unknown_runtime",
            format!("Unknown runtime: {runtime}."),
        ));
    };
    validate_executable_path(Path::new(path))?;
    if withholds_below_minimum(&definition) {
        let fingerprint = versions::executable_fingerprint(path);
        let version = versions::probe_command_version(path, env);
        if let Some(installed) = version
            .as_deref()
            .filter(|version| is_withheld_version(&definition, Some(version)))
        {
            return Err(validation_error(
                "unsupported_version",
                unsupported_version_message(&definition, installed),
            ));
        }
        return Ok(Some(ValidatedOverrideProbe {
            version,
            fingerprint,
        }));
    }
    Ok(None)
}

use runner_core::protocol::runtime_paths::validate_executable_path;

use runner_core::protocol::runtime_paths::validation_error;

#[cfg(test)]
pub(crate) const TEST_ISOLATE_PATH_ENV: &str = "__RUNNER_TEST_ISOLATE_PATH";

#[cfg(test)]
pub(crate) const TEST_FALLBACK_PATH_ENV: &str = "__RUNNER_TEST_FALLBACK_PATH";

#[cfg(test)]
pub(crate) fn is_path_isolated(shell_env: &SharedShellEnv) -> bool {
    shell_env.read().is_ok_and(|env| {
        env.vars
            .get(TEST_ISOLATE_PATH_ENV)
            .is_some_and(|value| value == "1")
    })
}

pub fn direct_chat_path(shell_env: &LoginShellEnv) -> String {
    #[cfg(test)]
    if shell_env
        .vars
        .get(TEST_ISOLATE_PATH_ENV)
        .is_some_and(|value| value == "1")
    {
        return shell_env.path.clone().unwrap_or_default();
    }

    #[cfg(test)]
    if let Some(fallback_path) = shell_env.vars.get(TEST_FALLBACK_PATH_ENV) {
        return match shell_env.path.as_deref() {
            Some(path) if !path.is_empty() => std::env::join_paths(
                std::env::split_paths(path).chain(std::env::split_paths(fallback_path)),
            )
            .expect("test PATH entries must not contain the platform separator")
            .to_string_lossy()
            .into_owned(),
            _ => fallback_path.clone(),
        };
    }

    let process_path = std::env::var("PATH").ok();
    let home = runner_core::app_paths::home_dir();
    launch::compose_path(
        None,
        None,
        shell_env.path.as_deref(),
        home.as_deref(),
        process_path.as_deref(),
    )
}

pub use runner_core::protocol::runtime_paths::find_executable;

pub fn apply_discovery_result(
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    result: DiscoveryResult,
) -> Result<()> {
    let mut persistence_error = None;
    let captured_at = if result.outcome.is_success() {
        let captured_at = chrono::Utc::now().to_rfc3339();
        *shell_env
            .write()
            .map_err(|_| Error::msg("runtime shell environment lock poisoned"))? =
            result.env.clone();
        let snapshot = LoginShellEnvLkg {
            env: result.env.clone(),
            shell: result.shell.clone().unwrap_or_default(),
            captured_at: captured_at.clone(),
        };
        if let Err(error) = db::set_login_shell_env_lkg(pool, &snapshot) {
            persistence_error = Some(error);
        }
        Some(captured_at)
    } else {
        None
    };

    let mut state = discovery
        .write()
        .map_err(|_| Error::msg("runtime discovery lock poisoned"))?;
    state.checking = false;
    state.seeded_shell = if result.outcome.is_success() {
        result.shell.clone()
    } else {
        result.shell.clone().or_else(|| state.seeded_shell.clone())
    };
    state.result = Some(result);
    if let Some(captured_at) = captured_at {
        state.last_known_good_captured_at = Some(captured_at);
    }
    drop(state);
    match persistence_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Runs executable discovery in the background. `model_runtimes` are the
/// installed-and-enabled runtimes whose model catalogs may be queried
/// afterwards; executable discovery itself always covers every runtime.
pub fn start_background_discovery(
    events: EventChannel,
    pool: Arc<DbPool>,
    shell_env: SharedShellEnv,
    discovery: SharedDiscoveryState,
    force_models: bool,
    model_runtimes: Vec<Runtime>,
) {
    #[cfg(test)]
    if is_path_isolated(&shell_env) {
        return;
    }
    std::thread::spawn(move || {
        run_background_discovery_with(
            events,
            pool,
            shell_env,
            discovery,
            force_models,
            model_runtimes,
            crate::shell_path::resolve_login_shell_env,
        );
    });
}

pub(crate) fn run_background_discovery_with(
    events: EventChannel,
    pool: Arc<DbPool>,
    shell_env: SharedShellEnv,
    discovery: SharedDiscoveryState,
    force_models: bool,
    model_runtimes: Vec<Runtime>,
    resolve_env: impl FnOnce() -> DiscoveryResult,
) {
    // Persisted catalogs publish before the shell probe, so model fields
    // never wait on discovery to show what they already know.
    models::load_cached(&pool, &discovery, &events);
    let result = resolve_env();
    if let Err(error) = apply_discovery_result(&pool, &shell_env, &discovery, result) {
        log::warn!("runtime discovery state update failed: {error}");
        if let Ok(mut state) = discovery.write() {
            state.checking = false;
        }
    }
    log_runtime_paths(&pool, &shell_env);
    events.emit("runtime/changed", &());
    let runtimes: Vec<_> = crate::runtimes::catalogs()
        .iter()
        .map(|runtime| runtime.name)
        .collect();
    versions::request_probes(&pool, &shell_env, &discovery, &events, &runtimes, true);
    versions::run_deferred_latest(&pool, &shell_env, &discovery, &events);
    models::request(
        &pool,
        &shell_env,
        &discovery,
        &events,
        &model_runtimes,
        force_models,
    );
}

pub fn refresh_background_discovery(
    events: EventChannel,
    pool: Arc<DbPool>,
    shell_env: SharedShellEnv,
    discovery: SharedDiscoveryState,
    model_runtimes: Vec<Runtime>,
) -> Result<bool> {
    #[cfg(test)]
    if is_path_isolated(&shell_env) {
        return Ok(false);
    }
    refresh_background_discovery_with(
        events,
        pool,
        shell_env,
        discovery,
        model_runtimes,
        crate::shell_path::resolve_login_shell_env,
    )
    .map(|worker| worker.is_some())
}

fn refresh_background_discovery_with(
    events: EventChannel,
    pool: Arc<DbPool>,
    shell_env: SharedShellEnv,
    discovery: SharedDiscoveryState,
    model_runtimes: Vec<Runtime>,
    resolve_env: impl FnOnce() -> DiscoveryResult + Send + 'static,
) -> Result<Option<std::thread::JoinHandle<()>>> {
    {
        let mut state = discovery
            .write()
            .map_err(|_| Error::msg("runtime discovery lock poisoned"))?;
        if state.checking {
            return Ok(None);
        }
        state.checking = true;
        state.versions.clear_probes();
    }
    events.emit("runtime/changed", &());
    Ok(Some(std::thread::spawn(move || {
        run_background_discovery_with(
            events,
            pool,
            shell_env,
            discovery,
            true,
            model_runtimes,
            resolve_env,
        );
    })))
}

fn log_runtime_paths(pool: &DbPool, shell_env: &SharedShellEnv) {
    let Ok(shell_env) = shell_env.read() else {
        return;
    };
    let path = direct_chat_path(&shell_env);
    for runtime in crate::runtimes::catalogs() {
        match find_executable(runtime.command, &path) {
            Some(found) => log::info!(
                "runtime discovery result: runtime={} detected={}",
                runtime.name,
                found.display()
            ),
            None => log::info!(
                "runtime discovery result: runtime={} detected=not_found",
                runtime.name
            ),
        }
    }
    if let Ok(overrides) = db::runtime_overrides(pool) {
        for (runtime, path) in overrides {
            log::info!(
                "runtime override configured: runtime={} valid={}",
                runtime,
                validate_executable_path(Path::new(&path)).is_ok()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    fn executable(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn completed_discovery() -> SharedDiscoveryState {
        Arc::new(RwLock::new(DiscoveryState {
            checking: false,
            result: Some(DiscoveryResult {
                shell: Some("/bin/zsh".into()),
                outcome: DiscoveryOutcome::Ok,
                duration_ms: 10,
                env: LoginShellEnv::default(),
            }),
            seeded_shell: None,
            last_known_good_captured_at: None,
            models: Default::default(),
            versions: Default::default(),
        }))
    }

    #[test]
    fn status_list_includes_all_catalog_runtimes() {
        let pool = crate::db::open_in_memory().unwrap();
        let shell_env = Arc::new(RwLock::new(crate::test_support::isolated_shell_env(None)));
        let status = status_list(&pool, &shell_env, &completed_discovery()).unwrap();
        assert_eq!(
            status
                .runtimes
                .iter()
                .map(|runtime| (
                    runtime.name,
                    runtime.display_name.as_str(),
                    runtime.command.as_str(),
                ))
                .collect::<Vec<_>>(),
            vec![
                (Runtime::Codex, "Codex", "codex"),
                (Runtime::ClaudeCode, "Claude Code", "claude"),
                (Runtime::Antigravity, "Antigravity CLI", "agy"),
                (Runtime::Pi, "pi", "pi"),
                (Runtime::Copilot, "GitHub Copilot CLI", "copilot"),
                (Runtime::Trae, "TRAE CLI", "traecli"),
                (Runtime::Cursor, "Cursor", "cursor-agent"),
                (Runtime::Grok, "Grok", "grok"),
                (Runtime::OpenCode, "OpenCode v2", "opencode"),
            ],
        );
    }

    #[test]
    fn no_shell_classification_reports_failed_discovery() {
        let pool = crate::db::open_in_memory().unwrap();
        let shell_env = Arc::new(RwLock::new(crate::test_support::isolated_shell_env(None)));
        let discovery = completed_discovery();
        discovery.write().unwrap().last_known_good_captured_at = Some("saved".into());
        let success = status_list(&pool, &shell_env, &discovery).unwrap();
        discovery.write().unwrap().result.as_mut().unwrap().outcome = DiscoveryOutcome::NoShell;
        let no_shell = status_list(&pool, &shell_env, &discovery).unwrap();
        assert!(no_shell.shell.using_last_known_good);
        for actual in &no_shell.runtimes {
            assert_eq!(actual.state, RuntimeRowState::ProbeTimedOut);
        }
        for (expected, actual) in success.runtimes.iter().zip(&no_shell.runtimes) {
            assert_eq!(actual.detected_path, expected.detected_path);
            assert_eq!(actual.effective_command, expected.effective_command);
        }
    }

    #[test]
    fn windows_registry_classification_is_successful() {
        let pool = crate::db::open_in_memory().unwrap();
        let shell_env = Arc::new(RwLock::new(crate::test_support::isolated_shell_env(None)));
        let discovery = completed_discovery();
        discovery.write().unwrap().result.as_mut().unwrap().outcome =
            DiscoveryOutcome::WindowsRegistry;
        let status = status_list(&pool, &shell_env, &discovery).unwrap();
        assert!(!status.shell.using_last_known_good);
        assert!(status
            .runtimes
            .iter()
            .all(|runtime| runtime.state != RuntimeRowState::ProbeTimedOut));
    }

    #[cfg(windows)]
    #[test]
    fn resolver_uses_pathext_before_bare_names_and_keeps_path_order() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let command = "runner-path-test";
        let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        let extension = extensions
            .split(';')
            .find(|extension| !extension.is_empty())
            .unwrap();
        let expected = first.join(format!("{command}{extension}"));
        std::fs::write(&expected, "").unwrap();
        std::fs::write(first.join(command), "").unwrap();
        let later = second.join(format!("{command}{extension}"));
        std::fs::write(&later, "").unwrap();
        let path = std::env::join_paths([first, second]).unwrap();
        let path = path.to_str().unwrap();
        assert_eq!(find_executable(command, path), Some(expected.clone()));
        std::fs::remove_file(expected).unwrap();
        assert_eq!(find_executable(command, path), Some(later.clone()));
        assert_eq!(
            find_executable(&format!("{command}{extension}"), path),
            Some(later.clone())
        );
        std::fs::remove_file(later).unwrap();
        assert_eq!(find_executable(command, path), None);
    }

    #[test]
    #[cfg(unix)]
    fn resolver_requires_regular_executable_file() {
        let dir = tempfile::tempdir().unwrap();
        let executable = executable(dir.path(), "codex");
        assert_eq!(
            find_executable("codex", &dir.path().display().to_string()),
            Some(executable.clone())
        );
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o644);
        std::fs::set_permissions(&executable, permissions).unwrap();
        assert_eq!(
            find_executable("codex", &dir.path().display().to_string()),
            None
        );
    }

    #[test]
    #[cfg(unix)]
    fn override_validation_rejects_relative_missing_directory_and_non_executable_paths() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            validate_override(Runtime::Codex, "codex").unwrap_err().code,
            "not_absolute"
        );
        assert_eq!(
            validate_override(Runtime::Codex, "/definitely/missing/codex")
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            validate_override(Runtime::Codex, dir.path().to_str().unwrap())
                .unwrap_err()
                .code,
            "not_file"
        );
        let file = dir.path().join("codex");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        assert_eq!(
            validate_override(Runtime::Codex, file.to_str().unwrap())
                .unwrap_err()
                .code,
            "not_executable"
        );
    }

    #[test]
    #[cfg(unix)]
    fn cursor_detection_uses_only_cursor_agent_and_override_wins() {
        let pool = crate::db::open_in_memory().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let custom = executable(bin.path(), "agent");
        let path = bin.path().to_str().unwrap();
        let command = || {
            effective_runtime_command_on_path(
                Runtime::Cursor,
                &db::runtime_overrides(&pool).unwrap(),
                path,
                false,
            )
        };
        assert!(command().is_err());
        let detected = executable(bin.path(), "cursor-agent");
        assert_eq!(command().unwrap().command, detected.display().to_string());
        assert_eq!(command().unwrap().source, RuntimeCommandSource::Detected);
        db::set_runtime_override(&pool, "cursor", Some(custom.to_str().unwrap())).unwrap();
        assert_eq!(command().unwrap().command, custom.display().to_string());
        assert_eq!(command().unwrap().source, RuntimeCommandSource::Override);
        db::set_runtime_override(&pool, "cursor", None).unwrap();
        assert_eq!(command().unwrap().command, detected.display().to_string());
    }

    #[test]
    #[cfg(unix)]
    fn grok_detection_and_override_use_the_grok_executable() {
        let pool = crate::db::open_in_memory().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let custom = executable(bin.path(), "agent");
        let path = bin.path().to_str().unwrap();
        let command = || {
            effective_runtime_command_on_path(
                Runtime::Grok,
                &db::runtime_overrides(&pool).unwrap(),
                path,
                false,
            )
        };
        assert!(command().is_err());
        let detected = executable(bin.path(), "grok");
        assert_eq!(command().unwrap().command, detected.display().to_string());
        assert_eq!(command().unwrap().source, RuntimeCommandSource::Detected);
        db::set_runtime_override(&pool, "grok", Some(custom.to_str().unwrap())).unwrap();
        assert_eq!(command().unwrap().command, custom.display().to_string());
        assert_eq!(command().unwrap().source, RuntimeCommandSource::Override);
        db::set_runtime_override(&pool, "grok", None).unwrap();
        assert_eq!(command().unwrap().command, detected.display().to_string());
    }

    #[test]
    #[cfg(unix)]
    fn effective_command_precedence_and_stale_override_fallthrough() {
        let pool = crate::db::open_in_memory().unwrap();
        let detected_dir = tempfile::tempdir().unwrap();
        let override_dir = tempfile::tempdir().unwrap();
        let detected = executable(detected_dir.path(), "codex");
        let override_path = executable(override_dir.path(), "codex-custom");
        let shell_env = Arc::new(RwLock::new(crate::test_support::isolated_shell_env(Some(
            detected_dir.path(),
        ))));
        let discovery = completed_discovery();

        db::set_runtime_override(
            &pool,
            Runtime::Codex.key(),
            Some(override_path.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(
            effective_runtime_command(Runtime::Codex, &pool, &shell_env, &discovery)
                .unwrap()
                .command,
            override_path.display().to_string()
        );
        std::fs::remove_file(&override_path).unwrap();
        assert_eq!(
            effective_runtime_command(Runtime::Codex, &pool, &shell_env, &discovery)
                .unwrap()
                .command,
            detected.display().to_string()
        );
        assert_eq!(
            status_list(&pool, &shell_env, &discovery).unwrap().runtimes[0].state,
            RuntimeRowState::InvalidOverride
        );
    }

    #[test]
    fn pending_probe_allows_catalog_but_completed_missing_probe_errors() {
        assert_eq!(
            effective_runtime_command_on_path(
                Runtime::Codex,
                &Default::default(),
                "/definitely/missing",
                true,
            )
            .unwrap()
            .command,
            "codex"
        );
        let error = effective_runtime_command_on_path(
            Runtime::Codex,
            &Default::default(),
            "/definitely/missing",
            false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("Settings → Agents"));
        assert!(error.to_string().contains("Codex"));
    }

    #[test]
    fn failed_probe_keeps_prior_environment() {
        let pool = crate::db::open_in_memory().unwrap();
        let prior = LoginShellEnv {
            path: Some("/prior/bin".into()),
            vars: Default::default(),
        };
        let shell_env = Arc::new(RwLock::new(prior.clone()));
        let discovery = Arc::new(RwLock::new(DiscoveryState::pending()));
        apply_discovery_result(
            &pool,
            &shell_env,
            &discovery,
            DiscoveryResult {
                shell: Some("/bin/zsh".into()),
                outcome: DiscoveryOutcome::Timeout,
                duration_ms: 5_000,
                env: LoginShellEnv::default(),
            },
        )
        .unwrap();
        assert_eq!(*shell_env.read().unwrap(), prior);
        assert_eq!(
            discovery.read().unwrap().result.as_ref().unwrap().outcome,
            DiscoveryOutcome::Timeout
        );
    }

    #[test]
    fn registry_refresh_replaces_stale_environment_and_persisted_snapshot() {
        let pool = crate::db::open_in_memory().unwrap();
        let stale = LoginShellEnv {
            path: Some(r"C:\stale".into()),
            vars: Default::default(),
        };
        db::set_login_shell_env_lkg(
            &pool,
            &LoginShellEnvLkg {
                env: stale.clone(),
                shell: String::new(),
                captured_at: "2026-09-20T00:00:00Z".into(),
            },
        )
        .unwrap();
        let shell_env = Arc::new(RwLock::new(stale));
        let discovery = Arc::new(RwLock::new(DiscoveryState::startup(
            Some("/bin/stale-shell".into()),
            Some("2026-09-20T00:00:00Z".into()),
        )));
        let refreshed = LoginShellEnv {
            path: Some(r"C:\Windows\System32;C:\Users\Jason\bin".into()),
            vars: Default::default(),
        };

        apply_discovery_result(
            &pool,
            &shell_env,
            &discovery,
            DiscoveryResult {
                shell: None,
                outcome: DiscoveryOutcome::WindowsRegistry,
                duration_ms: 1,
                env: refreshed.clone(),
            },
        )
        .unwrap();

        assert_eq!(*shell_env.read().unwrap(), refreshed);
        assert_eq!(discovery.read().unwrap().seeded_shell, None);
        let snapshot = db::login_shell_env_lkg(&pool).unwrap().unwrap();
        assert_eq!(snapshot.env, refreshed);
        assert!(snapshot.shell.is_empty());
        assert_ne!(snapshot.captured_at, "2026-09-20T00:00:00Z");
    }

    #[test]
    #[cfg(unix)]
    fn opencode_v1_on_path_is_not_found_and_still_probes_until_upgraded_to_v2() {
        let pool = crate::db::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("opencode");
        std::fs::write(&bin, "#!/bin/sh\necho '1.19.5'\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let shell_env = Arc::new(RwLock::new(LoginShellEnv {
            path: Some(dir.path().display().to_string()),
            vars: Default::default(),
        }));
        let discovery = completed_discovery();
        let events = EventChannel::new();

        assert_eq!(
            versions::probe_now(Runtime::OpenCode, &pool, &shell_env, &discovery, &events)
                .as_deref(),
            Some("1.19.5")
        );
        let status = status_list(&pool, &shell_env, &discovery).unwrap();
        let row = status
            .runtimes
            .iter()
            .find(|runtime| runtime.name == Runtime::OpenCode)
            .unwrap();
        assert_eq!(row.state, RuntimeRowState::NotFound);
        assert_eq!(row.effective_command, None);
        assert_eq!(row.effective_source, None);
        assert!(row.below_minimum);
        assert_eq!(row.installed_version.as_deref(), Some("1.19.5"));
        assert_eq!(row.detected_path.as_deref(), Some(bin.to_str().unwrap()));

        let err = effective_runtime_command(Runtime::OpenCode, &pool, &shell_env, &discovery)
            .unwrap_err();
        let Error::RuntimeVersionGate(gate) = err else {
            panic!("expected RuntimeVersionGate, got {err}");
        };
        assert_eq!(gate.installed_version, "1.19.5");
        assert_eq!(gate.min_version, "2.1.0");

        // Candidate executable still probes after an in-place upgrade to v2.1+.
        std::fs::write(&bin, "#!/bin/sh\necho '2.1.0'\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            versions::probe_now(Runtime::OpenCode, &pool, &shell_env, &discovery, &events)
                .as_deref(),
            Some("2.1.0")
        );
        let status = status_list(&pool, &shell_env, &discovery).unwrap();
        let row = status
            .runtimes
            .iter()
            .find(|runtime| runtime.name == Runtime::OpenCode)
            .unwrap();
        assert_eq!(row.state, RuntimeRowState::Detected);
        assert_eq!(
            row.effective_command.as_deref(),
            Some(bin.to_str().unwrap())
        );
        assert_eq!(row.effective_source, Some(RuntimeCommandSource::Detected));
        assert!(!row.below_minimum);
        assert_eq!(row.installed_version.as_deref(), Some("2.1.0"));
    }

    #[test]
    #[cfg(unix)]
    fn opencode_v1_override_is_rejected_in_validation_and_status_list() {
        let pool = crate::db::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let v1 = dir.path().join("opencode-v1");
        std::fs::write(&v1, "#!/bin/sh\necho 'opencode 1.19.5'\n").unwrap();
        std::fs::set_permissions(&v1, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = validate_override(Runtime::OpenCode, v1.to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, "unsupported_version");
        assert!(err.message.contains("1.19.5"), "{}", err.message);
        assert!(
            err.message.contains("OpenCode v2 (2.1.0 or newer)"),
            "{}",
            err.message
        );

        db::set_runtime_override(&pool, "opencode", Some(v1.to_str().unwrap())).unwrap();
        let shell_env = Arc::new(RwLock::new(crate::test_support::isolated_shell_env(None)));
        let discovery = completed_discovery();
        let status = status_list(&pool, &shell_env, &discovery).unwrap();
        let row = status
            .runtimes
            .iter()
            .find(|runtime| runtime.name == Runtime::OpenCode)
            .unwrap();
        assert_eq!(row.state, RuntimeRowState::InvalidOverride);
        assert_eq!(row.effective_command, None);
        assert_eq!(row.effective_source, None);
        assert!(row.below_minimum);
        assert_eq!(row.installed_version.as_deref(), Some("1.19.5"));
        assert!(row
            .invalid_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("1.19.5")
                && reason.contains("OpenCode v2 (2.1.0 or newer)")));

        let v2 = dir.path().join("opencode-v2");
        std::fs::write(&v2, "#!/bin/sh\necho '2.1.0'\n").unwrap();
        std::fs::set_permissions(&v2, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(validate_override(Runtime::OpenCode, v2.to_str().unwrap()).is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn refresh_clears_cached_version_before_discovery_runs() {
        let dir = tempfile::tempdir().unwrap();
        let bin = executable(dir.path(), "opencode");
        let command = bin.to_str().unwrap();
        let pool = Arc::new(crate::db::open_fixture_pool(&dir.path().join("runner.db")).unwrap());
        let shell_env = Arc::new(RwLock::new(crate::test_support::isolated_shell_env(None)));
        let discovery = completed_discovery();
        discovery.write().unwrap().versions.record_override_probe(
            Runtime::OpenCode,
            command.into(),
            versions::executable_fingerprint(command),
            Some("2.0.29".into()),
        );
        assert_eq!(
            discovery
                .read()
                .unwrap()
                .versions
                .probed(Runtime::OpenCode, command),
            Some(Some("2.0.29"))
        );

        let (release, wait) = std::sync::mpsc::channel();
        let worker = refresh_background_discovery_with(
            EventChannel::new(),
            pool,
            shell_env,
            Arc::clone(&discovery),
            vec![Runtime::OpenCode],
            move || {
                let _ = wait.recv();
                DiscoveryResult {
                    shell: None,
                    outcome: DiscoveryOutcome::Ok,
                    duration_ms: 0,
                    env: LoginShellEnv {
                        path: None,
                        vars: [(TEST_ISOLATE_PATH_ENV.into(), "1".into())].into(),
                    },
                }
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            discovery
                .read()
                .unwrap()
                .versions
                .probed(Runtime::OpenCode, command),
            None
        );
        release.send(()).unwrap();
        worker.join().unwrap();
    }
}
