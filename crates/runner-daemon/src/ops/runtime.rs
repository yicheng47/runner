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

pub fn runtime_status_list(state: &AppCore) -> Result<RuntimeStatusResponse> {
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

/// Asks npm whether a newer version of each installed, updatable runtime
/// exists, off the calling thread. Answers are cached for six hours per app
/// run; `force` skips the cache. Failures leave the rows showing versions
/// only. Installed versions are refreshed even when registry answers are cached.
pub fn runtime_check_updates(state: &AppCore, force: bool) {
    crate::runtime_status::versions::request_latest(
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
        &state.events,
        force,
    );
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
    runtime_status_list(state)
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
mod tests {
    use super::*;

    #[test]
    fn effort_filter_keeps_default_and_falls_back_for_unknown_capabilities() {
        let mut runtime = runtime_catalog_options().remove(0);
        let model = runtime.models[1].value.clone();
        let values = |runtime: &RuntimeCatalogEntry, model: &str| {
            runtime
                .efforts_for_model(model)
                .into_iter()
                .map(|option| option.value)
                .collect::<Vec<_>>()
        };
        let fallback = values(&runtime, &model);
        assert_eq!(values(&runtime, ""), [""]);
        runtime.models[1].supported_efforts = Some(vec!["low".into(), "high".into()]);
        assert_eq!(values(&runtime, &model), ["", "low", "high"]);
        assert_eq!(values(&runtime, "custom-model"), fallback);
        runtime.default_model = Some(model.clone());
        assert_eq!(values(&runtime, ""), [""]);
        assert_eq!(values(&runtime, "  "), [""]);
        runtime.models[1].supported_efforts = Some(Vec::new());
        assert_eq!(values(&runtime, &model), [""]);
        runtime.models[1].supported_efforts = None;
        assert_eq!(values(&runtime, &model), fallback);
    }

    #[test]
    fn catalog_matches_supported_runtime_order_and_defaults() {
        let definitions = runtime_list();
        let pi = definitions
            .iter()
            .find(|runtime| runtime.name == Runtime::Pi)
            .unwrap();
        assert_eq!(pi.command, "pi");
        assert!(pi.native_fork);
        let agy = definitions
            .iter()
            .find(|runtime| runtime.name == Runtime::Antigravity)
            .unwrap();
        assert_eq!(agy.display_name, "Antigravity CLI");
        assert_eq!(agy.command, "agy");
        assert!(!agy.native_fork);

        let catalog = runtime_catalog_options();
        let expected = [
            Runtime::Codex,
            Runtime::ClaudeCode,
            Runtime::Antigravity,
            Runtime::Pi,
            Runtime::Copilot,
            Runtime::Trae,
            Runtime::Cursor,
            Runtime::Grok,
            Runtime::OpenCode,
        ];
        assert_eq!(
            definitions
                .iter()
                .map(|runtime| runtime.name)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(&Runtime::ALL[..9], &expected);
        assert_eq!(
            catalog
                .iter()
                .map(|runtime| runtime.name)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(catalog[0].default_enabled);
        assert!(catalog[1].default_enabled);
        assert!(catalog[2].default_enabled);
        assert_eq!(catalog[4].models[1].value, "auto");
        assert_eq!(catalog[4].models.len(), 28);
        assert!(catalog[4].default_enabled);
        assert!(catalog[3].default_enabled);
        assert!(catalog[3].native_fork);
        assert_eq!(catalog[3].models.len(), 1);
        assert_eq!(
            catalog[3]
                .efforts
                .iter()
                .map(|effort| effort.value.as_str())
                .collect::<Vec<_>>(),
            ["", "off", "minimal", "low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(
            catalog[4]
                .efforts
                .iter()
                .map(|effort| effort.value.as_str())
                .collect::<Vec<_>>(),
            ["", "none", "minimal", "low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(
            catalog[0]
                .models
                .iter()
                .map(|model| model.value.as_str())
                .collect::<Vec<_>>(),
            [
                "",
                "gpt-6-astra",
                "gpt-5.6-sol",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "gpt-5.5",
                "gpt-5.4",
                "gpt-5.4-mini",
                "gpt-5.3-codex-spark",
            ]
        );
        assert_eq!(
            catalog[0]
                .efforts
                .iter()
                .map(|effort| effort.value.as_str())
                .collect::<Vec<_>>(),
            ["", "low", "medium", "high", "xhigh", "max", "ultra"]
        );
        assert_eq!(
            catalog[5]
                .efforts
                .iter()
                .map(|effort| effort.value.as_str())
                .collect::<Vec<_>>(),
            ["", "low", "medium", "high", "xhigh"]
        );

        let agy = &catalog[2];
        assert_eq!(agy.command, "agy");
        assert!(!agy.native_fork);
        assert!(agy.default_enabled);
        assert_eq!(
            agy.models
                .iter()
                .map(|model| model.value.as_str())
                .collect::<Vec<_>>(),
            [
                "",
                "gemini-4-carbon",
                "gemini-4-argon",
                "gemini-4-hydrogen",
                "gemini-pro-orchestrator",
                "gemini-pro-orchestrator-argon",
                "gemini-3.8-flash",
                "claude-opus-5-5",
                "claude-sonnet-5-5",
                "gpt-oss-120b",
            ]
        );
        let efforts = |model: &str| {
            agy.efforts_for_model(model)
                .into_iter()
                .map(|effort| effort.value)
                .collect::<Vec<_>>()
        };
        assert_eq!(efforts(""), [""]);
        assert_eq!(
            efforts("gemini-4-hydrogen"),
            ["", "low", "medium", "high", "xhigh"]
        );
        assert_eq!(efforts("claude-opus-5-5"), ["", "low", "medium", "high"]);
        assert_eq!(efforts("gpt-oss-120b"), ["", "medium"]);
        for model in [
            "gemini-pro-orchestrator",
            "gemini-pro-orchestrator-argon",
            "gemini-4-hydrogen-xhigh",
            "gpt-oss-120b-medium",
        ] {
            assert_eq!(efforts(model), [""], "{model}");
        }
    }

    #[test]
    fn selectable_catalog_requires_availability_and_honors_agent_settings() {
        let mut catalog = runtime_catalog_options();
        assert!(filter_selectable_runtime_catalog(catalog.clone(), None).is_empty());
        assert!(
            filter_selectable_runtime_catalog(catalog.clone(), Some(&["trae".into()])).is_empty()
        );
        for runtime in &mut catalog {
            runtime.available = true;
        }
        let expected = vec![
            Runtime::Codex,
            Runtime::ClaudeCode,
            Runtime::Antigravity,
            Runtime::Pi,
            Runtime::Copilot,
            Runtime::Cursor,
            Runtime::Grok,
            Runtime::OpenCode,
        ];
        assert_eq!(
            filter_selectable_runtime_catalog(catalog.clone(), None)
                .iter()
                .map(|runtime| runtime.name)
                .collect::<Vec<_>>(),
            expected
        );

        let enabled = vec!["trae".into()];
        assert_eq!(
            filter_selectable_runtime_catalog(catalog, Some(&enabled))
                .iter()
                .map(|runtime| runtime.name)
                .collect::<Vec<_>>(),
            [Runtime::Trae]
        );
    }

    #[cfg(unix)]
    #[test]
    fn update_spec_runs_the_effective_executable_with_its_update_argument() {
        use std::os::unix::fs::PermissionsExt;
        let bin = tempfile::tempdir().unwrap();
        for command in ["codex", "traecli"] {
            let path = bin.path().join(command);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let state = crate::test_support::test_core();
        state.runtime_shell_env.write().unwrap().path = Some(bin.path().display().to_string());

        let spec = runtime_update_spawn_spec(&state, Runtime::Codex, (90, 28)).unwrap();
        assert_eq!(spec.command, bin.path().join("codex").display().to_string());
        assert_eq!(spec.args, ["update"]);
        assert_eq!(spec.cwd, runner_core::app_paths::home_dir());
        assert_eq!(spec.initial_size, Some((90, 28)));

        assert!(runtime_update_spawn_spec(&state, Runtime::Trae, (90, 28)).is_err());
    }
    #[test]
    fn missing_agent_preview_uses_catalog_command_without_requiring_installation() {
        let state = crate::test_support::test_core();
        let root = tempfile::tempdir().unwrap();
        state.runtime_shell_env.write().unwrap().path = Some(root.path().display().to_string());
        state.runtime_discovery.write().unwrap().checking = false;
        let preview = runtime_command_preview(&state, Runtime::Trae, false).unwrap();
        assert_eq!(preview.lines[0].command, "traecli");
        assert!(!preview.lines.is_empty());
    }

    #[test]
    fn claude_preview_only_sets_config_dir_for_an_active_non_native_account() {
        use runner_core::protocol::runtime::RuntimeCommandNote;
        let state = crate::test_support::test_core();
        let preview = runtime_command_preview(&state, Runtime::ClaudeCode, false).unwrap();
        assert!(!preview
            .lines
            .iter()
            .any(|line| line.command.starts_with("CLAUDE_CONFIG_DIR=")));
        let mut accounts = crate::claude_accounts::load(&state.db).unwrap();
        accounts
            .accounts
            .push(runner_core::protocol::ClaudeAccount {
                id: "work".into(),
                name: "Work".into(),
                folder: "/tmp/runner-preview-work".into(),
                native: false,
            });
        accounts.active_id = "work".into();
        crate::db::app_state_set(
            &state.db.get().unwrap(),
            "claude_accounts",
            &serde_json::to_string(&accounts).unwrap(),
        )
        .unwrap();
        let preview = runtime_command_preview(&state, Runtime::ClaudeCode, false).unwrap();
        assert_eq!(
            preview.lines[0].command,
            "CLAUDE_CONFIG_DIR=/tmp/runner-preview-work"
        );
        assert_eq!(
            preview.lines[0].note,
            RuntimeCommandNote::ActiveAccount {
                name: "Work".into()
            }
        );
        accounts.active_id = accounts
            .accounts
            .iter()
            .find(|account| account.native)
            .unwrap()
            .id
            .clone();
        crate::db::app_state_set(
            &state.db.get().unwrap(),
            "claude_accounts",
            &serde_json::to_string(&accounts).unwrap(),
        )
        .unwrap();
        let preview = runtime_command_preview(&state, Runtime::ClaudeCode, false).unwrap();
        assert!(!preview
            .lines
            .iter()
            .any(|line| line.command.starts_with("CLAUDE_CONFIG_DIR=")));
    }

    #[test]
    fn custom_flags_validation_save_refusal_warning_and_clear() {
        let state = crate::test_support::test_core();
        {
            let mut discovery = state.runtime_discovery.write().unwrap();
            discovery.versions.set_help_flags(
                Runtime::Codex,
                Some(
                    ["-c", "--config", "--search", "--add-dir"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                ),
            );
        }

        // Save valid flags -> persisted verbatim, no warning.
        let initial = r#"--search --add-dir "/tmp/a b""#;
        let saved = runtime_set_custom_flags(&state, Runtime::Codex, initial).unwrap();
        assert_eq!(saved.warning, None);
        let codex = saved
            .status
            .runtimes
            .iter()
            .find(|r| r.name == Runtime::Codex)
            .unwrap();
        assert_eq!(codex.launch_options.custom_flags, initial);

        // Refusal (unclosed quote and reserved flag) keeps previously saved flags untouched.
        for bad in [
            "--search \"unclosed",
            "--model gpt-5",
            "-c features.remote_plugin=true",
        ] {
            assert!(
                runtime_set_custom_flags(&state, Runtime::Codex, bad).is_err(),
                "{bad}"
            );
            let current = crate::db::runtime_launch_options(&state.db).unwrap();
            assert_eq!(current["codex"].custom_flags, initial);
        }

        // Warning still saves and returns warning; preview recomputes it too.
        let warned = runtime_set_custom_flags(&state, Runtime::Codex, "--serch").unwrap();
        assert_eq!(
            warned.warning.as_deref(),
            Some("codex --help doesn't list --serch.")
        );
        let current = crate::db::runtime_launch_options(&state.db).unwrap();
        assert_eq!(current["codex"].custom_flags, "--serch");
        let preview = runtime_command_preview(&state, Runtime::Codex, false).unwrap();
        assert_eq!(
            preview.custom_flags_warning.as_deref(),
            Some("codex --help doesn't list --serch.")
        );

        // Clearing (whitespace/empty) removes the stored flags and warning.
        let cleared = runtime_set_custom_flags(&state, Runtime::Codex, "   ").unwrap();
        assert_eq!(cleared.warning, None);
        let codex = cleared
            .status
            .runtimes
            .iter()
            .find(|r| r.name == Runtime::Codex)
            .unwrap();
        assert_eq!(codex.launch_options.custom_flags, "");
        let preview = runtime_command_preview(&state, Runtime::Codex, false).unwrap();
        assert_eq!(preview.custom_flags_warning, None);
    }

    #[cfg(unix)]
    #[test]
    fn custom_flags_reads_stderr_from_help_probe_like_agy() {
        use std::os::unix::fs::PermissionsExt;

        let bin = tempfile::tempdir().unwrap();
        let agy = bin.path().join("agy");
        std::fs::write(
            &agy,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"1.0.0\"; exit 0; fi\necho \"  --known-flag  help\" >&2\n",
        )
        .unwrap();
        std::fs::set_permissions(&agy, std::fs::Permissions::from_mode(0o755)).unwrap();

        let state = crate::test_support::test_core();
        state.runtime_shell_env.write().unwrap().path = Some(bin.path().display().to_string());
        runtime_set_override(&state, Runtime::Antigravity, agy.to_str().unwrap()).unwrap();

        let ok = runtime_set_custom_flags(&state, Runtime::Antigravity, "--known-flag").unwrap();
        assert_eq!(ok.warning, None);

        let warned = runtime_set_custom_flags(&state, Runtime::Antigravity, "--unknown").unwrap();
        assert_eq!(
            warned.warning.as_deref(),
            Some("agy --help doesn't list --unknown.")
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejected_opencode_override_preserves_active_path_version_cache() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path_dir = dir.path().join("path");
        let other_dir = dir.path().join("other");
        std::fs::create_dir_all(&path_dir).unwrap();
        std::fs::create_dir_all(&other_dir).unwrap();
        let path_v1 = path_dir.join("opencode");
        let other_v1 = other_dir.join("opencode");
        for script in [&path_v1, &other_v1] {
            std::fs::write(script, "#!/bin/sh\necho '1.19.5'\n").unwrap();
            std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let state = crate::test_support::test_core();
        state.runtime_shell_env.write().unwrap().path = Some(path_dir.display().to_string());
        {
            let mut discovery = state.runtime_discovery.write().unwrap();
            discovery.checking = false;
            discovery.result = Some(crate::shell_path::DiscoveryResult {
                shell: Some("/bin/zsh".into()),
                outcome: crate::shell_path::DiscoveryOutcome::Ok,
                duration_ms: 10,
                env: crate::shell_path::LoginShellEnv::default(),
            });
        }

        // Cold model request probes the active PATH executable and caches its v1 result.
        let mut rx = state.events.subscribe();
        runtime_request_models(&state, &[Runtime::OpenCode]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut saw_change = false;
        while std::time::Instant::now() < deadline {
            if let Ok(ev) = rx.try_recv() {
                if ev.name == "runtime/changed" {
                    saw_change = true;
                    break;
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        assert!(
            saw_change,
            "expected cold runtime_request_models sync probe to emit runtime/changed"
        );

        let before = runtime_catalog(&state)
            .unwrap()
            .into_iter()
            .find(|entry| entry.name == Runtime::OpenCode)
            .unwrap();
        assert!(!before.available);
        assert!(before.below_minimum);
        assert_eq!(before.installed_version.as_deref(), Some("1.19.5"));

        // Rejected v1 override leaves DB unset AND preserves the active PATH cache.
        let err = runtime_set_override(&state, Runtime::OpenCode, other_v1.to_str().unwrap())
            .unwrap_err();
        assert!(err.to_string().contains("2.0.0"), "{err}");
        assert!(!crate::db::runtime_overrides(&state.db)
            .unwrap()
            .contains_key("opencode"));

        let after = runtime_catalog(&state)
            .unwrap()
            .into_iter()
            .find(|entry| entry.name == Runtime::OpenCode)
            .unwrap();
        assert!(!after.available);
        assert!(after.below_minimum);
        assert_eq!(after.installed_version.as_deref(), Some("1.19.5"));
    }

    #[cfg(unix)]
    #[test]
    fn sync_probe_preserves_async_notification_and_generation_ordering() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};

        // Part 1: an unchanneled (events = None) sync probe finishing while a background
        // probe for the same executable is in flight still lets the background probe emit.
        let dir = tempfile::tempdir().unwrap();
        let started = dir.path().join("async-started");
        let release_async = dir.path().join("release-async");
        let opencode = dir.path().join("opencode");
        std::fs::write(
            &opencode,
            format!(
                "#!/bin/sh\nif [ ! -f \"{}\" ]; then\n  : > \"{}\"\n  i=0\n  while [ ! -f \"{}\" ] && [ $i -lt 300 ]; do\n    sleep 0.01\n    i=$((i + 1))\n  done\n  echo 'unrecognized'\nelse\n  echo '1.19.5'\nfi\n",
                started.display(),
                started.display(),
                release_async.display(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&opencode, std::fs::Permissions::from_mode(0o755)).unwrap();

        let state = crate::test_support::test_core();
        state.runtime_shell_env.write().unwrap().path = Some(dir.path().display().to_string());
        let mut rx = state.events.subscribe();

        crate::runtime_status::versions::request_probes(
            &state.db,
            &state.runtime_shell_env,
            &state.runtime_discovery,
            &state.events,
            &[Runtime::OpenCode],
            false,
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !started.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(started.exists(), "background probe did not start");

        // Sync probe without EventChannel completes first and populates cache.
        assert_eq!(
            crate::runtime_status::versions::ensure_probed(
                Runtime::OpenCode,
                opencode.to_str().unwrap(),
                &state.runtime_shell_env,
                &state.runtime_discovery,
                None,
            )
            .as_deref(),
            Some("1.19.5")
        );
        assert_eq!(
            state
                .runtime_discovery
                .read()
                .unwrap()
                .versions
                .installed(Runtime::OpenCode, opencode.to_str().unwrap()),
            Some("1.19.5")
        );

        // Releasing the background probe still delivers runtime/changed.
        std::fs::write(&release_async, "go").unwrap();
        let mut saw_change = false;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(ev) = rx.try_recv() {
                if ev.name == "runtime/changed" {
                    saw_change = true;
                    break;
                }
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        assert!(saw_change, "background probe notification was swallowed");
        assert_eq!(
            state
                .runtime_discovery
                .read()
                .unwrap()
                .versions
                .installed(Runtime::OpenCode, opencode.to_str().unwrap()),
            Some("1.19.5")
        );

        // Part 2: an older in-flight sync probe cannot overwrite a newer accepted override.
        let slow_v1 = dir.path().join("slow-v1");
        let slow_started = dir.path().join("slow-started");
        let release_slow = dir.path().join("release-slow");
        std::fs::write(
            &slow_v1,
            format!(
                "#!/bin/sh\n: > \"{}\"\ni=0\nwhile [ ! -f \"{}\" ] && [ $i -lt 300 ]; do\n  sleep 0.01\n  i=$((i + 1))\ndone\necho '1.19.5'\n",
                slow_started.display(),
                release_slow.display(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&slow_v1, std::fs::Permissions::from_mode(0o755)).unwrap();

        let fast_v2 = dir.path().join("fast-v2");
        std::fs::write(&fast_v2, "#!/bin/sh\necho '2.0.29'\n").unwrap();
        std::fs::set_permissions(&fast_v2, std::fs::Permissions::from_mode(0o755)).unwrap();

        let state2 = crate::test_support::test_core();
        crate::db::set_runtime_override(&state2.db, "opencode", Some(slow_v1.to_str().unwrap()))
            .unwrap();

        let shell_env = std::sync::Arc::clone(&state2.runtime_shell_env);
        let discovery = std::sync::Arc::clone(&state2.runtime_discovery);
        let events = state2.events.clone();
        let slow_path = slow_v1.to_str().unwrap().to_owned();
        let sync_thread = std::thread::spawn(move || {
            crate::runtime_status::versions::ensure_probed(
                Runtime::OpenCode,
                &slow_path,
                &shell_env,
                &discovery,
                Some(&events),
            )
        });

        let deadline = Instant::now() + Duration::from_secs(10);
        while !slow_started.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(slow_started.exists(), "slow sync probe did not start");

        // Supersede the in-flight sync probe with a new v2 override, then release the old one.
        runtime_set_override(&state2, Runtime::OpenCode, fast_v2.to_str().unwrap()).unwrap();
        std::fs::write(&release_slow, "go").unwrap();
        assert_eq!(sync_thread.join().unwrap().as_deref(), Some("1.19.5"));

        assert_eq!(
            state2
                .runtime_discovery
                .read()
                .unwrap()
                .versions
                .installed(Runtime::OpenCode, fast_v2.to_str().unwrap()),
            Some("2.0.29")
        );
    }

    #[cfg(unix)]
    fn wait_for(deadline_secs: u64, mut ready: impl FnMut() -> bool, failure_msg: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(deadline_secs);
        while !ready() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(ready(), "{failure_msg}");
    }

    #[cfg(unix)]
    fn write_two_stage_probe_script(
        path: &std::path::Path,
        first_started: &std::path::Path,
        release_first: &std::path::Path,
        first_output: &str,
        second_started: &std::path::Path,
        release_second: &std::path::Path,
        second_output: &str,
    ) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(
            path,
            format!(
                "#!/bin/sh\nif [ \"$1\" != \"--version\" ]; then\n  exit 9\nfi\nif [ ! -f \"{}\" ]; then\n  : > \"{}\"\n  i=0\n  while [ ! -f \"{}\" ] && [ $i -lt 300 ]; do\n    sleep 0.01\n    i=$((i + 1))\n  done\n  echo '{}'\n  exit 0\nfi\n: > \"{}\"\ni=0\nwhile [ ! -f \"{}\" ] && [ $i -lt 300 ]; do\n  sleep 0.01\n  i=$((i + 1))\ndone\necho '{}'\n",
                first_started.display(),
                first_started.display(),
                release_first.display(),
                first_output,
                second_started.display(),
                release_second.display(),
                second_output,
            ),
        )
        .unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn older_async_probe_completing_first_cannot_bypass_sync_launch_gate() {
        let dir = tempfile::tempdir().unwrap();
        let async_started = dir.path().join("async-started");
        let release_async = dir.path().join("release-async");
        let sync_started = dir.path().join("sync-started");
        let release_sync = dir.path().join("release-sync");
        let opencode = dir.path().join("opencode");
        write_two_stage_probe_script(
            &opencode,
            &async_started,
            &release_async,
            "unrecognized",
            &sync_started,
            &release_sync,
            "1.19.5",
        );

        let state = crate::test_support::test_core();
        state.runtime_shell_env.write().unwrap().path = Some(dir.path().display().to_string());

        request_version_probe(&state, Runtime::OpenCode);
        wait_for(
            10,
            || async_started.exists(),
            "background probe did not start",
        );

        let role = crate::session::manager::runtime_direct_role(
            "opencode",
            Some(opencode.to_str().unwrap()),
            None,
            None,
        )
        .unwrap();
        let sessions = Arc::clone(&state.sessions);
        let db = Arc::clone(&state.db);
        let events = Arc::new(state.session_events());
        let app_dir = dir.path().to_path_buf();
        let spawn_handle = std::thread::spawn(move || {
            sessions.spawn_runtime_direct(
                &role,
                None,
                app_dir.to_str(),
                None,
                None,
                &app_dir,
                db,
                events,
            )
        });

        wait_for(
            10,
            || sync_started.exists(),
            "sync launch probe did not start",
        );

        // Complete older async worker first with unrecognized output.
        std::fs::write(&release_async, "go").unwrap();
        wait_for(
            10,
            || {
                !state
                    .runtime_discovery
                    .read()
                    .unwrap()
                    .versions
                    .has_in_flight_async(Runtime::OpenCode)
            },
            "background probe did not finish",
        );
        assert!(
            state
                .runtime_discovery
                .read()
                .unwrap()
                .versions
                .probed(Runtime::OpenCode, opencode.to_str().unwrap())
                .is_none(),
            "superseded background probe must not write None into cache"
        );

        // Now complete newer sync probe with v1.19.5; launch must be refused by RuntimeVersionGate.
        std::fs::write(&release_sync, "go").unwrap();
        let err = spawn_handle
            .join()
            .unwrap()
            .expect_err("opencode 1.19.5 must be refused before launching");
        let crate::error::Error::RuntimeVersionGate(gate) = err else {
            panic!("expected RuntimeVersionGate, got {err:#}");
        };
        assert_eq!(gate.runtime, Runtime::OpenCode);
        assert_eq!(gate.installed_version, "1.19.5");
        assert_eq!(gate.min_version, "2.0.0");
        assert_eq!(
            state
                .runtime_discovery
                .read()
                .unwrap()
                .versions
                .installed(Runtime::OpenCode, opencode.to_str().unwrap()),
            Some("1.19.5")
        );
    }

    #[cfg(unix)]
    #[test]
    fn older_async_probe_completing_first_still_emits_runtime_changed_on_status_read() {
        let dir = tempfile::tempdir().unwrap();
        let async_started = dir.path().join("async-started");
        let release_async = dir.path().join("release-async");
        let sync_started = dir.path().join("sync-started");
        let release_sync = dir.path().join("release-sync");
        let opencode = dir.path().join("opencode");
        write_two_stage_probe_script(
            &opencode,
            &async_started,
            &release_async,
            "unrecognized",
            &sync_started,
            &release_sync,
            "1.19.5",
        );

        let state = crate::test_support::test_core();
        crate::db::set_runtime_override(&state.db, "opencode", Some(opencode.to_str().unwrap()))
            .unwrap();
        let mut rx = state.events.subscribe();

        request_version_probe(&state, Runtime::OpenCode);
        wait_for(
            10,
            || async_started.exists(),
            "background probe did not start",
        );

        let state_bg = state.clone();
        let status_handle = std::thread::spawn(move || runtime_status_list(&state_bg));
        wait_for(
            10,
            || sync_started.exists(),
            "status sync probe did not start",
        );

        // Older async probe completes first; status read's sync probe still finishes and emits.
        std::fs::write(&release_async, "go").unwrap();
        wait_for(
            10,
            || {
                !state
                    .runtime_discovery
                    .read()
                    .unwrap()
                    .versions
                    .has_in_flight_async(Runtime::OpenCode)
            },
            "background probe did not finish",
        );

        std::fs::write(&release_sync, "go").unwrap();
        let response = status_handle.join().unwrap().unwrap();
        let row = response
            .runtimes
            .into_iter()
            .find(|entry| entry.name == Runtime::OpenCode)
            .unwrap();
        assert_eq!(
            row.state,
            crate::runtime_status::RuntimeRowState::InvalidOverride
        );
        assert!(row.below_minimum);
        assert_eq!(row.installed_version.as_deref(), Some("1.19.5"));

        let mut saw_change = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok(ev) = rx.try_recv() {
                if ev.name == "runtime/changed" {
                    saw_change = true;
                    break;
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        assert!(
            saw_change,
            "status read must emit runtime/changed when older async worker finished first"
        );
    }

    #[cfg(unix)]
    #[test]
    fn same_path_replacement_before_refresh_refuses_launch_without_spawning_standalone() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let opencode = dir.path().join("opencode");
        let standalone_called = dir.path().join("standalone.called");
        let write_script = |tmp_name: &str, version: &str| {
            let tmp = dir.path().join(tmp_name);
            std::fs::write(
                &tmp,
                format!(
                    "#!/bin/sh\nif [ \"$1\" = --version ]; then\n  echo '{version}'\n  exit 0\nfi\ntouch '{}'\nexit 0\n",
                    standalone_called.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::rename(&tmp, &opencode).unwrap();
        };

        write_script("opencode.v2", "2.0.29");

        let state = crate::test_support::test_core();
        state.runtime_shell_env.write().unwrap().path = Some(dir.path().display().to_string());
        assert_eq!(
            crate::runtime_status::versions::ensure_probed(
                Runtime::OpenCode,
                opencode.to_str().unwrap(),
                &state.runtime_shell_env,
                &state.runtime_discovery,
                None,
            )
            .as_deref(),
            Some("2.0.29")
        );

        // Replace binary on disk with v1.19.5 without clicking Refresh first.
        write_script("opencode.v1", "1.19.5");
        assert!(!standalone_called.exists());

        let role =
            crate::session::manager::runtime_direct_role("opencode", None, None, None).unwrap();
        let err = state
            .sessions
            .spawn_runtime_direct(
                &role,
                None,
                dir.path().to_str(),
                None,
                None,
                dir.path(),
                Arc::clone(&state.db),
                Arc::new(state.session_events()),
            )
            .expect_err("replaced v1 binary must be blocked before launching --standalone");
        let crate::error::Error::RuntimeVersionGate(gate) = err else {
            panic!("expected RuntimeVersionGate, got {err:#}");
        };
        assert_eq!(gate.runtime, Runtime::OpenCode);
        assert_eq!(gate.installed_version, "1.19.5");
        assert_eq!(gate.min_version, "2.0.0");
        assert!(
            !standalone_called.exists(),
            "opencode v1 must not be spawned when binary changed on disk before Refresh"
        );
    }

    #[cfg(unix)]
    #[test]
    fn classified_opencode_startup_error_clears_cached_probe_so_status_list_preserves_error() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let opencode = dir.path().join("opencode");
        let mode_file = dir.path().join("mode");
        std::fs::write(&mode_file, "v2").unwrap();
        std::fs::write(
            &opencode,
            format!(
                "#!/bin/sh\nmode=$(cat '{}')\nif [ \"$1\" = --version ]; then\n  if [ \"$mode\" = v2 ]; then\n    echo '2.0.29'\n  else\n    echo '1.19.5'\n  fi\n  exit 0\nfi\nif [ \"$mode\" = v1 ]; then\n  echo \"error: unknown option '--standalone'\"\n  exit 1\nfi\nexit 0\n",
                mode_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&opencode, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut state = crate::test_support::test_core_in(dir.path().to_path_buf());
        state.sessions = crate::session::manager::SessionManager::new(
            Arc::clone(&state.runtime_shell_env),
            Arc::clone(&state.runtime_discovery),
            Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
        );
        state.sessions.set_runtime_events(state.events.clone());
        state.runtime_shell_env.write().unwrap().path = Some(dir.path().display().to_string());
        state.runtime_discovery.write().unwrap().checking = false;

        assert_eq!(
            crate::runtime_status::versions::ensure_probed(
                Runtime::OpenCode,
                opencode.to_str().unwrap(),
                &state.runtime_shell_env,
                &state.runtime_discovery,
                Some(&state.events),
            )
            .as_deref(),
            Some("2.0.29")
        );

        // Flip mode behind an untouched wrapper script so cached 2.0.29 allows spawn once,
        // then v1 rejects --standalone.
        std::fs::write(&mode_file, "v1").unwrap();
        let mut rx = state.events.subscribe();
        let role =
            crate::session::manager::runtime_direct_role("opencode", None, None, None).unwrap();
        let spawned = state
            .sessions
            .spawn_runtime_direct(
                &role,
                None,
                dir.path().to_str(),
                None,
                None,
                dir.path(),
                Arc::clone(&state.db),
                Arc::new(state.session_events()),
            )
            .unwrap();

        wait_for(
            10,
            || {
                state
                    .sessions
                    .agent_status(&spawned.id)
                    .startup_error
                    .is_some()
            },
            "startup error was not classified",
        );

        let mut saw_change = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(ev) = rx.try_recv() {
                if ev.name == "runtime/changed" {
                    saw_change = true;
                    break;
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        assert!(saw_change, "clearing stale probe must emit runtime/changed");
        assert_eq!(
            state
                .runtime_discovery
                .read()
                .unwrap()
                .versions
                .installed(Runtime::OpenCode, opencode.to_str().unwrap()),
            None
        );

        // Passive status_list must not immediately retire the StartupError using stale v2 cache.
        let status = runtime_status_list(&state).unwrap();
        let row = status
            .runtimes
            .into_iter()
            .find(|entry| entry.name == Runtime::OpenCode)
            .unwrap();
        assert_eq!(row.installed_version, None);
        assert!(
            state
                .sessions
                .agent_status(&spawned.id)
                .startup_error
                .is_some(),
            "startup error must remain visible until a compatible v2 probe succeeds"
        );

        // Once switched back to v2 and re-probed, runtime_status_list retires the StartupError.
        std::fs::write(&mode_file, "v2").unwrap();
        assert_eq!(
            crate::runtime_status::versions::probe_now(
                Runtime::OpenCode,
                &state.db,
                &state.runtime_shell_env,
                &state.runtime_discovery,
                &state.events,
            )
            .as_deref(),
            Some("2.0.29")
        );
        let status = runtime_status_list(&state).unwrap();
        let row = status
            .runtimes
            .into_iter()
            .find(|entry| entry.name == Runtime::OpenCode)
            .unwrap();
        assert_eq!(row.installed_version.as_deref(), Some("2.0.29"));
        assert!(!row.below_minimum);
        assert!(
            state
                .sessions
                .agent_status(&spawned.id)
                .startup_error
                .is_none(),
            "compatible v2 refresh must retire obsolete startup error"
        );
    }
}
