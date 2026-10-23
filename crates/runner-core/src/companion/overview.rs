use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{wire_enum, SessionStatus};

wire_enum!(MissionState, Stopped, { Running => "running", Stopped => "stopped" });
wire_enum!(SessionKind, Chat, { Chat => "chat", Agent => "agent", Shell => "shell" });
wire_enum!(SplitDirection, Row, { Row => "row", Column => "column" });
wire_enum!(SidebarNodeKind, Tab, { Project => "project", Tab => "tab", Mission => "mission" });

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub projects: Vec<Project>,
    pub tabs: Vec<Tab>,
    pub missions: Vec<Mission>,
    pub sessions: BTreeMap<String, Session>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar: Option<Vec<SidebarNode>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarNode {
    pub kind: SidebarNodeKind,
    pub id: String,
    pub parent: Option<String>,
    pub position: i64,
    pub pinned_position: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
    pub id: String,
    pub project: Option<String>,
    pub name: String,
    pub pinned: bool,
    pub panes: Pane,
    pub shells: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Pane {
    Split {
        split: SplitDirection,
        sizes: [f64; 2],
        a: Box<Pane>,
        b: Box<Pane>,
    },
    Leaf {
        #[serde(deserialize_with = "Option::<String>::deserialize")]
        session: Option<String>,
    },
    Unknown(serde_json::Value),
}

impl Pane {
    pub fn effective(&self) -> Self {
        match self {
            Self::Unknown(_) => Self::Leaf { session: None },
            known => known.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mission {
    pub id: String,
    pub project: Option<String>,
    pub title: String,
    pub crew: String,
    pub state: MissionState,
    pub pinned: bool,
    pub members: Vec<Member>,
    pub asks: Vec<Ask>,
    pub shells: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub handle: String,
    pub session: String,
    pub lead: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub question: String,
    pub from: String,
    pub prompt: String,
    pub choices: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub kind: SessionKind,
    pub title: String,
    pub runtime: Option<String>,
    pub cwd: Option<String>,
    #[serde(flatten)]
    pub status: SessionStatus,
}
