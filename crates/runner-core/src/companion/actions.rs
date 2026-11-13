use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ActionRequest {
    Role {
        id: u64,
        role: String,
    },
    Crew {
        id: u64,
        crew: String,
    },
    CreateRole {
        id: u64,
        handle: String,
        name: String,
        runtime: String,
        command: Option<String>,
        args: Vec<String>,
        working_dir: Option<String>,
        system_prompt: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        speed: Option<String>,
        permission_mode: Option<RolePermissionMode>,
    },
    UpdateRole {
        id: u64,
        role: String,
        name: String,
        runtime: String,
        command: Option<String>,
        args: Vec<String>,
        working_dir: Option<String>,
        system_prompt: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        speed: Option<String>,
        permission_mode: Option<RolePermissionMode>,
    },
    DeleteRole {
        id: u64,
        role: String,
    },
    CreateCrew {
        id: u64,
        name: String,
        conventions: Option<String>,
    },
    UpdateCrew {
        id: u64,
        crew: String,
        name: String,
        conventions: Option<String>,
    },
    DeleteCrew {
        id: u64,
        crew: String,
    },
    AddSlot {
        id: u64,
        crew: String,
        role: String,
        handle: String,
        runtime: Option<String>,
        model: Option<String>,
    },
    UpdateSlot {
        id: u64,
        slot: String,
        handle: String,
        runtime: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        speed: Option<String>,
    },
    RemoveSlot {
        id: u64,
        slot: String,
    },
    ReorderSlots {
        id: u64,
        crew: String,
        order: Vec<String>,
    },
    RuntimeTaskStart {
        id: u64,
        runtime: String,
    },
    RuntimeTask {
        id: u64,
        runtime: String,
    },
    Catalog {
        id: u64,
    },
    Usage {
        id: u64,
        popover: bool,
    },
    Folders {
        id: u64,
        path: Option<String>,
    },
    StartChat {
        id: u64,
        role: Option<String>,
        runtime: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        speed: Option<String>,
        project: Option<String>,
        cwd: Option<String>,
        cols: u16,
        rows: u16,
    },
    StartShell {
        id: u64,
        project: Option<String>,
        cwd: Option<String>,
        cols: u16,
        rows: u16,
    },
    Stop {
        id: u64,
        session: String,
    },
    MarkViewed {
        id: u64,
        session: String,
    },
    Resume {
        id: u64,
        session: String,
        cols: u16,
        rows: u16,
    },
    Restart {
        id: u64,
        session: String,
        cols: u16,
        rows: u16,
    },
    Fork {
        id: u64,
        session: String,
        title: Option<String>,
        cols: u16,
        rows: u16,
    },
    Close {
        id: u64,
        session: String,
    },
    Rename {
        id: u64,
        target: ActionTarget,
        name: String,
    },
    Archive {
        id: u64,
        target: ActionTarget,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        with_missions: bool,
    },
    Layout {
        id: u64,
        target: ActionTarget,
        project: Option<String>,
        name: String,
        panes: ActionPane,
        shells: Vec<String>,
    },
    Pin {
        id: u64,
        target: ActionTarget,
        pinned: bool,
    },
    Move {
        id: u64,
        target: ActionTarget,
        project: Option<String>,
        order: Vec<ActionTarget>,
    },
    ReorderPinned {
        id: u64,
        order: Vec<ActionTarget>,
    },
    CreateProject {
        id: u64,
        name: String,
        path: String,
    },
    DeleteProject {
        id: u64,
        project: String,
        force: bool,
    },
    StartMission {
        id: u64,
        crew: String,
        project: Option<String>,
        title: String,
        goal: Option<String>,
        cwd: Option<String>,
        cols: u16,
        rows: u16,
    },
    StopMission {
        id: u64,
        mission: String,
    },
    ResumeMission {
        id: u64,
        mission: String,
        cols: u16,
        rows: u16,
    },
    LinkMission {
        id: u64,
        mission: String,
        session: String,
    },
    UnlinkMission {
        id: u64,
        mission: String,
    },
}

