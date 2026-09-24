use crate::model::Runtime;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::runtime_status::{OverrideValidationError, RuntimeCommandSource, RuntimeStatusResponse};
use crate::AppCore;

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeDefinition {
    pub name: Runtime,
    pub display_name: String,
    pub command: String,
    pub native_fork: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct RuntimeCatalogOption {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported_efforts: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeCatalogEntry {
    pub name: Runtime,
    pub display_name: String,
    pub command: String,
    pub native_fork: bool,
    pub description: String,
    pub install_url: String,
    pub default_enabled: bool,
    pub available: bool,
    pub default_model: Option<String>,
    pub default_effort: Option<String>,
    pub models: Vec<RuntimeCatalogOption>,
    pub efforts: Vec<RuntimeCatalogOption>,
}

impl RuntimeCatalogEntry {
    pub fn efforts_for_model(&self, model: &str) -> Vec<RuntimeCatalogOption> {
        let model = model.trim();
        let supported = self
            .models
            .iter()
            .find(|entry| entry.value == model)
            .and_then(|entry| entry.supported_efforts.as_ref());
        self.efforts
            .iter()
            .filter(|effort| {
                effort.value.is_empty()
                    || (!model.is_empty()
                        && supported.is_none_or(|levels| levels.contains(&effort.value)))
            })
            .cloned()
            .collect()
    }
}

pub fn runtime_list() -> Vec<RuntimeDefinition> {
    crate::router::runtime::runtime_definitions()
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
    if crate::router::runtime::runtime_definition(runtime).is_none() {
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
    if crate::router::runtime::runtime_definition(runtime).is_none() {
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
    let definition = crate::router::runtime::runtime_definition(runtime)
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
/// session: `events` alone hears its output and exit.
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

pub fn runtime_request_models(state: &AppCore, runtimes: &[Runtime]) {
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
        return;
    }
    crate::runtime_status::models::request(
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
        &state.events,
        runtimes,
        false,
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
    crate::runtime_status::models::DISCOVERY_RUNTIMES.to_vec()
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

pub fn filter_selectable_runtime_catalog(
    catalog: Vec<RuntimeCatalogEntry>,
    enabled_agents: Option<&[String]>,
) -> Vec<RuntimeCatalogEntry> {
    let enabled_agents: Option<HashSet<&str>> =
        enabled_agents.map(|agents| agents.iter().map(String::as_str).collect());
    catalog
        .into_iter()
        .filter(|runtime| {
            let enabled = enabled_agents
                .as_ref()
                .map_or(runtime.default_enabled, |agents| {
                    agents.contains(runtime.name.key())
                });
            enabled && runtime.available
        })
        .collect()
}

fn option(value: &str, label: &str, description: &str) -> RuntimeCatalogOption {
    RuntimeCatalogOption {
        value: value.into(),
        label: label.into(),
        description: Some(description.into()),
        supported_efforts: None,
    }
}

fn plain_option(value: &str, label: &str) -> RuntimeCatalogOption {
    RuntimeCatalogOption {
        value: value.into(),
        label: label.into(),
        description: None,
        supported_efforts: None,
    }
}

fn default_model_option() -> RuntimeCatalogOption {
    option("", "default", "Use the agent's own default model.")
}

fn default_effort() -> RuntimeCatalogOption {
    option(
        "",
        "default",
        "Use the agent's own default effort; no flag passed.",
    )
}

fn common_efforts() -> Vec<RuntimeCatalogOption> {
    vec![
        default_effort(),
        option("low", "Low", "Fast responses with lighter reasoning."),
        option("medium", "Medium", "Balances speed and reasoning depth."),
        option(
            "high",
            "High",
            "Greater reasoning depth for complex problems.",
        ),
        option(
            "xhigh",
            "Extra high",
            "Extra reasoning depth for complex problems.",
        ),
    ]
}

fn runtime_catalog_options() -> Vec<RuntimeCatalogEntry> {
    let claude_efforts = vec![
        default_effort(),
        plain_option("low", "low"),
        plain_option("medium", "medium"),
        plain_option("high", "high"),
        plain_option("xhigh", "xhigh"),
        plain_option("max", "max"),
    ];
    let mut codex_efforts = common_efforts();
    codex_efforts.push(option(
        "max",
        "Max",
        "Maximum reasoning depth for the hardest problems.",
    ));
    codex_efforts.push(option(
        "ultra",
        "Ultra",
        "Maximum reasoning with automatic task delegation.",
    ));

    vec![
        RuntimeCatalogEntry {
            name: Runtime::Codex,
            display_name: "Codex".into(),
            command: "codex".into(),
            native_fork: crate::router::runtime::supports_native_fork(Some(Runtime::Codex)),
            description: "OpenAI Codex CLI".into(),
            install_url: "https://developers.openai.com/codex/cli".into(),
            default_enabled: true,
            available: false,
            default_model: None,
            default_effort: None,
            models: vec![
                default_model_option(),
                option(
                    "gpt-6-astra",
                    "gpt-6-astra",
                    "Our most capable model for complex, demanding work.",
                ),
                option(
                    "gpt-5.6-sol",
                    "gpt-5.6-sol",
                    "Reliable agentic workhorse for everyday tasks.",
                ),
                option(
                    "gpt-5.6-terra",
                    "gpt-5.6-terra",
                    "Balanced agentic coding model for everyday work.",
                ),
                option(
                    "gpt-5.6-luna",
                    "gpt-5.6-luna",
                    "Fast and affordable agentic coding model.",
                ),
                option(
                    "gpt-5.5",
                    "gpt-5.5",
                    "Frontier model for complex coding, research, and real-world work.",
                ),
                option("gpt-5.4", "gpt-5.4", "Strong model for everyday coding."),
                option(
                    "gpt-5.4-mini",
                    "gpt-5.4-mini",
                    "Small, fast, and cost-efficient model for simpler coding tasks.",
                ),
                option(
                    "gpt-5.3-codex-spark",
                    "gpt-5.3-codex-spark",
                    "Ultra-fast coding model.",
                ),
            ],
            efforts: codex_efforts,
        },
        RuntimeCatalogEntry {
            name: Runtime::ClaudeCode,
            display_name: "Claude Code".into(),
            command: "claude".into(),
            native_fork: crate::router::runtime::supports_native_fork(Some(Runtime::ClaudeCode)),
            description: "Anthropic Claude Code CLI".into(),
            install_url: "https://code.claude.com/docs/en/setup".into(),
            default_enabled: true,
            available: false,
            default_model: None,
            default_effort: None,
            models: vec![
                default_model_option(),
                option("fable", "fable", "Latest Claude Fable."),
                option("opus", "opus", "Latest Claude Opus."),
                option("sonnet", "sonnet", "Latest Claude Sonnet."),
                option("haiku", "haiku", "Latest Claude Haiku."),
            ],
            efforts: claude_efforts,
        },
        RuntimeCatalogEntry {
            name: Runtime::Copilot,
            display_name: "GitHub Copilot CLI".into(),
            command: "copilot".into(),
            native_fork: false,
            description: "GitHub Copilot CLI (requires a Copilot subscription)".into(),
            install_url: "https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli".into(),
            default_enabled: true,
            available: false,
            default_model: None,
            default_effort: None,
            models: std::iter::once(default_model_option())
                .chain(
                    [
                        "auto",
                        "claude-sonnet-5",
                        "claude-fable-5.1",
                        "claude-fable-5",
                        "claude-opus-5",
                        "claude-opus-4.8",
                        "claude-opus-4.8-fast",
                        "claude-opus-4.7",
                        "claude-sonnet-4.6",
                        "claude-haiku-4.5",
                        "gpt-5.6-sol",
                        "gpt-5.6-terra",
                        "gpt-5.6-luna",
                        "gpt-5.5",
                        "gpt-5.4",
                        "gpt-5.4-mini",
                        "gpt-5.3-codex",
                        "gpt-5-mini",
                        "mai-code-1.1-flash",
                        "mai-code-1-flash-picker",
                        "gemini-3.8-flash",
                        "gemini-3.7-flash",
                        "gemini-3.6-flash",
                        "gemini-3.5-flash",
                        "grok-4.5",
                        "kimi-k3",
                        "kimi-k2.7-code",
                    ]
                    .into_iter()
                    .map(|model| plain_option(model, model)),
                )
                .collect(),
            efforts: std::iter::once(default_effort())
                .chain(
                    ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
                        .into_iter()
                        .map(|effort| plain_option(effort, effort)),
                )
                .collect(),
        },
        RuntimeCatalogEntry {
            name: Runtime::Pi,
            display_name: "pi".into(),
            command: "pi".into(),
            native_fork: crate::router::runtime::supports_native_fork(Some(Runtime::Pi)),
            description: "pi coding agent (bring your own model provider)".into(),
            install_url: "https://github.com/earendil-works/pi".into(),
            default_enabled: true,
            available: false,
            default_model: None,
            default_effort: None,
            models: vec![default_model_option()],
            efforts: std::iter::once(default_effort())
                .chain(
                    ["off", "minimal", "low", "medium", "high", "xhigh", "max"]
                        .into_iter()
                        .map(|effort| plain_option(effort, effort)),
                )
                .collect(),
        },
        RuntimeCatalogEntry {
            name: Runtime::Trae,
            display_name: "TRAE CLI".into(),
            command: "traecli".into(),
            native_fork: crate::router::runtime::supports_native_fork(Some(Runtime::Trae)),
            description: String::new(),
            install_url: String::new(),
            default_enabled: cfg!(target_os = "macos"),
            available: false,
            default_model: None,
            default_effort: None,
            models: vec![default_model_option()],
            efforts: common_efforts(),
        },
        RuntimeCatalogEntry {
            name: Runtime::Antigravity,
            display_name: "Antigravity CLI".into(),
            command: "agy".into(),
            native_fork: crate::router::runtime::supports_native_fork(Some(Runtime::Antigravity)),
            description: "Google Antigravity CLI (signs in with a Google account)".into(),
            install_url: "https://antigravity.google/docs/cli/reference".into(),
            default_enabled: cfg!(target_os = "macos"),
            available: false,
            default_model: None,
            default_effort: None,
            models: std::iter::once(default_model_option())
                .chain(crate::router::runtime::ANTIGRAVITY_MODELS.iter().map(
                    |(model, efforts)| RuntimeCatalogOption {
                        supported_efforts: Some(
                            efforts.iter().map(|effort| (*effort).into()).collect(),
                        ),
                        ..plain_option(model, model)
                    },
                ))
                .collect(),
            efforts: std::iter::once(default_effort())
                .chain(
                    crate::router::runtime::ANTIGRAVITY_EFFORTS
                        .iter()
                        .map(|effort| plain_option(effort, effort)),
                )
                .collect(),
        },
        // Models are `provider/model` from the user's own providers, typed
        // freely; the TUI has no effort flag (spec 592 decision 4).
        RuntimeCatalogEntry {
            name: Runtime::OpenCode,
            display_name: "OpenCode".into(),
            command: "opencode".into(),
            native_fork: crate::router::runtime::supports_native_fork(Some(Runtime::OpenCode)),
            description: "OpenCode (bring your own model provider)".into(),
            install_url: "https://opencode.ai/docs/".into(),
            default_enabled: cfg!(target_os = "macos"),
            available: false,
            default_model: None,
            default_effort: None,
            models: vec![default_model_option()],
            efforts: vec![default_effort()],
        },
    ]
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
        let opencode = definitions
            .iter()
            .find(|runtime| runtime.name == Runtime::OpenCode)
            .unwrap();
        assert_eq!(opencode.display_name, "OpenCode");
        assert_eq!(opencode.command, "opencode");
        assert!(opencode.native_fork);
        let definition = crate::router::runtime::runtime_definition(Runtime::OpenCode).unwrap();
        assert_eq!(
            definition.skills_dirs,
            [
                ".config/opencode/skills",
                ".claude/skills",
                ".agents/skills"
            ]
        );
        assert_eq!(definition.update_args, ["upgrade"]);
        assert_eq!(definition.npm_package, Some("opencode-ai"));

        let catalog = runtime_catalog_options();
        assert_eq!(
            catalog
                .iter()
                .map(|runtime| runtime.name)
                .collect::<Vec<_>>(),
            [
                Runtime::Codex,
                Runtime::ClaudeCode,
                Runtime::Copilot,
                Runtime::Pi,
                Runtime::Trae,
                Runtime::Antigravity,
                Runtime::OpenCode,
            ]
        );
        assert!(catalog[0].default_enabled);
        assert!(catalog[1].default_enabled);
        assert!(catalog[2].default_enabled);
        assert_eq!(catalog[2].models[1].value, "auto");
        assert_eq!(catalog[2].models.len(), 28);
        assert_eq!(catalog[4].default_enabled, cfg!(target_os = "macos"));
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
            catalog[2]
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
            catalog[4]
                .efforts
                .iter()
                .map(|effort| effort.value.as_str())
                .collect::<Vec<_>>(),
            ["", "low", "medium", "high", "xhigh"]
        );

        let agy = &catalog[5];
        assert_eq!(agy.command, "agy");
        assert!(!agy.native_fork);
        assert_eq!(agy.default_enabled, cfg!(target_os = "macos"));
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

        let opencode = &catalog[6];
        assert_eq!(opencode.command, "opencode");
        assert!(opencode.native_fork);
        assert_eq!(opencode.default_enabled, cfg!(target_os = "macos"));
        assert_eq!(
            opencode
                .models
                .iter()
                .map(|model| model.value.as_str())
                .collect::<Vec<_>>(),
            [""]
        );
        assert_eq!(
            opencode
                .efforts
                .iter()
                .map(|effort| effort.value.as_str())
                .collect::<Vec<_>>(),
            [""]
        );
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
        let expected = if cfg!(target_os = "macos") {
            vec![
                Runtime::Codex,
                Runtime::ClaudeCode,
                Runtime::Copilot,
                Runtime::Pi,
                Runtime::Trae,
                Runtime::Antigravity,
                Runtime::OpenCode,
            ]
        } else {
            vec![
                Runtime::Codex,
                Runtime::ClaudeCode,
                Runtime::Copilot,
                Runtime::Pi,
            ]
        };
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
        for command in ["codex", "traecli", "opencode"] {
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

        let spec = runtime_update_spawn_spec(&state, Runtime::OpenCode, (90, 28)).unwrap();
        assert_eq!(
            spec.command,
            bin.path().join("opencode").display().to_string()
        );
        assert_eq!(spec.args, ["upgrade"]);
    }
}
