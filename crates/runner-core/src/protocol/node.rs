use serde::{Deserialize, Serialize};
#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct NodeTabUpsertInput {
    pub id: String,
    /// Scope for a NEW tab node; an existing node keeps its stored
    /// placement — reparenting/reordering go through `node_move` only,
    /// so a layout/name write can never scramble sibling positions.
    pub parent_id: Option<String>,
    pub name: String,
    pub layout: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Project,
    Tab,
    Mission,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeRow {
    pub id: String,
    pub parent_id: Option<String>,
    pub position: i64,
    /// Column is named `type` in SQL; `type` is a Rust keyword.
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub name: Option<String>,
    pub ref_id: Option<String>,
    pub layout: Option<String>,
    pub pinned_position: Option<i64>,
    pub last_completed_at: Option<String>,
    pub last_viewed_at: Option<String>,
    pub created_at: String,
}
