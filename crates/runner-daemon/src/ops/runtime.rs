use crate::model::Runtime;
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::runtime_status::{OverrideValidationError, RuntimeCommandSource, RuntimeStatusResponse};
use crate::AppCore;

pub use runner_core::protocol::runtime::RuntimeDefinition;

pub use runner_core::protocol::runtime::RuntimeCatalogOption;

pub use runner_core::protocol::runtime::RuntimeCatalogEntry;

pub fn runtime_list() -> Vec<RuntimeDefinition> {
    crate::runtimes::catalogs()
        .iter()
        .map(|runtime| RuntimeDefinition {
            name: runtime.name,
            display_name: runtime.display_name.to_string(),
            command: runtime.command.to_string(),
            native_fork: runtime.native_fork,
        })
        .collect()
}

use runner_core::protocol::runtime::{RuntimeRowState, RuntimeSignInStatus};
use runner_core::protocol::usage::UnavailableReason;

pub(crate) fn runtime_status_list_base(state: &AppCore) -> Result<RuntimeStatusResponse> {
    let status = crate::runtime_status::status_list_with_events(
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
        Some(&state.events),
    )?;
    for runtime in &status.runtimes {
        if runtime.installed_version.is_some() && !runtime.below_minimum {
            state
                .sessions
                .clear_obsolete_startup_errors(runtime.name, &state.session_events());
        }
    }
    Ok(status)
}

pub fn runtime_status_list(state: &AppCore) -> Result<RuntimeStatusResponse> {
    let mut response = runtime_status_list_base(state)?;
    populate_sign_in_statuses(state, &mut response);
    Ok(response)
}

fn populate_sign_in_statuses(state: &AppCore, response: &mut RuntimeStatusResponse) {
    let shell_env = state
        .runtime_shell_env
        .read()
        .map(|env| env.clone())
        .unwrap_or_default();
    let usage = state.usage.snapshot();
    for entry in &mut response.runtimes {
        let installed = matches!(
            entry.state,
            RuntimeRowState::Detected | RuntimeRowState::Override
        );
        if !installed {
            entry.sign_in = RuntimeSignInStatus::Unknown;
            continue;
        }
        entry.sign_in = match entry.name {
            Runtime::ClaudeCode => claude_sign_in_status(state, &shell_env.vars, &usage),
            Runtime::Codex => codex_sign_in_status(&shell_env.vars, &usage),
            Runtime::Antigravity => antigravity_sign_in_status(&shell_env, &usage),
            _ => RuntimeSignInStatus::Unknown,
        };
    }
}

fn usage_sign_in_status(
    usage: &runner_core::protocol::UsageSnapshot,
    runtime: Runtime,
) -> Option<RuntimeSignInStatus> {
    let entry = usage.runtimes.get(&runtime)?;
    if entry.value.is_some() {
        Some(RuntimeSignInStatus::SignedIn)
    } else if entry.error == Some(UnavailableReason::SignIn) {
        Some(RuntimeSignInStatus::SignInNeeded)
    } else {
        None
    }
}

fn claude_sign_in_status(
    state: &AppCore,
    shell_vars: &std::collections::BTreeMap<String, String>,
    usage: &runner_core::protocol::UsageSnapshot,
) -> RuntimeSignInStatus {
    match crate::claude_accounts::active_sign_in_status(state, shell_vars) {
        Some(RuntimeSignInStatus::Unknown) => {
            // An earlier `claude auth status` probe errored and was cached;
            // avoid re-spawning it whenever `runtime/changed` reloads the list.
            usage_sign_in_status(usage, Runtime::ClaudeCode).unwrap_or(RuntimeSignInStatus::Unknown)
        }
        Some(status) => status,
        None => {
            if let Some(status) = usage_sign_in_status(usage, Runtime::ClaudeCode) {
                return status;
            }
            // Kick off a single background `claude auth status` check on native
            // single-account setups when no cached sign-in status is available yet.
            crate::claude_accounts::refresh(state);
            RuntimeSignInStatus::Unknown
        }
    }
}