impl ActionRequest {
    pub(super) fn field_problem(kind: &str, value: &serde_json::Value) -> Option<String> {
        macro_rules! fields {
            ($($field:ident: $ty:ty => $expected:literal),* $(,)?) => {{
                let problem:Option<String>=None;
                $(let problem=problem.or_else(|| super::messages::field_problem::<$ty>(value,stringify!($field),$expected));)*
                problem
            }};
        }
        match kind {
            "runtimeTaskStart" | "runtimeTask" => fields!(runtime: String => "a string"),
            "role" => fields!(role: String => "a string"),
            "crew" => fields!(crew: String => "a string"),
            "createRole" => {
                fields!(handle: String => "a string", name: String => "a string", runtime: String => "a string", command: Option<String> => "a string or null", args: Vec<String> => "an array of strings", workingDir: Option<String> => "a string or null", systemPrompt: Option<String> => "a string or null", model: Option<String> => "a string or null", effort: Option<String> => "a string or null", speed: Option<String> => "a string or null", permissionMode: Option<RolePermissionMode> => "a string or null")
            }
            "updateRole" => {
                fields!(role: String => "a string", name: String => "a string", runtime: String => "a string", command: Option<String> => "a string or null", args: Vec<String> => "an array of strings", workingDir: Option<String> => "a string or null", systemPrompt: Option<String> => "a string or null", model: Option<String> => "a string or null", effort: Option<String> => "a string or null", speed: Option<String> => "a string or null", permissionMode: Option<RolePermissionMode> => "a string or null")
            }
            "deleteRole" => fields!(role: String => "a string"),
            "createCrew" => {
                fields!(name: String => "a string", conventions: Option<String> => "a string or null")
            }
            "updateCrew" => {
                fields!(crew: String => "a string", name: String => "a string", conventions: Option<String> => "a string or null")
            }
            "deleteCrew" => fields!(crew: String => "a string"),
            "addSlot" => {
                fields!(crew: String => "a string", role: String => "a string", handle: String => "a string", runtime: Option<String> => "a string or null", model: Option<String> => "a string or null")
            }
            "updateSlot" => {
                fields!(slot: String => "a string", handle: String => "a string", runtime: Option<String> => "a string or null", model: Option<String> => "a string or null", effort: Option<String> => "a string or null", speed: Option<String> => "a string or null")
            }
            "removeSlot" => fields!(slot: String => "a string"),
            "reorderSlots" => {
                fields!(crew: String => "a string", order: Vec<String> => "an array of strings")
            }
            "catalog" => None,
            "usage" => fields!(popover: bool => "a boolean"),
            "folders" => fields!(path: Option<String> => "a string or null"),
            "startChat" => {
                fields!(role: Option<String> => "a string or null", runtime: Option<String> => "a string or null", model: Option<String> => "a string or null", effort: Option<String> => "a string or null", speed: Option<String> => "a string or null", project: Option<String> => "a string or null", cwd: Option<String> => "a string or null", cols: u16 => "an unsigned 16-bit integer", rows: u16 => "an unsigned 16-bit integer")
            }
            "startShell" => {
                fields!(project: Option<String> => "a string or null", cwd: Option<String> => "a string or null", cols: u16 => "an unsigned 16-bit integer", rows: u16 => "an unsigned 16-bit integer")
            }
            "stop" | "close" | "markViewed" => fields!(session: String => "a string"),
            "resume" | "restart" => {
                fields!(session: String => "a string", cols: u16 => "an unsigned 16-bit integer", rows: u16 => "an unsigned 16-bit integer")
            }
            "fork" => {
                fields!(session: String => "a string", title: Option<String> => "a string or null", cols: u16 => "an unsigned 16-bit integer", rows: u16 => "an unsigned 16-bit integer")
            }
            "rename" => {
                fields!(target: ActionTarget => "an item target", name: String => "a string")
            }
            "archive" => fields!(target: ActionTarget => "an item target").or_else(|| {
                value
                    .get("withMissions")
                    .is_some()
                    .then(|| {
                        super::messages::field_problem::<bool>(value, "withMissions", "a boolean")
                    })
                    .flatten()
            }),
            "layout" => {
                fields!(target: ActionTarget => "an item target", project: Option<String> => "a string or null", name: String => "a string", panes: ActionPane => "a pane tree", shells: Vec<String> => "an array of strings")
            }
            "pin" => fields!(target: ActionTarget => "an item target", pinned: bool => "a boolean"),
            "move" => {
                fields!(target: ActionTarget => "an item target", project: Option<String> => "a string or null", order: Vec<ActionTarget> => "an array of item targets")
            }
            "reorderPinned" => fields!(order: Vec<ActionTarget> => "an array of item targets"),
            "createProject" => fields!(name: String => "a string", path: String => "a string"),
            "deleteProject" => fields!(project: String => "a string", force: bool => "a boolean"),
            "startMission" => {
                fields!(crew: String => "a string", project: Option<String> => "a string or null", title: String => "a string", goal: Option<String> => "a string or null", cwd: Option<String> => "a string or null", cols: u16 => "an unsigned 16-bit integer", rows: u16 => "an unsigned 16-bit integer")
            }
            "stopMission" | "unlinkMission" => fields!(mission: String => "a string"),
            "linkMission" => {
                fields!(mission: String => "a string", session: String => "a string")
            }
            "resumeMission" => {
                fields!(mission: String => "a string", cols: u16 => "an unsigned 16-bit integer", rows: u16 => "an unsigned 16-bit integer")
            }
            _ => None,
        }
    }
    pub const NAMES: &[&str] = &[
        "role",
        "crew",
        "createRole",
        "updateRole",
        "deleteRole",
        "createCrew",
        "updateCrew",
        "deleteCrew",
        "addSlot",
        "updateSlot",
        "removeSlot",
        "reorderSlots",
        "runtimeTaskStart",
        "runtimeTask",
        "catalog",
        "usage",
        "folders",
        "startChat",
        "startShell",
        "stop",
        "markViewed",
        "resume",
        "restart",
        "fork",
        "close",
        "rename",
        "archive",
        "layout",
        "pin",
        "move",
        "reorderPinned",
        "createProject",
        "deleteProject",
        "startMission",
        "stopMission",
        "resumeMission",
        "linkMission",
        "unlinkMission",
    ];
    pub fn id(&self) -> u64 {
        match self {
            Self::Role { id, .. } => *id,
            Self::Crew { id, .. } => *id,
            Self::CreateRole { id, .. } => *id,
            Self::UpdateRole { id, .. } => *id,
            Self::DeleteRole { id, .. } => *id,
            Self::CreateCrew { id, .. } => *id,
            Self::UpdateCrew { id, .. } => *id,
            Self::DeleteCrew { id, .. } => *id,
            Self::AddSlot { id, .. } => *id,
            Self::UpdateSlot { id, .. } => *id,
            Self::RemoveSlot { id, .. } => *id,
            Self::ReorderSlots { id, .. } => *id,
            Self::RuntimeTaskStart { id, .. } | Self::RuntimeTask { id, .. } => *id,
            Self::Catalog { id, .. } => *id,
            Self::Usage { id, .. } => *id,
            Self::Folders { id, .. } => *id,
            Self::StartChat { id, .. } => *id,
            Self::StartShell { id, .. } => *id,
            Self::Stop { id, .. } => *id,
            Self::MarkViewed { id, .. } => *id,
            Self::Resume { id, .. } => *id,
            Self::Restart { id, .. } => *id,
            Self::Fork { id, .. } => *id,
            Self::Close { id, .. } => *id,
            Self::Rename { id, .. } => *id,
            Self::Archive { id, .. } => *id,
            Self::Layout { id, .. } => *id,
            Self::Pin { id, .. } => *id,
            Self::Move { id, .. } => *id,
            Self::ReorderPinned { id, .. } => *id,
            Self::CreateProject { id, .. } => *id,
            Self::DeleteProject { id, .. } => *id,
            Self::StartMission { id, .. } => *id,
            Self::StopMission { id, .. } => *id,
            Self::ResumeMission { id, .. } => *id,
            Self::LinkMission { id, .. } => *id,
            Self::UnlinkMission { id, .. } => *id,
        }
    }
    pub fn set_id(&mut self, value: u64) {
        match self {
            Self::Role { id, .. } => *id = value,
            Self::Crew { id, .. } => *id = value,
            Self::CreateRole { id, .. } => *id = value,
            Self::UpdateRole { id, .. } => *id = value,
            Self::DeleteRole { id, .. } => *id = value,
            Self::CreateCrew { id, .. } => *id = value,
            Self::UpdateCrew { id, .. } => *id = value,
            Self::DeleteCrew { id, .. } => *id = value,
            Self::AddSlot { id, .. } => *id = value,
            Self::UpdateSlot { id, .. } => *id = value,
            Self::RemoveSlot { id, .. } => *id = value,
            Self::ReorderSlots { id, .. } => *id = value,
            Self::RuntimeTaskStart { id, .. } | Self::RuntimeTask { id, .. } => *id = value,
            Self::Catalog { id, .. } => *id = value,
            Self::Usage { id, .. } => *id = value,
            Self::Folders { id, .. } => *id = value,
            Self::StartChat { id, .. } => *id = value,
            Self::StartShell { id, .. } => *id = value,
            Self::Stop { id, .. } => *id = value,
            Self::MarkViewed { id, .. } => *id = value,
            Self::Resume { id, .. } => *id = value,
            Self::Restart { id, .. } => *id = value,
            Self::Fork { id, .. } => *id = value,
            Self::Close { id, .. } => *id = value,
            Self::Rename { id, .. } => *id = value,
            Self::Archive { id, .. } => *id = value,
            Self::Layout { id, .. } => *id = value,
            Self::Pin { id, .. } => *id = value,
            Self::Move { id, .. } => *id = value,
            Self::ReorderPinned { id, .. } => *id = value,
            Self::CreateProject { id, .. } => *id = value,
            Self::DeleteProject { id, .. } => *id = value,
            Self::StartMission { id, .. } => *id = value,
            Self::StopMission { id, .. } => *id = value,
            Self::ResumeMission { id, .. } => *id = value,
            Self::LinkMission { id, .. } => *id = value,
            Self::UnlinkMission { id, .. } => *id = value,
        }
    }
}

