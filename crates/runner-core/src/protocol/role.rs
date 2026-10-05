use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CreateRoleInput {
    pub handle: String,
    pub display_name: String,
    pub runtime: Runtime,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "schemars", schemars(with = "Option<String>"))]
    pub codex_speed: Option<CodexSpeed>,
    /// Permission mode the role-edit form's dropdown chose. Mapped
    /// to concrete flags on the row's `args` column at create time
    /// via `router::runtime::apply_permission_mode`. Defaults to
    /// `Auto`. The new-role form defaults to codex, where Auto
    /// maps to `--ask-for-approval on-request --sandbox workspace-write`.
    /// For claude-code, Auto is still plan/model-gated; callers that
    /// default to claude-code should send AcceptEdits explicitly.
    #[serde(default = "default_permission_mode")]
    pub permission_mode: super::permissions::PermissionMode,
}

/// Default permission mode for new roles — `Auto`. Matches the
/// frontend's dropdown default and the seed's role args.
/// Pulled out so serde's `#[serde(default = "...")]` can name it.
pub fn default_permission_mode() -> super::permissions::PermissionMode {
    super::permissions::PermissionMode::Auto
}

// `handle` is intentionally excluded from updates: per arch §2.2 and §5.2
// the handle is the role template's identity in events, CLI
// addressing, and policy rules. Renaming after creation would break
// historical event attribution and any persisted policy references.
// Users who want a different handle delete the role and create a
// new one. (Per-slot in-crew identity lives on `slots.slot_handle`
// and is renameable.)
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct UpdateRoleInput {
    pub display_name: Option<String>,
    pub runtime: Option<Runtime>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::double_option"
    )]
    pub working_dir: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::double_option"
    )]
    pub system_prompt: Option<Option<String>>,
    pub env: Option<HashMap<String, String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::double_option"
    )]
    pub model: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::double_option"
    )]
    pub effort: Option<Option<String>>,
    #[cfg_attr(feature = "schemars", schemars(with = "Option<String>"))]
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::double_option"
    )]
    #[cfg_attr(feature = "schemars", schemars(extend("default" = ())))]
    pub codex_speed: Option<Option<CodexSpeed>>,
    /// Form's "Permission mode" segmented control. `Some(mode)`
    /// rewrites the runtime's permission flags to the canonical args
    /// for that mode (replacing any prior occurrence so duplicates
    /// can't accumulate). `None` preserves the args as-is — callers
    /// that don't surface the control (CLI patches, programmatic
    /// updates) shouldn't have to reason about it. See
    /// `router::runtime::apply_permission_mode`.
    pub permission_mode: Option<super::permissions::PermissionMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleActivity {
    pub role_id: String,
    pub active_sessions: i64,
    pub active_missions: i64,
    pub crew_count: i64,
    pub last_started_at: Option<Timestamp>,
    /// Most recent running direct-chat session for this role, if any.
    /// Lets the sidebar's SESSION list re-attach to a live PTY across page
    /// reloads — without this, the frontend `activeSessions` map starts
    /// empty on reload and we'd fall back to the role detail page.
    pub direct_session_id: Option<String>,
}

/// Role row plus its `RoleActivity`. Returned by `role_list_with_activity`
/// so the Roles list page can render every card's badges in one IPC round-
/// trip — without this the page would do N+1 calls (one `role_list` and
/// one `role_activity` per row), which also produces a flicker as
/// counters fill in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleWithActivity {
    #[serde(flatten)]
    pub role: Role,
    #[serde(flatten)]
    pub activity: RoleActivity,
}
