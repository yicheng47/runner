mod format;
mod parse;
mod plan;

#[cfg(test)]
mod tests;

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::protocol::CodexSpeed;
use crate::Runtime;

pub use format::{
    crew_filename, export_crew, export_role, format_template, role_filename, slugify,
};
pub use parse::{parse_template, read_template_bounded};
pub use plan::{
    allocate_role_handle, build_preview, prompt_stats, resolve_crew_import, resolve_role_import,
    suggest_crew_name, PlannedRole, PlannedSlot, ResolvedCrewImport, ResolvedRoleImport,
    ResolvedSlot, TemplatePreview, TemplatePreviewPlan,
};

pub const TEMPLATE_VERSION: u32 = 1;
pub const MAX_TEMPLATE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateKind {
    Crew,
    Role,
}

impl TemplateKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Crew => "crew",
            Self::Role => "role",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
}

impl TemplateMeta {
    pub fn is_empty(&self) -> bool {
        self.description.is_none()
            && self.author.is_none()
            && self.tags.is_empty()
            && self.homepage.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleTemplate {
    pub handle: String,
    pub display_name: String,
    pub runtime: Runtime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_speed: Option<CodexSpeed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotTemplate {
    pub handle: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Runtime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_speed: Option<CodexSpeed>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrewSection {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conventions: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrewTemplate {
    #[serde(default, skip_serializing_if = "TemplateMeta::is_empty")]
    pub meta: TemplateMeta,
    pub crew: CrewSection,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<SlotTemplate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<RoleTemplate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandaloneRoleTemplate {
    #[serde(default, skip_serializing_if = "TemplateMeta::is_empty")]
    pub meta: TemplateMeta,
    pub role: RoleTemplate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Template {
    Crew(CrewTemplate),
    Role(StandaloneRoleTemplate),
}

impl Template {
    pub fn kind(&self) -> TemplateKind {
        match self {
            Self::Crew(_) => TemplateKind::Crew,
            Self::Role(_) => TemplateKind::Role,
        }
    }

    pub fn meta(&self) -> &TemplateMeta {
        match self {
            Self::Crew(crew) => &crew.meta,
            Self::Role(role) => &role.meta,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportedTemplate {
    pub filename: String,
    pub toml: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateError {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

impl TemplateError {
    pub fn new(message: impl Into<String>, line: Option<usize>) -> Self {
        Self {
            message: message.into(),
            line,
        }
    }
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{} (line {line})", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for TemplateError {}

pub(crate) fn is_agent_runtime(runtime: Runtime) -> bool {
    crate::protocol::runtime_metadata::catalog_for(runtime).is_some()
}

pub(crate) fn supports_codex_speed(runtime: Runtime) -> bool {
    crate::protocol::runtime_metadata::catalog_for(runtime)
        .is_some_and(|catalog| catalog.capabilities.codex_speed)
}
