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
    crate::runtime_status::status_list(
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
    )
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
    if path.is_empty() {
        crate::db::set_runtime_override(&state.db, runtime.key(), None)
            .map_err(persistence_error)?;
    } else {
        crate::runtime_status::validate_override(runtime, path)?;
        crate::db::set_runtime_override(&state.db, runtime.key(), Some(path))
            .map_err(persistence_error)?;
        log::info!("runtime override saved: runtime={runtime} path={path}");
    }
    state.events.emit("runtime/changed", &());
    request_version_probe(state, runtime);
    runtime_status_list(state).map_err(persistence_error)
}

pub fn runtime_clear_override(state: &AppCore, runtime: Runtime) -> Result<RuntimeStatusResponse> {
    if crate::runtimes::adapter(runtime).catalog().is_none() {
        return Err(Error::msg(format!("unknown runtime: {runtime}")));
    }
    crate::db::set_runtime_override(&state.db, runtime.key(), None)?;
    log::info!("runtime override cleared: runtime={runtime}");
    state.events.emit("runtime/changed", &());
    request_version_probe(state, runtime);
    runtime_status_list(state)
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
/// only.
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
/// without Runner's layers, and the home directory. On Windows it refuses
/// while any session of the runtime is alive, because the executable is in
/// use.
pub fn runtime_update_spawn_spec(
    state: &AppCore,
    runtime: Runtime,
    size: (u16, u16),
) -> Result<crate::session::runtime::SpawnSpec> {
    let definition = crate::runtimes::adapter(runtime)
        .catalog()
        .filter(|definition| !definition.update_args.is_empty())
        .ok_or_else(|| Error::msg(format!("{runtime} has no update command")))?;
    #[cfg(windows)]
    if let Some(&count) = crate::ops::session::live_session_counts(state)?
        .get(&runtime)
        .filter(|count| **count > 0)
    {
        return Err(Error::msg(format!(
            "Stop the {count} running {} sessions first.",
            definition.display_name
        )));
    }
    let command = crate::runtime_status::effective_runtime_command(
        runtime,
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
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
) -> Result<()> {
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
        .map(|runtime| {
            let available = matches!(
                runtime.effective_source,
                Some(RuntimeCommandSource::Detected | RuntimeCommandSource::Override)
            );
            (
                runtime.name,
                (
                    available,
                    runtime.default_model,
                    runtime.default_effort,
                    runtime.effective_command,
                ),
            )
        })
        .collect();
    let discovery = state
        .runtime_discovery
        .read()
        .map_err(|_| Error::msg("runtime discovery lock poisoned"))?;
    Ok(runtime_catalog_options()
        .into_iter()
        .map(|mut runtime| {
            if let Some((available, default_model, default_effort, command)) =
                statuses.get(&runtime.name)
            {
                runtime.available = *available;
                runtime.default_model.clone_from(default_model);
                runtime.default_effort.clone_from(default_effort);
                if let Some(catalog) = command.as_deref().and_then(|command| {
                    discovery.models.catalog(
                        runtime.name,
                        &crate::runtime_status::models::source(runtime.name, command),
                    )
                }) {
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

pub fn runtime_update_prepare(
    state: &AppCore,
    runtime: Runtime,
    size: (u16, u16),
) -> Result<runner_core::protocol::terminal::RuntimeUpdateCommand> {
    let spec = runtime_update_spawn_spec(state, runtime, size)?;
    let events: Arc<dyn crate::session::manager::SessionEvents> = Arc::new(state.session_events());
    state
        .sessions
        .prepare_unlisted_terminal(&spec.session_id, size, &state.db, &events)?;
    Ok(runner_core::protocol::terminal::RuntimeUpdateCommand {
        session_id: spec.session_id,
        command: spec.command,
        args: spec.args,
        cwd: spec.cwd,
        env: spec.env,
        shell_path: spec.shell_path,
        size,
    })
}

pub fn runtime_update_run(
    state: &AppCore,
    command: runner_core::protocol::terminal::RuntimeUpdateCommand,
) -> Result<()> {
    let spec = crate::session::runtime::SpawnSpec {
        agent_runtime: None,
        pending_turn: None,
        session_id: command.session_id,
        cwd: command.cwd,
        command: command.command,
        args: command.args,
        env: command.env,
        mission: false,
        shim_dir: None,
        bundled_bin_dir: None,
        shell_path: command.shell_path,
        initial_size: Some(command.size),
    };
    let id = spec.session_id.clone();
    let result = runtime_update_start(state, spec, Arc::new(state.session_events()));
    if result.is_err() {
        state.sessions.discard_unlisted_terminal(&id);
    }
    result
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
        ];
        assert_eq!(
            definitions
                .iter()
                .map(|runtime| runtime.name)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(&Runtime::ALL[..6], &expected);
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
                "gemini-3.8-flash",
                "gemini-3.7-flash",
                "gemini-3.6-flash",
                "gemini-3.1-pro",
                "claude-sonnet-4-6",
                "claude-opus-4-6-thinking",
                "gpt-oss-120b-medium",
            ]
        );
        let efforts = |model: &str| {
            agy.efforts_for_model(model)
                .into_iter()
                .map(|effort| effort.value)
                .collect::<Vec<_>>()
        };
        assert_eq!(efforts(""), [""]);
        assert_eq!(efforts("gemini-3.8-flash"), ["", "low", "medium", "high"]);
        assert_eq!(efforts("gemini-3.1-pro"), ["", "low", "high"]);
        for model in [
            "claude-sonnet-4-6",
            "claude-opus-4-6-thinking",
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
            Runtime::Trae,
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
}
