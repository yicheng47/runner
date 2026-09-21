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
    runtime_status_list(state).map_err(persistence_error)
}

pub fn runtime_clear_override(state: &AppCore, runtime: Runtime) -> Result<RuntimeStatusResponse> {
    if crate::router::runtime::runtime_definition(runtime).is_none() {
        return Err(Error::msg(format!("unknown runtime: {runtime}")));
    }
    crate::db::set_runtime_override(&state.db, runtime.key(), None)?;
    log::info!("runtime override cleared: runtime={runtime}");
    state.events.emit("runtime/changed", &());
    runtime_status_list(state)
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
            description: "TRAE CLI".into(),
            install_url: "https://docs.trae.cn/cli_get-started-with-trae-code-cli-2".into(),
            default_enabled: cfg!(target_os = "macos"),
            available: false,
            default_model: None,
            default_effort: None,
            models: vec![default_model_option()],
            efforts: common_efforts(),
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
}
