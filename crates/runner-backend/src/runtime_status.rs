use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::db::{self, DbPool, LoginShellEnvLkg};
use crate::error::{Error, Result};
use crate::events::EventChannel;
use crate::router::runtime::{runtime_definition, runtime_definitions};
use crate::session::launch;
use crate::shell_path::{DiscoveryOutcome, DiscoveryResult, DiscoveryState, LoginShellEnv};
use serde::Serialize;

pub type SharedShellEnv = Arc<RwLock<LoginShellEnv>>;
pub type SharedDiscoveryState = Arc<RwLock<DiscoveryState>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCommandSource {
    Override,
    Detected,
    Catalog,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveRuntimeCommand {
    pub command: String,
    pub source: RuntimeCommandSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeRowState {
    Detected,
    Override,
    NotFound,
    Checking,
    ProbeTimedOut,
    InvalidOverride,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShellDiscoveryStatus {
    pub shell: Option<String>,
    pub outcome: Option<DiscoveryOutcome>,
    pub duration_ms: Option<u64>,
    pub checking: bool,
    pub using_last_known_good: bool,
    pub last_known_good_captured_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeExecutableStatus {
    pub name: String,
    pub display_name: String,
    pub command: String,
    pub default_model: Option<String>,
    pub default_effort: Option<String>,
    pub detected_path: Option<String>,
    pub override_path: Option<String>,
    pub effective_command: Option<String>,
    pub effective_source: Option<RuntimeCommandSource>,
    pub state: RuntimeRowState,
    pub invalid_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeStatusResponse {
    pub shell: ShellDiscoveryStatus,
    pub runtimes: Vec<RuntimeExecutableStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverrideValidationError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for OverrideValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for OverrideValidationError {}

pub fn status_list(
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
) -> Result<RuntimeStatusResponse> {
    let shell_env = shell_env
        .read()
        .map_err(|_| Error::msg("runtime shell environment lock poisoned"))?
        .clone();
    let discovery = discovery
        .read()
        .map_err(|_| Error::msg("runtime discovery lock poisoned"))?
        .clone();
    let overrides = db::runtime_overrides(pool)?;
    let path = direct_chat_path(&shell_env);
    let home = runner_core::app_paths::home_dir();

    let runtimes = runtime_definitions()
        .iter()
        .map(|runtime| {
            let defaults = home
                .as_deref()
                .map(|home| crate::runtime_defaults::runtime_defaults(runtime.name, home))
                .unwrap_or_default();
            let detected_path =
                find_executable(runtime.command, &path).map(|path| path.display().to_string());
            let override_path = overrides.get(runtime.name).cloned();
            let invalid_reason = override_path
                .as_deref()
                .and_then(|path| validate_executable_path(Path::new(path)).err())
                .map(|error| error.message);
            let valid_override = override_path
                .as_deref()
                .filter(|_| invalid_reason.is_none())
                .map(str::to_string);
            let (effective_command, effective_source) = if let Some(path) = valid_override {
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
            } else if discovery.checking {
                RuntimeRowState::Checking
            } else if discovery
                .result
                .as_ref()
                .is_some_and(|result| result.outcome != DiscoveryOutcome::Ok)
            {
                RuntimeRowState::ProbeTimedOut
            } else if detected_path.is_some() {
                RuntimeRowState::Detected
            } else {
                RuntimeRowState::NotFound
            };
            RuntimeExecutableStatus {
                name: runtime.name.to_string(),
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
            }
        })
        .collect();

    let result = discovery.result.as_ref();
    let failed = result.is_some_and(|result| result.outcome != DiscoveryOutcome::Ok);
    Ok(RuntimeStatusResponse {
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

pub fn effective_runtime_command(
    runtime: &str,
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

fn effective_runtime_command_on_path(
    runtime: &str,
    overrides: &std::collections::BTreeMap<String, String>,
    path: &str,
    checking: bool,
) -> Result<EffectiveRuntimeCommand> {
    let definition = runtime_definition(runtime)
        .ok_or_else(|| Error::msg(format!("unknown runtime: {runtime}")))?;
    if let Some(path) = overrides.get(runtime) {
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

pub fn runtime_not_found_error(runtime: &str) -> Error {
    let name = runtime_definition(runtime)
        .map(|definition| definition.display_name)
        .unwrap_or(runtime);
    Error::msg(format!(
        "{name} executable was not found. Open Settings → Agents to refresh discovery or set an override."
    ))
}

pub fn validate_override(
    runtime: &str,
    path: &str,
) -> std::result::Result<(), OverrideValidationError> {
    if runtime_definition(runtime).is_none() {
        return Err(validation_error(
            "unknown_runtime",
            format!("Unknown runtime: {runtime}."),
        ));
    }
    validate_executable_path(Path::new(path))
}

fn validate_executable_path(path: &Path) -> std::result::Result<(), OverrideValidationError> {
    if !path.is_absolute() {
        return Err(validation_error(
            "not_absolute",
            "Choose an absolute executable path.",
        ));
    }
    let metadata = std::fs::metadata(path)
        .map_err(|_| validation_error("not_found", "File does not exist."))?;
    if !metadata.is_file() {
        return Err(validation_error("not_file", "Not a regular file."));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(validation_error(
                "not_executable",
                "Not an executable file.",
            ));
        }
    }
    Ok(())
}

pub(crate) fn executable_path_is_valid(path: &Path) -> bool {
    validate_executable_path(path).is_ok()
}

fn validation_error(code: &str, message: impl Into<String>) -> OverrideValidationError {
    OverrideValidationError {
        code: code.to_string(),
        message: message.into(),
    }
}

pub fn direct_chat_path(shell_env: &LoginShellEnv) -> String {
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

pub fn find_executable(command: &str, path: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    for entry in std::env::split_paths(path).filter(|entry| !entry.as_os_str().is_empty()) {
        #[cfg(windows)]
        for extension in extensions
            .split(';')
            .filter(|extension| !extension.is_empty())
        {
            let candidate = entry.join(format!("{command}{extension}"));
            if validate_executable_path(&candidate).is_ok() {
                return Some(candidate);
            }
        }
        #[cfg(windows)]
        if !Path::new(command)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extensions.split(';').any(|suffix| {
                    suffix
                        .strip_prefix('.')
                        .is_some_and(|suffix| extension.eq_ignore_ascii_case(suffix))
                })
            })
        {
            continue;
        }
        let candidate = entry.join(command);
        if validate_executable_path(&candidate).is_ok() {
            return Some(candidate);
        }
    }
    None
}

pub fn apply_discovery_result(
    pool: &DbPool,
    shell_env: &SharedShellEnv,
    discovery: &SharedDiscoveryState,
    result: DiscoveryResult,
) -> Result<()> {
    let mut persistence_error = None;
    let captured_at = if result.outcome == DiscoveryOutcome::Ok {
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
    state.seeded_shell = result.shell.clone().or_else(|| state.seeded_shell.clone());
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

pub fn start_background_discovery(
    events: EventChannel,
    pool: Arc<DbPool>,
    shell_env: SharedShellEnv,
    discovery: SharedDiscoveryState,
) {
    std::thread::spawn(move || {
        let result = crate::shell_path::resolve_login_shell_env();
        if let Err(error) = apply_discovery_result(&pool, &shell_env, &discovery, result) {
            log::warn!("runtime discovery state update failed: {error}");
            if let Ok(mut state) = discovery.write() {
                state.checking = false;
            }
        }
        log_runtime_paths(&pool, &shell_env);
        events.emit("runtime/changed", &());
    });
}

pub fn refresh_background_discovery(
    events: EventChannel,
    pool: Arc<DbPool>,
    shell_env: SharedShellEnv,
    discovery: SharedDiscoveryState,
) -> Result<bool> {
    {
        let mut state = discovery
            .write()
            .map_err(|_| Error::msg("runtime discovery lock poisoned"))?;
        if state.checking {
            return Ok(false);
        }
        state.checking = true;
    }
    events.emit("runtime/changed", &());
    start_background_discovery(events, pool, shell_env, discovery);
    Ok(true)
}

fn log_runtime_paths(pool: &DbPool, shell_env: &SharedShellEnv) {
    let Ok(shell_env) = shell_env.read() else {
        return;
    };
    let path = direct_chat_path(&shell_env);
    for runtime in runtime_definitions() {
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
    use std::os::unix::fs::PermissionsExt;

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
        }))
    }

    #[test]
    fn status_list_includes_all_catalog_runtimes() {
        let pool = crate::db::open_in_memory().unwrap();
        let shell_env = Arc::new(RwLock::new(LoginShellEnv::default()));
        let status = status_list(&pool, &shell_env, &completed_discovery()).unwrap();
        assert_eq!(
            status
                .runtimes
                .iter()
                .map(|runtime| (
                    runtime.name.as_str(),
                    runtime.display_name.as_str(),
                    runtime.command.as_str(),
                ))
                .collect::<Vec<_>>(),
            vec![
                ("codex", "Codex", "codex"),
                ("claude-code", "Claude Code", "claude"),
                ("trae", "TRAE CLI", "traecli"),
            ],
        );
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
    fn override_validation_rejects_relative_missing_directory_and_non_executable_paths() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            validate_override("codex", "codex").unwrap_err().code,
            "not_absolute"
        );
        assert_eq!(
            validate_override("codex", "/definitely/missing/codex")
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            validate_override("codex", dir.path().to_str().unwrap())
                .unwrap_err()
                .code,
            "not_file"
        );
        let file = dir.path().join("codex");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        assert_eq!(
            validate_override("codex", file.to_str().unwrap())
                .unwrap_err()
                .code,
            "not_executable"
        );
    }

    #[test]
    fn effective_command_precedence_and_stale_override_fallthrough() {
        let pool = crate::db::open_in_memory().unwrap();
        let detected_dir = tempfile::tempdir().unwrap();
        let override_dir = tempfile::tempdir().unwrap();
        let detected = executable(detected_dir.path(), "codex");
        let override_path = executable(override_dir.path(), "codex-custom");
        let shell_env = Arc::new(RwLock::new(LoginShellEnv {
            path: Some(detected_dir.path().display().to_string()),
            vars: Default::default(),
        }));
        let discovery = completed_discovery();

        db::set_runtime_override(&pool, "codex", Some(override_path.to_str().unwrap())).unwrap();
        assert_eq!(
            effective_runtime_command("codex", &pool, &shell_env, &discovery)
                .unwrap()
                .command,
            override_path.display().to_string()
        );
        std::fs::remove_file(&override_path).unwrap();
        assert_eq!(
            effective_runtime_command("codex", &pool, &shell_env, &discovery)
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
                "codex",
                &Default::default(),
                "/definitely/missing",
                true,
            )
            .unwrap()
            .command,
            "codex"
        );
        let error = effective_runtime_command_on_path(
            "codex",
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
}