fn codex_sign_in_status(
    shell_vars: &std::collections::BTreeMap<String, String>,
    usage: &runner_core::protocol::UsageSnapshot,
) -> RuntimeSignInStatus {
    if shell_vars
        .get("OPENAI_API_KEY")
        .is_some_and(|value| !value.trim().is_empty())
    {
        return RuntimeSignInStatus::SignedIn;
    }
    if let Some(status) = usage_sign_in_status(usage, Runtime::Codex) {
        return status;
    }
    let codex_home = shell_vars
        .get("CODEX_HOME")
        .filter(|value| !value.trim().is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            shell_vars
                .get("HOME")
                .filter(|value| !value.trim().is_empty())
                .map(|home| std::path::Path::new(home).join(".codex"))
        })
        .or_else(|| runner_core::app_paths::home_dir().map(|home| home.join(".codex")));
    if let Some(codex_home) = codex_home {
        if let Ok(raw) = std::fs::read_to_string(codex_home.join("auth.json")) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
                let has_key = json
                    .get("OPENAI_API_KEY")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty());
                let has_tokens = json
                    .get("tokens")
                    .and_then(serde_json::Value::as_object)
                    .is_some_and(|tokens| {
                        ["access_token", "refresh_token", "id_token"]
                            .into_iter()
                            .any(|field| {
                                tokens
                                    .get(field)
                                    .and_then(serde_json::Value::as_str)
                                    .is_some_and(|value| !value.trim().is_empty())
                            })
                    });
                if has_key || has_tokens {
                    return RuntimeSignInStatus::SignedIn;
                }
            }
        }
    }
    RuntimeSignInStatus::Unknown
}

fn antigravity_sign_in_status(
    shell_env: &crate::shell_path::LoginShellEnv,
    usage: &runner_core::protocol::UsageSnapshot,
) -> RuntimeSignInStatus {
    if let Some(status) = usage_sign_in_status(usage, Runtime::Antigravity) {
        return status;
    }
    if crate::runtimes::antigravity::usage::has_stored_credential(shell_env) {
        return RuntimeSignInStatus::SignedIn;
    }
    RuntimeSignInStatus::Unknown
}

pub fn runtime_set_launch_option(
    state: &AppCore,
    runtime: Runtime,
    option: runner_core::protocol::runtime::RuntimeLaunchOption,
    enabled: bool,
) -> Result<RuntimeStatusResponse> {
    if !option.supported(runtime) {
        return Err(Error::msg("launch option is not supported by this runtime"));
    }
    crate::db::set_runtime_launch_option(&state.db, runtime, option, enabled)?;
    state.events.emit("runtime/changed", &());
    runtime_status_list(state)
}

pub fn runtime_command_preview(
    state: &AppCore,
    runtime: Runtime,
    mission: bool,
) -> Result<runner_core::protocol::runtime::RuntimeCommandPreview> {
    let command = runtime_status_list(state)?
        .runtimes
        .into_iter()
        .find(|entry| entry.name == runtime)
        .map(|entry| entry.effective_command.unwrap_or(entry.command))
        .ok_or_else(|| Error::msg(format!("unknown runtime: {runtime}")))?;
    let options = crate::db::runtime_launch_options(&state.db)?
        .get(runtime.key())
        .cloned()
        .unwrap_or_default();
    let env = state
        .runtime_shell_env
        .read()
        .map_err(|_| Error::msg("runtime shell environment lock poisoned"))?
        .vars
        .clone();
    let mut preview = state.sessions.command_preview(
        runtime,
        &command,
        mission,
        &options,
        &state.app_data_dir,
        env,
    );
    let words = crate::runtimes::custom_flags::parse_lossy(&options.custom_flags);
    if !words.is_empty() {
        let listed = crate::runtime_status::versions::probe_help_flags(
            runtime,
            &state.db,
            &state.runtime_shell_env,
            &state.runtime_discovery,
        );
        preview.custom_flags_warning =
            crate::runtimes::custom_flags::help_flag_warning(runtime, &words, listed.as_ref());
    }
    if runtime == Runtime::ClaudeCode {
        let accounts = crate::claude_accounts::load(&state.db)?;
        if let Some(account) = accounts.accounts.iter().find(|account| {
            accounts.accounts.len() > 1 && account.id == accounts.active_id && !account.native
        }) {
            preview.lines.insert(
                0,
                runner_core::protocol::runtime::RuntimeCommandLine {
                    command: format!("CLAUDE_CONFIG_DIR={}", account.folder),
                    note: runner_core::protocol::runtime::RuntimeCommandNote::ActiveAccount {
                        name: account.name.clone(),
                    },
                    option: None,
                },
            );
        }
    }
    Ok(preview)
}

