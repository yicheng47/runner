use serde::{Deserialize, Serialize};
#[derive(Debug, serde::Serialize, Clone, Deserialize)]
pub struct ProjectDeleteOutcome {
    pub archived_mission_ids: Vec<String>,
    pub archived_session_ids: Vec<String>,
}

/// The project a new session or mission joins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectScope {
    /// The project that owns the working directory, if any.
    Infer,
    /// No project, whatever the working directory.
    Root,
    Project(String),
}

impl ProjectScope {
    /// For callers that may pass only a directory: the CLI and socket tools.
    pub fn or_infer(project_id: Option<String>) -> Self {
        project_id.map_or(Self::Infer, Self::Project)
    }

    /// For starts from inside the app, which belong to the place they were
    /// started from: a project, or the sidebar root.
    pub fn or_root(project_id: Option<String>) -> Self {
        project_id.map_or(Self::Root, Self::Project)
    }

    pub fn project_id(&self) -> Option<&str> {
        match self {
            Self::Project(project_id) => Some(project_id),
            Self::Infer | Self::Root => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub position: i64,
    pub created_at: String,
}