impl std::fmt::Debug for ActionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionRequest")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionKind {
    Session,
    Tab,
    Mission,
    Project,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionTarget {
    pub kind: ActionKind,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ActionPane {
    Split {
        id: String,
        split: super::SplitDirection,
        sizes: [f64; 2],
        a: Box<ActionPane>,
        b: Box<ActionPane>,
    },
    Leaf {
        id: String,
        session: Option<String>,
    },
}

impl<'de> Deserialize<'de> for ActionPane {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("split").is_some() {
            #[derive(Deserialize)]
            struct Fields {
                id: String,
                split: super::SplitDirection,
                sizes: [f64; 2],
                a: Box<ActionPane>,
                b: Box<ActionPane>,
            }
            let fields: Fields = serde_json::from_value(value).map_err(D::Error::custom)?;
            Ok(Self::Split {
                id: fields.id,
                split: fields.split,
                sizes: fields.sizes,
                a: fields.a,
                b: fields.b,
            })
        } else {
            #[derive(Deserialize)]
            struct Fields {
                id: String,
                session: Option<String>,
            }
            let fields: Fields = serde_json::from_value(value).map_err(D::Error::custom)?;
            Ok(Self::Leaf {
                id: fields.id,
                session: fields.session,
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderPage {
    pub path: String,
    pub parent: Option<String>,
    pub folders: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub roles: Vec<CatalogRole>,
    pub runtimes: Vec<CatalogRuntime>,
    pub accounts: Vec<CatalogName>,
    pub active_account: String,
    pub crews: Vec<CatalogCrew>,
    pub projects: Vec<CatalogProject>,
    pub recent_folders: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogName {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRole {
    pub id: String,
    pub name: String,
    pub handle: String,
    pub runtime: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub speed: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRuntime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub below_minimum: bool,

    pub name: String,
    pub label: String,
    pub available: bool,
    pub native_fork: bool,
    pub default_model: Option<String>,
    pub default_effort: Option<String>,
    pub models: Vec<CatalogOption>,
    pub efforts: Vec<CatalogOption>,
    pub speeds: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogOption {
    pub value: String,
    pub label: String,
    pub supported_efforts: Option<Vec<String>>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogCrew {
    pub id: String,
    pub name: String,
    pub slots: Vec<CatalogSlot>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogSlot {
    pub handle: String,
    pub role: String,
    pub lead: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogProject {
    pub id: String,
    pub name: String,
    pub path: String,
}

super::wire_enum!(RolePermissionMode, Default, {
    Default => "default", AcceptEdits => "acceptEdits", Auto => "auto", Bypass => "bypass",
});

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleRecord {
    pub id: String,
    pub handle: String,
    pub name: String,
    pub runtime: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    pub system_prompt: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub speed: Option<String>,
    pub permission_mode: Option<RolePermissionMode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrewRecord {
    pub id: String,
    pub name: String,
    pub conventions: Option<String>,
    pub slots: Vec<SlotRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotRecord {
    pub id: String,
    pub handle: String,
    pub role: String,
    pub lead: bool,
    pub runtime: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub speed: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub agents: Vec<UsageAgent>,
    pub refreshing: bool,
    pub active_claude_account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageAgent {
    pub name: String,
    pub value: Option<UsageValue>,
    pub error: Option<UsageUnavailableReason>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageValue {
    pub windows: Vec<UsageWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<crate::protocol::usage::UsageCredits>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub name: String,
    pub used_percent: f64,
    pub resets_at: Option<chrono::DateTime<chrono::Utc>>,
}

super::wire_enum!(UsageUnavailableReason, InvalidResponse, {
    SignIn => "signIn", KeychainDenied => "keychainDenied",
    KeychainUnavailable => "keychainUnavailable", ClaudeUnreachable => "claudeUnreachable",
    CodexNoAnswer => "codexNoAnswer", AntigravityNoAnswer => "antigravityNoAnswer",
    InvalidResponse => "invalidResponse",
});

impl From<crate::protocol::usage::AgentUsage> for UsageValue {
    fn from(usage: crate::protocol::usage::AgentUsage) -> Self {
        Self {
            windows: usage.windows.into_iter().map(Into::into).collect(),
            credits: usage.credits,
            updated_at: usage.updated_at,
        }
    }
}

impl From<UsageValue> for crate::protocol::usage::AgentUsage {
    fn from(usage: UsageValue) -> Self {
        Self {
            windows: usage.windows.into_iter().map(Into::into).collect(),
            credits: usage.credits,
            updated_at: usage.updated_at,
        }
    }
}

impl From<crate::protocol::usage::UsageWindow> for UsageWindow {
    fn from(window: crate::protocol::usage::UsageWindow) -> Self {
        Self {
            name: window.name,
            used_percent: window.used_percent,
            resets_at: window.resets_at,
        }
    }
}

impl From<UsageWindow> for crate::protocol::usage::UsageWindow {
    fn from(window: UsageWindow) -> Self {
        Self {
            name: window.name,
            used_percent: window.used_percent,
            resets_at: window.resets_at,
        }
    }
}

impl From<crate::protocol::usage::UnavailableReason> for UsageUnavailableReason {
    fn from(reason: crate::protocol::usage::UnavailableReason) -> Self {
        use crate::protocol::usage::UnavailableReason as Reason;
        match reason {
            Reason::SignIn => Self::SignIn,
            Reason::KeychainDenied => Self::KeychainDenied,
            Reason::KeychainUnavailable => Self::KeychainUnavailable,
            Reason::ClaudeUnreachable => Self::ClaudeUnreachable,
            Reason::CodexNoAnswer => Self::CodexNoAnswer,
            Reason::AntigravityNoAnswer => Self::AntigravityNoAnswer,
            Reason::InvalidResponse => Self::InvalidResponse,
        }
    }
}

impl From<UsageUnavailableReason> for crate::protocol::usage::UnavailableReason {
    fn from(reason: UsageUnavailableReason) -> Self {
        match reason.effective() {
            UsageUnavailableReason::SignIn => Self::SignIn,
            UsageUnavailableReason::KeychainDenied => Self::KeychainDenied,
            UsageUnavailableReason::KeychainUnavailable => Self::KeychainUnavailable,
            UsageUnavailableReason::ClaudeUnreachable => Self::ClaudeUnreachable,
            UsageUnavailableReason::CodexNoAnswer => Self::CodexNoAnswer,
            UsageUnavailableReason::AntigravityNoAnswer => Self::AntigravityNoAnswer,
            _ => Self::InvalidResponse,
        }
    }
}