pub fn runtime_set_override(
    state: &AppCore,
    runtime: Runtime,
    path: &str,
) -> std::result::Result<RuntimeStatusResponse, OverrideValidationError> {
    let path = path.trim();
    if crate::runtimes::adapter(runtime).catalog().is_none() {
        return Err(OverrideValidationError {
            code: "unknown_runtime".into(),
            message: format!("Unknown runtime: {runtime}."),
        });
    }
    let previous = crate::runtime_status::versions::installed_command(
        runtime,
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
    );
    if path.is_empty() {
        crate::db::set_runtime_override(&state.db, runtime.key(), None)
            .map_err(persistence_error)?;
        if let Ok(mut discovery) = state.runtime_discovery.write() {
            discovery.versions.clear_runtime_probe(runtime);
        }
    } else {
        let shell_env = state
            .runtime_shell_env
            .read()
            .map(|env| env.clone())
            .unwrap_or_default();
        let validated =
            crate::runtime_status::validate_and_probe_override(runtime, path, &shell_env)?;
        crate::db::set_runtime_override(&state.db, runtime.key(), Some(path))
            .map_err(persistence_error)?;
        if let Some(validated) = validated {
            if let Ok(mut discovery) = state.runtime_discovery.write() {
                discovery.versions.record_override_probe(
                    runtime,
                    path.to_owned(),
                    validated.fingerprint,
                    validated.version.clone(),
                );
            }
            let min_version = crate::runtimes::adapter(runtime)
                .catalog()
                .and_then(|catalog| catalog.min_version);
            if validated.version.is_some()
                && !crate::runtime_status::versions::below_minimum(
                    validated.version.as_deref(),
                    min_version,
                )
            {
                state
                    .sessions
                    .clear_obsolete_startup_errors(runtime, &state.session_events());
            }
        }
        log::info!("runtime override saved: runtime={runtime} path={path}");
    }
    invalidate_override_help_flags(state, runtime, previous.as_deref());
    state.events.emit("runtime/changed", &());
    request_version_probe(state, runtime);
    runtime_status_list(state).map_err(persistence_error)
}

pub fn runtime_set_custom_flags(
    state: &AppCore,
    runtime: Runtime,
    raw_flags: &str,
) -> std::result::Result<
    runner_core::protocol::runtime::RuntimeCustomFlagsResponse,
    runner_core::protocol::runtime::CustomFlagsValidationError,
> {
    use runner_core::protocol::runtime::{CustomFlagsValidationError, RuntimeCustomFlagsResponse};
    if crate::runtimes::adapter(runtime).catalog().is_none() {
        return Err(CustomFlagsValidationError {
            code: "unknown_runtime".into(),
            message: format!("Unknown runtime: {runtime}."),
        });
    }
    let (words, warning) =
        crate::runtimes::custom_flags::validate_save(runtime, raw_flags, || {
            crate::runtime_status::versions::probe_help_flags(
                runtime,
                &state.db,
                &state.runtime_shell_env,
                &state.runtime_discovery,
            )
        })?;
    let to_save = if words.is_empty() { "" } else { raw_flags };
    crate::db::set_runtime_custom_flags(&state.db, runtime, to_save).map_err(|error| {
        CustomFlagsValidationError {
            code: "persistence_failed".into(),
            message: error.to_string(),
        }
    })?;
    log::info!(
        "runtime custom flags updated: runtime={runtime} empty={} warning={}",
        to_save.is_empty(),
        warning.is_some()
    );
    state.events.emit("runtime/changed", &());
    let status = runtime_status_list(state).map_err(|error| CustomFlagsValidationError {
        code: "persistence_failed".into(),
        message: error.to_string(),
    })?;
    Ok(RuntimeCustomFlagsResponse { status, warning })
}

pub fn runtime_clear_override(state: &AppCore, runtime: Runtime) -> Result<RuntimeStatusResponse> {
    if crate::runtimes::adapter(runtime).catalog().is_none() {
        return Err(Error::msg(format!("unknown runtime: {runtime}")));
    }
    let previous = crate::runtime_status::versions::installed_command(
        runtime,
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
    );
    crate::db::set_runtime_override(&state.db, runtime.key(), None)?;
    if let Ok(mut discovery) = state.runtime_discovery.write() {
        discovery.versions.clear_runtime_probe(runtime);
    }
    invalidate_override_help_flags(state, runtime, previous.as_deref());
    log::info!("runtime override cleared: runtime={runtime}");
    state.events.emit("runtime/changed", &());
    request_version_probe(state, runtime);
    runtime_status_list(state)
}

fn invalidate_override_help_flags(state: &AppCore, runtime: Runtime, previous: Option<&str>) {
    if let Some(command) = previous {
        crate::runtime_status::versions::clear_help_flags(&state.runtime_discovery, Some(command));
    }
    if let Some(current) = crate::runtime_status::versions::installed_command(
        runtime,
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
    ) {
        if previous != Some(current.as_str()) {
            crate::runtime_status::versions::clear_help_flags(
                &state.runtime_discovery,
                Some(&current),
            );
        }
    }
}

