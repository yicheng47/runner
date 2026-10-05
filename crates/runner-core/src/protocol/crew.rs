use super::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CreateCrewInput {
    pub name: String,
    /// Optional team-conventions text. Empty after trim → stored as NULL.
    /// Plain Option (not Option<Option>) because create has no "leave
    /// existing" semantic. See #54.
    #[serde(default)]
    pub system_prompt_addendum: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct UpdateCrewInput {
    pub name: Option<String>,
    /// Outer None = leave existing untouched; outer Some(inner) =
    /// write inner. Inner Some("") / whitespace-only collapses to
    /// NULL.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::double_option"
    )]
    pub system_prompt_addendum: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrewListItem {
    #[serde(flatten)]
    pub crew: Crew,
    pub role_count: i64,
    /// Member preview for the Crews list cards: one entry per slot,
    /// in `position` order, carrying just the labels the card pills
    /// need (`@slot_handle` + `runtime-role_handle`). Sourced
    /// inline so the frontend doesn't N+1 `slot_list` for each crew
    /// on every page load.
    pub members: Vec<CrewMemberPreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrewMemberPreview {
    pub slot_handle: String,
    pub role_handle: String,
    pub runtime: String,
    pub lead: bool,
}
