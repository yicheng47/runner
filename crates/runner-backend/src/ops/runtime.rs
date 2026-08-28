use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::runtime_status::{OverrideValidationError, RuntimeCommandSource, RuntimeStatusResponse};
use crate::AppCore;

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeDefinition {
    pub name: String,
    pub display_name: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeCatalogOption {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeCatalogEntry {
    pub name: String,
    pub display_name: String,
    pub command: String,
    pub description: String,
    pub default_enabled: bool,
    pub available: bool,
    pub default_model: Option<String>,
    pub default_effort: Option<String>,
    pub models: Vec<RuntimeCatalogOption>,
    pub efforts: Vec<RuntimeCatalogOption>,
}

pub fn runtime_list() -> Vec<RuntimeDefinition> {
    crate::router::runtime::runtime_definitions()
        .iter()
        .map(|runtime| RuntimeDefinition {
            name: runtime.name.to_string(),
            display_name: runtime.display_name.to_string(),
            command: runtime.command.to_string(),
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
    runtime: &str,
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
        crate::db::set_runtime_override(&state.db, runtime, None).map_err(persistence_error)?;
    } else {
        crate::runtime_status::validate_override(runtime, path)?;
        crate::db::set_runtime_override(&state.db, runtime, Some(path))
            .map_err(persistence_error)?;
        log::info!("runtime override saved: runtime={runtime} path={path}");
    }
    state.events.emit("runtime/changed", &());
    runtime_status_list(state).map_err(persistence_error)
}

pub fn runtime_clear_override(state: &AppCore, runtime: &str) -> Result<RuntimeStatusResponse> {
    if crate::router::runtime::runtime_definition(runtime).is_none() {
        return Err(Error::msg(format!("unknown runtime: {runtime}")));
    }
    crate::db::set_runtime_override(&state.db, runtime, None)?;
    log::info!("runtime override cleared: runtime={runtime}");
    state.events.emit("runtime/changed", &());
    runtime_status_list(state)
}

pub fn runtime_refresh(state: &AppCore) -> Result<RuntimeStatusResponse> {
    crate::runtime_status::refresh_background_discovery(
        state.events.clone(),
        Arc::clone(&state.db),
        Arc::clone(&state.runtime_shell_env),
        Arc::clone(&state.runtime_discovery),
    )?;
    runtime_status_list(state)
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
                (available, runtime.default_model, runtime.default_effort),
            )
        })
        .collect();
    Ok(runtime_catalog_options()
        .into_iter()
        .map(|mut runtime| {
            if let Some((available, default_model, default_effort)) = statuses.get(&runtime.name) {
                runtime.available = *available;
                runtime.default_model.clone_from(default_model);
                runtime.default_effort.clone_from(default_effort);
            }
            runtime
        })
        .collect())
}

pub fn selectable_runtime_catalog(
    state: &AppCore,
    enabled_agents: Option<&[String]>,
) -> Result<Vec<RuntimeCatalogEntry>> {
    Ok(filter_selectable_runtime_catalog(
        runtime_catalog(state)?,
        enabled_agents,
    ))
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
                    agents.contains(runtime.name.as_str())
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
    }
}

fn plain_option(value: &str, label: &str) -> RuntimeCatalogOption {
    RuntimeCatalogOption {
        value: value.into(),
        label: label.into(),
        description: None,
    }
}

fn default_model() -> RuntimeCatalogOption {
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
            name: "codex".into(),
            display_name: "Codex".into(),
            command: "codex".into(),
            description: "OpenAI Codex CLI".into(),
            default_enabled: true,
            available: false,
            default_model: None,
            default_effort: None,
            models: vec![
                default_model(),
                option(
                    "gpt-5.6-sol",
                    "gpt-5.6-sol",
                    "Latest frontier agentic coding model.",
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
            name: "claude-code".into(),
            display_name: "Claude Code".into(),
            command: "claude".into(),
            description: "Anthropic Claude Code CLI".into(),
            default_enabled: true,
            available: false,
            default_model: None,
            default_effort: None,
            models: vec![
                default_model(),
                option("fable", "fable", "Latest Claude Fable."),
                option("opus", "opus", "Latest Claude Opus."),
                option("sonnet", "sonnet", "Latest Claude Sonnet."),
                option("haiku", "haiku", "Latest Claude Haiku."),
            ],
            efforts: claude_efforts,
        },
        RuntimeCatalogEntry {
            name: "trae".into(),
            display_name: "TRAE CLI".into(),
            command: "traecli".into(),
            description: "TRAE CLI".into(),
            default_enabled: false,
            available: false,
            default_model: None,
            default_effort: None,
            models: vec![default_model()],
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
    fn catalog_matches_supported_runtime_order_and_defaults() {
        let catalog = runtime_catalog_options();
        assert_eq!(
            catalog
                .iter()
                .map(|runtime| runtime.name.as_str())
                .collect::<Vec<_>>(),
            ["codex", "claude-code", "trae"]
        );
        assert!(catalog[0].default_enabled);
        assert!(catalog[1].default_enabled);
        assert!(!catalog[2].default_enabled);
        assert_eq!(
            catalog[0]
                .models
                .iter()
                .map(|model| model.value.as_str())
                .collect::<Vec<_>>(),
            [
                "",
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
            catalog[2]
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
        for runtime in &mut catalog {
            runtime.available = true;
        }
        assert_eq!(
            filter_selectable_runtime_catalog(catalog.clone(), None)
                .iter()
                .map(|runtime| runtime.name.as_str())
                .collect::<Vec<_>>(),
            ["codex", "claude-code"]
        );

        let enabled = vec!["trae".to_string()];
        assert_eq!(
            filter_selectable_runtime_catalog(catalog, Some(&enabled))
                .iter()
                .map(|runtime| runtime.name.as_str())
                .collect::<Vec<_>>(),
            ["trae"]
        );
    }
}
