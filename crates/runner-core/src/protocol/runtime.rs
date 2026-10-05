use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCommandSource {
    Override,
    Detected,
    Catalog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeRowState {
    Detected,
    Override,
    NotFound,
    Checking,
    ProbeTimedOut,
    InvalidOverride,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellDiscoveryStatus {
    pub shell: Option<String>,
    pub outcome: Option<DiscoveryOutcome>,
    pub duration_ms: Option<u64>,
    pub checking: bool,
    pub using_last_known_good: bool,
    pub last_known_good_captured_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeExecutableStatus {
    pub name: Runtime,
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
    /// First semver token of `<effective command> --version`, once probed.
    pub installed_version: Option<String>,
    /// npm's latest version when it is newer than `installed_version` and
    /// the runtime can update itself; its presence shows the Update button.
    pub available_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStatusResponse {
    pub shell: ShellDiscoveryStatus,
    pub runtimes: Vec<RuntimeExecutableStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCapabilities {
    pub usage: bool,
    pub global_skill_toggle: bool,
    pub skill_toggle_requires_marker: bool,
    pub codex_speed: bool,
    pub effort_needs_launch_model: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(skip)]
    pub capabilities: super::runtime::RuntimeCapabilities,
    pub description: String,
    pub install_url: String,
    pub default_enabled: bool,
    pub available: bool,
    pub default_model: Option<String>,
    pub default_effort: Option<String>,
    pub models: Vec<RuntimeCatalogOption>,
    pub efforts: Vec<RuntimeCatalogOption>,
}

impl<'de> Deserialize<'de> for RuntimeCatalogEntry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Fields {
            name: Runtime,
            display_name: String,
            command: String,
            native_fork: bool,
            description: String,
            install_url: String,
            default_enabled: bool,
            available: bool,
            default_model: Option<String>,
            default_effort: Option<String>,
            models: Vec<RuntimeCatalogOption>,
            efforts: Vec<RuntimeCatalogOption>,
        }
        let fields = Fields::deserialize(deserializer)?;
        // Capabilities are immutable registry metadata, omitted from the public catalog JSON.
        let capabilities = super::runtime_metadata::catalog_for(fields.name)
            .map(|catalog| catalog.capabilities)
            .unwrap_or_default();
        Ok(Self {
            name: fields.name,
            display_name: fields.display_name,
            command: fields.command,
            native_fork: fields.native_fork,
            description: fields.description,
            install_url: fields.install_url,
            default_enabled: fields.default_enabled,
            available: fields.available,
            default_model: fields.default_model,
            default_effort: fields.default_effort,
            models: fields.models,
            efforts: fields.efforts,
            capabilities,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeCatalog {
    pub name: Runtime,
    pub display_name: &'static str,
    pub command: &'static str,
    pub native_fork: bool,
    pub capabilities: RuntimeCapabilities,
    pub skills_dirs: &'static [&'static str],
    pub update_args: &'static [&'static str],
    pub npm_package: Option<&'static str>,
    pub description: &'static str,
    pub install_url: &'static str,
    pub default_enabled: bool,
    pub models: Vec<RuntimeCatalogOption>,
    pub efforts: Vec<RuntimeCatalogOption>,
}

impl RuntimeCatalog {
    pub fn into_entry(self) -> RuntimeCatalogEntry {
        RuntimeCatalogEntry {
            name: self.name,
            display_name: self.display_name.into(),
            command: self.command.into(),
            native_fork: self.native_fork,
            capabilities: self.capabilities,
            description: self.description.into(),
            install_url: self.install_url.into(),
            default_enabled: self.default_enabled,
            available: false,
            default_model: None,
            default_effort: None,
            models: self.models,
            efforts: self.efforts,
        }
    }
}

impl RuntimeCatalogEntry {
    pub fn for_runtime(runtime: Runtime) -> Option<Self> {
        super::runtime_metadata::catalog_for(runtime).map(RuntimeCatalog::into_entry)
    }
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