fn request_version_probe(state: &AppCore, runtime: Runtime) {
    crate::runtime_status::versions::request_probes(
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
        &state.events,
        &[runtime],
        false,
    );
}

/// Probes one runtime's installed version on the calling thread, after its
/// update exits, and returns it. The Agents pane hears `runtime/changed`.
pub fn runtime_probe_version(state: &AppCore, runtime: Runtime) -> Option<String> {
    crate::runtime_status::versions::probe_now(
        runtime,
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
        &state.events,
    )
}

/// Checks each installed, updatable runtime's latest-version source (npm
/// registry or HTTPS release manifest) off the calling thread. Answers are
/// cached for six hours per app run; `force` skips the cache. Failures leave
/// the rows showing versions only. Installed versions are refreshed even when
/// remote answers are cached.
pub fn runtime_check_updates(state: &AppCore, force: bool) {
    crate::runtime_status::versions::request_latest(
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
        &state.events,
        force,
    );
}

/// argv, env and cwd for a global npm install of a runtime (#1171).
pub fn runtime_install_spawn_spec(
    state: &AppCore,
    runtime: Runtime,
    size: (u16, u16),
) -> Result<crate::session::runtime::SpawnSpec> {
    let package = crate::runtimes::adapter(runtime)
        .catalog()
        .and_then(|definition| definition.npm_package)
        .ok_or_else(|| Error::msg(format!("{runtime} cannot be installed via npm")))?;
    let shell_env = state
        .runtime_shell_env
        .read()
        .map_err(|_| Error::msg("runtime shell environment lock poisoned"))?;
    let path = crate::runtime_status::direct_chat_path(&shell_env);
    let npm = crate::runtime_status::find_executable("npm", &path)
        .ok_or_else(|| Error::msg("npm was not found on PATH; install Node.js first"))?;
    Ok(state.sessions.update_spawn_spec(
        npm.display().to_string(),
        vec!["install".into(), "-g".into(), package.to_owned()],
        runner_core::app_paths::home_dir(),
        size,
    ))
}

/// argv, env and cwd for a runtime's own update (#533): the effective
/// executable with the runtime's update arguments, the agent environment
/// without Runner's layers, and the home directory.
pub fn runtime_update_spawn_spec(
    state: &AppCore,
    runtime: Runtime,
    size: (u16, u16),
) -> Result<crate::session::runtime::SpawnSpec> {
    let definition = crate::runtimes::adapter(runtime)
        .catalog()
        .filter(|definition| !definition.update_args.is_empty())
        .ok_or_else(|| Error::msg(format!("{runtime} has no update command")))?;
    let command = crate::runtime_status::effective_runtime_command_with_events(
        runtime,
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
        Some(&state.events),
    )?;
    if command.source == RuntimeCommandSource::Catalog {
        return Err(crate::runtime_status::runtime_not_found_error(runtime));
    }
    Ok(state.sessions.update_spawn_spec(
        command.command,
        definition
            .update_args
            .iter()
            .map(|arg| (*arg).to_owned())
            .collect(),
        runner_core::app_paths::home_dir(),
        size,
    ))
}

/// Starts the update PTY `runtime_update_spawn_spec` described. It is not a
/// listed session; its terminal is attached by id.
pub fn runtime_update_start(
    state: &AppCore,
    spec: crate::session::runtime::SpawnSpec,
    events: Arc<dyn crate::session::manager::SessionEvents>,
) -> Result<std::sync::mpsc::Receiver<Option<i32>>> {
    log::info!(
        "runtime update started: session={} command={} args={:?}",
        spec.session_id,
        spec.command,
        spec.args
    );
    state.sessions.spawn_unlisted(spec, &state.db, events)
}

/// Requests model catalogs in the background, honoring the cache TTL.
pub fn runtime_request_models(state: &AppCore, runtimes: &[Runtime]) {
    request_models(state, runtimes, false);
}

/// Refreshes model catalogs after an explicit Start Chat action, bypassing
/// the cache TTL. Queries coalesce with one already in flight.
pub fn runtime_refresh_models(state: &AppCore, runtimes: &[Runtime]) {
    request_models(state, runtimes, true);
}

fn request_models(state: &AppCore, runtimes: &[Runtime], force: bool) {
    // A source is only identifiable once executable discovery has resolved
    // the launch environment. Until then the persisted catalogs still publish.
    let ready = state
        .runtime_discovery
        .read()
        .is_ok_and(|discovery| !discovery.checking && discovery.result.is_some());
    if !ready {
        crate::runtime_status::models::load_cached(
            &state.db,
            &state.runtime_discovery,
            &state.events,
        );
        let queued = state
            .runtime_discovery
            .write()
            .map_or(true, |mut discovery| {
                if discovery.checking || discovery.result.is_none() {
                    if force {
                        discovery.models.queue_refresh(runtimes);
                    }
                    true
                } else {
                    false
                }
            });
        if queued {
            return;
        }
    }
    crate::runtime_status::models::request(
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
        &state.events,
        runtimes,
        force,
    );
}

