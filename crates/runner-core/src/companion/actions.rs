use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ActionRequest {
    Catalog {
        id: u64,
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
            "catalog" => None,
            "folders" => fields!(path: Option<String> => "a string or null"),
            "startChat" => {
                fields!(role: Option<String> => "a string or null", runtime: Option<String> => "a string or null", model: Option<String> => "a string or null", effort: Option<String> => "a string or null", speed: Option<String> => "a string or null", project: Option<String> => "a string or null", cwd: Option<String> => "a string or null", cols: u16 => "an unsigned 16-bit integer", rows: u16 => "an unsigned 16-bit integer")
            }
            "startShell" => {
                fields!(project: Option<String> => "a string or null", cwd: Option<String> => "a string or null", cols: u16 => "an unsigned 16-bit integer", rows: u16 => "an unsigned 16-bit integer")
            }
            "stop" | "close" => fields!(session: String => "a string"),
            "resume" | "restart" => {
                fields!(session: String => "a string", cols: u16 => "an unsigned 16-bit integer", rows: u16 => "an unsigned 16-bit integer")
            }
            "fork" => {
                fields!(session: String => "a string", title: Option<String> => "a string or null", cols: u16 => "an unsigned 16-bit integer", rows: u16 => "an unsigned 16-bit integer")
            }
            "rename" => {
                fields!(target: ActionTarget => "an item target", name: String => "a string")
            }
            "archive" => fields!(target: ActionTarget => "an item target"),
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
            "stopMission" => fields!(mission: String => "a string"),
            "resumeMission" => {
                fields!(mission: String => "a string", cols: u16 => "an unsigned 16-bit integer", rows: u16 => "an unsigned 16-bit integer")
            }
            _ => None,
        }
    }
    pub const NAMES: &[&str] = &[
        "catalog",
        "folders",
        "startChat",
        "startShell",
        "stop",
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
    ];
    pub fn id(&self) -> u64 {
        match self {
            Self::Catalog { id, .. } => *id,
            Self::Folders { id, .. } => *id,
            Self::StartChat { id, .. } => *id,
            Self::StartShell { id, .. } => *id,
            Self::Stop { id, .. } => *id,
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
        }
    }
    pub fn set_id(&mut self, value: u64) {
        match self {
            Self::Catalog { id, .. } => *id = value,
            Self::Folders { id, .. } => *id = value,
            Self::StartChat { id, .. } => *id = value,
            Self::StartShell { id, .. } => *id = value,
            Self::Stop { id, .. } => *id = value,
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
