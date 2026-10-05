use super::*;
use serde::{Deserialize, Serialize};
/// One crew that a given role template is referenced by, plus the
/// slot's lead flag and added-at timestamp. Returned by
/// `role_crews_list` to render the "Crews using this role" panel
/// on Role Detail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrewMembership {
    pub crew_id: String,
    pub crew_name: String,
    pub slot_id: String,
    pub slot_handle: String,
    pub lead: bool,
    pub position: i64,
    pub added_at: Timestamp,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct UpdateSlotInput {
    pub slot_handle: Option<String>,
    /// Per-slot engine choice. Omit to preserve, pass `null` to clear
    /// (back to the role's own runtime), pass a registry runtime
    /// name to override. Only agent runtimes are accepted; shell is not
    /// a valid slot override.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::double_option"
    )]
    #[cfg_attr(feature = "schemars", schemars(extend("default" = ())))]
    pub runtime_override: Option<Option<Runtime>>,
    /// Per-slot model. Omit to preserve, pass `null` or blank to
    /// inherit, or pass a model name to override.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::double_option"
    )]
    #[cfg_attr(feature = "schemars", schemars(extend("default" = ())))]
    pub model_override: Option<Option<String>>,
    /// Per-slot thinking effort. Omit to preserve, pass `null` or
    /// blank to inherit, or pass an effort level to override.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::double_option"
    )]
    #[cfg_attr(feature = "schemars", schemars(extend("default" = ())))]
    pub effort_override: Option<Option<String>>,
    /// Per-slot Codex Speed. Omit to preserve, pass `null` to inherit
    /// the role choice, or pass `standard` / `fast` to override it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::double_option"
    )]
    #[cfg_attr(feature = "schemars", schemars(extend("default" = ())))]
    pub codex_speed_override: Option<Option<CodexSpeed>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CreateSlotInput {
    pub crew_id: String,
    pub role_id: String,
    pub slot_handle: String,
    /// Optional per-slot engine choice. Omit (or null) for the
    /// "Role default" behavior; otherwise an agent runtime registry name.
    /// Shell is not a valid slot override.
    #[serde(default)]
    pub runtime_override: Option<Runtime>,
    /// Optional model pinned to the selected runtime. Blank or omitted
    /// inherits from the role template.
    #[serde(default)]
    pub model_override: Option<String>,
}
