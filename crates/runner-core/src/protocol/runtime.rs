use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeLaunchOptions {
    pub bypass: bool,
    pub remote_plugin_catalog: bool,
    pub status_hooks: bool,
}

impl Default for RuntimeLaunchOptions {
    fn default() -> Self {
        Self {
            bypass: false,
            remote_plugin_catalog: true,
            status_hooks: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLaunchOption {
    Bypass,
    RemotePluginCatalog,
    StatusHooks,
}

impl RuntimeLaunchOption {
    pub fn supported(self, runtime: Runtime) -> bool {
        match self {
            Self::Bypass => matches!(runtime, Runtime::Codex | Runtime::ClaudeCode),
            Self::RemotePluginCatalog => runtime == Runtime::Codex,
            Self::StatusHooks => {
                matches!(
                    runtime,
                    Runtime::Codex
                        | Runtime::ClaudeCode
                        | Runtime::Pi
                        | Runtime::Copilot
                        | Runtime::OpenCode
                ) || (runtime == Runtime::Grok && !cfg!(windows))
            }
        }
    }
}

impl RuntimeLaunchOptions {
    pub fn get(self, option: RuntimeLaunchOption) -> bool {
        match option {
            RuntimeLaunchOption::Bypass => self.bypass,
            RuntimeLaunchOption::RemotePluginCatalog => self.remote_plugin_catalog,
            RuntimeLaunchOption::StatusHooks => self.status_hooks,
        }
    }
    pub fn set(&mut self, option: RuntimeLaunchOption, enabled: bool) {
        match option {
            RuntimeLaunchOption::Bypass => self.bypass = enabled,
            RuntimeLaunchOption::RemotePluginCatalog => self.remote_plugin_catalog = enabled,
            RuntimeLaunchOption::StatusHooks => self.status_hooks = enabled,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCommandNote {
    #[default]
    None,
    Always,
    AlwaysInMissions,
    NamesChat,
    FromChat,
    FromRole,
    FirstTurn,
    MissionFiles,
    CodexBypass,
    ClaudeBypass,
    CatalogOff,
    StatusHooks,
    HooksBypass,
    IdentityBypass,
    Identity,
    ActiveAccount {
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeCommandLine {
    pub command: String,
    pub note: RuntimeCommandNote,
    pub option: Option<RuntimeLaunchOption>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeCommandPreview {
    pub lines: Vec<RuntimeCommandLine>,
}
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
    #[serde(default)]
    pub launch_options: RuntimeLaunchOptions,
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
    #[serde(default)]
    pub min_version: Option<String>,
    #[serde(default)]
    pub below_minimum: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub below_minimum: bool,

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
            #[serde(default)]
            installed_version: Option<String>,
            #[serde(default)]
            available_version: Option<String>,
            #[serde(default)]
            min_version: Option<String>,
            #[serde(default)]
            below_minimum: bool,
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
            installed_version: fields.installed_version,
            available_version: fields.available_version,
            min_version: fields.min_version,
            below_minimum: fields.below_minimum,
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
    pub min_version: Option<&'static str>,
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
            installed_version: None,
            available_version: None,
            min_version: self.min_version.map(str::to_owned),
            below_minimum: false,
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
        filter_efforts_for_model(self.name, &self.models, &self.efforts, model)
    }
}

/// Filter a runtime's effort options for `model`, preserving the default empty
/// entry and disabling conflicting effort choices on leveled Antigravity IDs.
pub fn filter_efforts_for_model(
    runtime: Runtime,
    models: &[RuntimeCatalogOption],
    efforts: &[RuntimeCatalogOption],
    model: &str,
) -> Vec<RuntimeCatalogOption> {
    let model = model.trim();
    let matched = models.iter().find(|entry| entry.value == model);
    let leveled_suffix = (runtime == Runtime::Antigravity && matched.is_none())
        .then(|| super::runtime_metadata::antigravity::split_leveled_model(model))
        .flatten();
    let supported = matched.and_then(|entry| entry.supported_efforts.as_ref());
    efforts
        .iter()
        .filter(|effort| {
            effort.value.is_empty()
                || (!model.is_empty()
                    && leveled_suffix.is_none()
                    && supported.is_none_or(|levels| levels.contains(&effort.value)))
        })
        .cloned()
        .collect()
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{runtime} {installed_version} is too old for Runner. Runner needs {min_version} or newer. Update {runtime} in Settings → Agents.")]
pub struct RuntimeVersionGate {
    pub runtime: Runtime,
    pub installed_version: String,
    pub min_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeUpdateState {
    Running,
    MayNeedInput,
    Verifying,
    Stopping,
    Succeeded,
    Failed,
    BrokenAfterStop,
}
impl RuntimeUpdateState {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Running | Self::MayNeedInput | Self::Verifying | Self::Stopping
        )
    }
    pub fn priority(self) -> u8 {
        match self {
            Self::MayNeedInput => 0,
            Self::Failed | Self::BrokenAfterStop => 1,
            Self::Running | Self::Verifying | Self::Stopping => 2,
            Self::Succeeded => 3,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeUpdateTask {
    pub id: String,
    pub runtime: Runtime,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    pub state: RuntimeUpdateState,
    pub step: String,
    pub message: Option<String>,
    pub exit_code: Option<i32>,
    pub installed_version: Option<String>,
    pub running_sessions: usize,
}
