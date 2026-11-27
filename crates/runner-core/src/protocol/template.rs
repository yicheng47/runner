use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{Crew, Role, Runtime, Slot};

pub use crate::template::{
    ExportedTemplate, PlannedRole, PlannedSlot, Template, TemplateError, TemplateKind,
    TemplateMeta, TemplatePreview, TemplatePreviewPlan,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TemplateImportInput {
    pub toml: String,
    #[serde(default)]
    pub crew_name: Option<String>,
    #[serde(default)]
    pub role_display_name: Option<String>,
    #[serde(default)]
    pub role_handle: Option<String>,
    #[serde(default)]
    pub runtime_overrides: BTreeMap<String, Runtime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateImportResult {
    pub kind: TemplateKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crew: Option<Crew>,
    pub roles: Vec<Role>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<Slot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uninstalled_runtimes: Vec<Runtime>,
}
