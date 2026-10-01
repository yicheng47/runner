pub(crate) mod antigravity;
pub(crate) mod catalog;
pub(crate) mod claude_code;
pub(crate) mod codex;
pub(crate) mod copilot;
mod helpers;
pub(crate) mod pi;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
pub(crate) mod trae;

use crate::model::{CodexSpeed, Runtime};
use crate::ops::runtime::{RuntimeCatalogEntry, RuntimeCatalogOption};
use crate::router::prompt::{LaunchPromptInput, SessionPromptKind};
use crate::router::runtime::{
    ForkPlan, MissionPermissionMode, PermissionMode, ResumePlan, FIRST_TURN_ARGV_MAX_BYTES,
};
pub use helpers::Permissions;
#[cfg(test)]
pub(crate) use helpers::{test_home, with_conversation_home};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct RuntimeCatalog {
    pub name: Runtime,
    pub display_name: &'static str,
    pub command: &'static str,
    pub native_fork: bool,
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

pub fn catalogs() -> Vec<RuntimeCatalog> {
    Runtime::ALL
        .into_iter()
        .filter_map(|runtime| adapter(runtime).catalog())
        .collect()
}

pub struct LaunchContext<'a> {
    pub role_args: &'a [String],
    pub app_data_dir: &'a Path,
    pub session_id: &'a str,
    pub resuming: bool,
    pub mission: bool,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub codex_speed: Option<CodexSpeed>,
    pub system_prompt: Option<&'a str>,
    pub first_turn: Option<&'a str>,
}

#[derive(Default)]
pub struct PromptChannels {
    pub system_prompt: bool,
    pub resend_persona_on_fresh: bool,
}

impl PromptChannels {
    pub fn split(
        &self,
        kind: SessionPromptKind,
        composed: Option<String>,
    ) -> (Option<String>, Option<String>) {
        if !self.system_prompt {
            return (None, composed);
        }
        let Some(composed) = composed else {
            return (None, None);
        };
        match kind {
            SessionPromptKind::Direct | SessionPromptKind::Worker => (Some(composed), None),
            SessionPromptKind::Lead => {
                unreachable!("pi lead prompts must be composed from LaunchPromptInput")
            }
        }
    }
    pub fn lead(&self, input: &LaunchPromptInput<'_>) -> (Option<String>, Option<String>) {
        if !self.system_prompt {
            return (
                None,
                Some(crate::router::prompt::compose_launch_prompt(input)),
            );
        }
        let sections = crate::router::prompt::compose_launch_prompt_sections(input);
        let mut system_prompt = sections.before_mission;
        system_prompt.push_str(&sections.after_mission);
        (Some(system_prompt), Some(sections.mission))
    }
    pub fn system_prompt_args(&self, prompt: Option<&str>) -> Vec<String> {
        match prompt {
            Some(prompt) if self.system_prompt && !prompt.trim().is_empty() => {
                helpers::strings(&["--append-system-prompt", prompt])
            }
            _ => Vec::new(),
        }
    }
}

pub struct ProbeContext<'a> {
    pub cwd: Option<&'a str>,
    pub role_env: &'a HashMap<String, String>,
}

#[derive(Default)]
pub struct MissingConversation {
    pub reuse_key: bool,
    pub resume_on_launch: bool,
}

pub trait RuntimeAdapter: Send + Sync {
    fn catalog(&self) -> Option<RuntimeCatalog>;
    fn permissions(&self) -> &'static Permissions;
    fn first_turn_argv(&self, body: Option<&str>) -> Vec<String>;
    fn resume_plan(&self, prior_key: Option<&str>) -> ResumePlan;
    fn model_effort_args(&self, _model: Option<&str>, _effort: Option<&str>) -> Vec<String> {
        Vec::new()
    }
    fn prompt_channels(&self) -> PromptChannels {
        PromptChannels::default()
    }
    fn launch_args(&self, ctx: &LaunchContext<'_>) -> Vec<String> {
        self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn })
    }
    fn mission_dir_args(&self, _dir: Option<&Path>) -> Vec<String> {
        Vec::new()
    }
    fn fork_plan(&self, _source_key: &str, _source_label: &str) -> Option<ForkPlan> {
        None
    }
    fn conversation_exists(&self, _key: &str, _ctx: &ProbeContext<'_>) -> Option<bool> {
        None
    }
    fn missing_conversation(&self) -> MissingConversation {
        MissingConversation::default()
    }
}

pub struct NoAgent;
static NO_PERMISSIONS: Permissions = Permissions {
    offered: &[],
    strip_flags: &[],
    equals_on_bool: false,
    variadic_flag: None,
    args: |_| Vec::new(),
    matches: |_, _| false,
    mission_bypass: None,
    strip_on_mission_resume: false,
};
impl RuntimeAdapter for NoAgent {
    fn catalog(&self) -> Option<RuntimeCatalog> {
        None
    }
    fn permissions(&self) -> &'static Permissions {
        &NO_PERMISSIONS
    }
    fn first_turn_argv(&self, body: Option<&str>) -> Vec<String> {
        helpers::first_turn_body(body);
        Vec::new()
    }
    fn resume_plan(&self, _prior_key: Option<&str>) -> ResumePlan {
        ResumePlan::fresh()
    }
}

pub fn adapter(runtime: Runtime) -> &'static dyn RuntimeAdapter {
    match runtime {
        Runtime::Codex => &codex::Codex,
        Runtime::ClaudeCode => &claude_code::ClaudeCode,
        Runtime::Antigravity => &antigravity::Antigravity,
        Runtime::Pi => &pi::Pi,
        Runtime::Copilot => &copilot::Copilot,
        Runtime::Trae => &trae::Trae,
        Runtime::Shell => &NoAgent,
    }
}

pub fn for_key(key: &str) -> &'static dyn RuntimeAdapter {
    Runtime::parse(key).map(adapter).unwrap_or(&NoAgent)
}