/// Re-runs executable discovery for every runtime, and model discovery for
/// the enabled runtimes the caller passes.
pub fn runtime_refresh(
    state: &AppCore,
    model_runtimes: &[Runtime],
) -> Result<RuntimeStatusResponse> {
    crate::runtime_status::refresh_background_discovery(
        state.events.clone(),
        Arc::clone(&state.db),
        Arc::clone(&state.runtime_shell_env),
        Arc::clone(&state.runtime_discovery),
        model_runtimes.to_vec(),
    )?;
    runtime_check_updates(state, true);
    let status = runtime_status_list(state)?;
    if status.runtimes.iter().any(|entry| {
        entry.name == Runtime::ClaudeCode
            && matches!(
                entry.state,
                RuntimeRowState::Detected | RuntimeRowState::Override
            )
    }) {
        crate::claude_accounts::refresh(state);
    }
    Ok(status)
}

/// Whether a runtime is one of the agents Runner ships enabled by default.
/// Startup reads it to decide which model catalogs may be queried before any
/// surface is open.
pub fn runtime_default_enabled(runtime: Runtime) -> bool {
    runtime_catalog_options()
        .iter()
        .any(|entry| entry.name == runtime && entry.default_enabled)
}

/// The runtimes whose models Runner can discover at all.
pub fn model_discovery_runtimes() -> Vec<Runtime> {
    crate::runtime_status::models::discovery_runtimes()
}

pub fn runtime_catalog(state: &AppCore) -> Result<Vec<RuntimeCatalogEntry>> {
    let statuses = runtime_status_list(state)?;
    let statuses: HashMap<_, _> = statuses
        .runtimes
        .into_iter()
        .map(|runtime| (runtime.name, runtime))
        .collect();
    let shell_env = state
        .runtime_shell_env
        .read()
        .map_err(|_| Error::msg("runtime shell environment lock poisoned"))?;
    let discovery = state
        .runtime_discovery
        .read()
        .map_err(|_| Error::msg("runtime discovery lock poisoned"))?;
    Ok(runtime_catalog_options()
        .into_iter()
        .map(|mut runtime| {
            if let Some(status) = statuses.get(&runtime.name) {
                runtime.available = matches!(
                    status.effective_source,
                    Some(RuntimeCommandSource::Detected | RuntimeCommandSource::Override)
                );
                runtime.default_model.clone_from(&status.default_model);
                runtime.default_effort.clone_from(&status.default_effort);
                runtime
                    .installed_version
                    .clone_from(&status.installed_version);
                runtime
                    .available_version
                    .clone_from(&status.available_version);
                runtime.min_version.clone_from(&status.min_version);
                runtime.below_minimum = status.below_minimum;
                if let Some(catalog) = status.effective_command.as_deref().and_then(|command| {
                    discovery.models.catalog(
                        runtime.name,
                        &crate::runtime_status::models::source_with_main(
                            runtime.name,
                            command,
                            &shell_env,
                            Some(&state.db),
                        ),
                    )
                }) {
                    if let Some(efforts) =
                        crate::runtimes::adapter(runtime.name).discovered_efforts(catalog)
                    {
                        runtime.efforts = efforts;
                    }
                    runtime.models = std::iter::once(default_model_option())
                        .chain(catalog.models.iter().cloned())
                        .collect();
                    if runtime.default_model.is_none() {
                        runtime.default_model.clone_from(&catalog.default_model);
                    }
                }
            }
            runtime
        })
        .collect())
}

pub use runner_core::protocol::runtime::filter_selectable_runtime_catalog;

use crate::runtimes::catalog::default_model_option;

fn runtime_catalog_options() -> Vec<RuntimeCatalogEntry> {
    crate::runtimes::catalogs()
        .into_iter()
        .map(crate::runtimes::RuntimeCatalog::into_entry)
        .collect()
}

#[cfg(all(test, unix))]
#[test]
fn catalog_golden() {
    crate::golden::assert_golden(
        "catalog",
        serde_json::to_value(runtime_catalog_options()).unwrap(),
    );
}

fn persistence_error(error: Error) -> OverrideValidationError {
    OverrideValidationError {
        code: "persistence_failed".into(),
        message: error.to_string(),
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
