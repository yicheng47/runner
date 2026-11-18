use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{wire_enum, SessionStatus};

wire_enum!(MissionState, Stopped, { Running => "running", Stopped => "stopped" });
wire_enum!(MissionPhase, Working, {
    NeedsYou => "needsYou", Failed => "failed", Interrupted => "interrupted",
    Working => "working", Settled => "settled", Ended => "ended",
});
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
    /// The palette the host answers its agents' color queries with, so a
    /// viewer draws their output against the background they styled it for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_palette: Option<TerminalPalette>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalPalette {
    pub background: [u8; 3],
    pub foreground: [u8; 3],
    pub cursor: [u8; 3],
    pub cursor_accent: [u8; 3],
    pub selection: [u8; 3],
    pub ansi: [[u8; 3]; 16],
}

impl From<crate::protocol::terminal::TerminalPalette> for TerminalPalette {
    fn from(palette: crate::protocol::terminal::TerminalPalette) -> Self {
        Self {
            background: palette.background,
            foreground: palette.foreground,
            cursor: palette.cursor,
            cursor_accent: palette.cursor_accent,
            selection: palette.selection,
            ansi: palette.ansi,
        }
    }
}

impl From<TerminalPalette> for crate::protocol::terminal::TerminalPalette {
    fn from(palette: TerminalPalette) -> Self {
        Self {
            background: palette.background,
            foreground: palette.foreground,
            cursor: palette.cursor,
            cursor_accent: palette.cursor_accent,
            selection: palette.selection,
            ansi: palette.ansi,
        }
    }
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
    /// The chat the mission reports to (#748); older hosts send none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_chat: Option<LinkedChat>,
    /// The host's derived mission state (#748); older hosts send none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<MissionPhase>,
    /// Working directory on the host (#1213); older hosts send none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Host-derived git worktree state and auto-detected base (#1213); older hosts send none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout: Option<Checkout>,
}

impl MissionPhase {
    /// The local protocol's phase, or None for a value this build does not know.
    pub fn protocol(&self) -> Option<crate::protocol::MissionPhase> {
        use crate::protocol::MissionPhase as P;
        match self {
            Self::NeedsYou => Some(P::NeedsYou),
            Self::Failed => Some(P::Failed),
            Self::Interrupted => Some(P::Interrupted),
            Self::Working => Some(P::Working),
            Self::Settled => Some(P::Settled),
            Self::Ended => Some(P::Ended),
            Self::Unrecognized(_) => None,
        }
    }
}

impl From<crate::protocol::MissionPhase> for MissionPhase {
    fn from(phase: crate::protocol::MissionPhase) -> Self {
        use crate::protocol::MissionPhase as P;
        match phase {
            P::NeedsYou => Self::NeedsYou,
            P::Failed => Self::Failed,
            P::Interrupted => Self::Interrupted,
            P::Working | P::Unknown => Self::Working,
            P::Settled => Self::Settled,
            P::Ended => Self::Ended,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedChat {
    pub session_id: String,
    pub title: String,
    pub archived: bool,
    pub reports: bool,
}

impl From<crate::protocol::LinkedChat> for LinkedChat {
    fn from(chat: crate::protocol::LinkedChat) -> Self {
        Self {
            session_id: chat.session_id,
            title: chat.title,
            archived: chat.archived,
            reports: chat.reports,
        }
    }
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
    /// Host-derived git worktree state and auto-detected base (#1213); older hosts send none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout: Option<Checkout>,
}

wire_enum!(CheckoutKind, Main, {
    Main => "main",
    Linked => "linked",
});

wire_enum!(MissingBranchState, Gone, {
    Surviving => "surviving",
    Ambiguous => "ambiguous",
    Gone => "gone",
});

/// Host-derived checkout state carried over Companion `Overview` (#1213).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Checkout {
    Git {
        top: String,
        main_checkout: String,
        kind: CheckoutKind,
        managed: bool,
        branch: Option<String>,
        commit: Option<String>,
        missing: bool,
        missing_branch: Option<MissingBranchState>,
        candidate_branches: Vec<String>,
        auto_base: Option<String>,
    },
    Plain {
        missing: bool,
    },
    Unknown(serde_json::Value),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitCheckoutWire {
    #[serde(default = "git_checkout_tag")]
    status: String,
    top: String,
    main_checkout: String,
    kind: CheckoutKind,
    managed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    missing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    missing_branch: Option<MissingBranchState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    candidate_branches: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_base: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlainCheckoutWire {
    #[serde(default = "plain_checkout_tag")]
    status: String,
    #[serde(default)]
    missing: bool,
}

fn git_checkout_tag() -> String {
    "git".to_owned()
}

fn plain_checkout_tag() -> String {
    "plain".to_owned()
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Serialize for Checkout {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Git {
                top,
                main_checkout,
                kind,
                managed,
                branch,
                commit,
                missing,
                missing_branch,
                candidate_branches,
                auto_base,
            } => GitCheckoutWire {
                status: git_checkout_tag(),
                top: top.clone(),
                main_checkout: main_checkout.clone(),
                kind: kind.clone(),
                managed: *managed,
                branch: branch.clone(),
                commit: commit.clone(),
                missing: *missing,
                missing_branch: missing_branch.clone(),
                candidate_branches: candidate_branches.clone(),
                auto_base: auto_base.clone(),
            }
            .serialize(serializer),
            Self::Plain { missing } => PlainCheckoutWire {
                status: plain_checkout_tag(),
                missing: *missing,
            }
            .serialize(serializer),
            Self::Unknown(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Checkout {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let Some(status) = value.get("status").and_then(serde_json::Value::as_str) else {
            return Err(serde::de::Error::missing_field("status"));
        };
        match status {
            "git" => {
                let wire: GitCheckoutWire =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                Ok(Self::Git {
                    top: wire.top,
                    main_checkout: wire.main_checkout,
                    kind: wire.kind,
                    managed: wire.managed,
                    branch: wire.branch,
                    commit: wire.commit,
                    missing: wire.missing,
                    missing_branch: wire.missing_branch,
                    candidate_branches: wire.candidate_branches,
                    auto_base: wire.auto_base,
                })
            }
            "plain" => {
                let wire: PlainCheckoutWire =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                Ok(Self::Plain {
                    missing: wire.missing,
                })
            }
            _ => Ok(Self::Unknown(value)),
        }
    }
}

impl Checkout {
    pub fn from_protocol(
        checkout: &crate::protocol::worktree::WorktreeCheckout,
        auto_base: Option<String>,
    ) -> Self {
        use crate::protocol::files::WorktreeKind as ProtoKind;
        use crate::protocol::worktree::{MissingWorktreeBranchState, WorktreeCheckout};
        match checkout {
            WorktreeCheckout::Git {
                top,
                main_checkout,
                kind,
                managed,
                branch,
                commit,
                missing,
                missing_branch,
                candidate_branches,
            } => Self::Git {
                top: top.to_string_lossy().into_owned(),
                main_checkout: main_checkout.to_string_lossy().into_owned(),
                kind: match kind {
                    ProtoKind::Main => CheckoutKind::Main,
                    ProtoKind::Linked => CheckoutKind::Linked,
                },
                managed: *managed,
                branch: branch.clone(),
                commit: commit.clone(),
                missing: *missing,
                missing_branch: missing_branch.map(|state| match state {
                    MissingWorktreeBranchState::Surviving => MissingBranchState::Surviving,
                    MissingWorktreeBranchState::Ambiguous => MissingBranchState::Ambiguous,
                    MissingWorktreeBranchState::Gone => MissingBranchState::Gone,
                }),
                candidate_branches: candidate_branches.clone(),
                auto_base,
            },
            WorktreeCheckout::Plain { missing } => Self::Plain { missing: *missing },
        }
    }

    pub fn to_protocol(
        &self,
    ) -> Option<(crate::protocol::worktree::WorktreeCheckout, Option<String>)> {
        use crate::protocol::files::WorktreeKind as ProtoKind;
        use crate::protocol::worktree::{MissingWorktreeBranchState, WorktreeCheckout};
        use std::path::PathBuf;
        match self {
            Self::Git {
                top,
                main_checkout,
                kind,
                managed,
                branch,
                commit,
                missing,
                missing_branch,
                candidate_branches,
                auto_base,
            } => {
                let proto_kind = match kind {
                    CheckoutKind::Main => ProtoKind::Main,
                    CheckoutKind::Linked => ProtoKind::Linked,
                    CheckoutKind::Unrecognized(_) => return None,
                };
                let proto_missing_branch = missing_branch.as_ref().map(|state| match state {
                    MissingBranchState::Surviving => MissingWorktreeBranchState::Surviving,
                    MissingBranchState::Ambiguous => MissingWorktreeBranchState::Ambiguous,
                    MissingBranchState::Gone | MissingBranchState::Unrecognized(_) => {
                        MissingWorktreeBranchState::Gone
                    }
                });
                Some((
                    WorktreeCheckout::Git {
                        top: PathBuf::from(top),
                        main_checkout: PathBuf::from(main_checkout),
                        kind: proto_kind,
                        managed: *managed,
                        branch: branch.clone(),
                        commit: commit.clone(),
                        missing: *missing,
                        missing_branch: proto_missing_branch,
                        candidate_branches: candidate_branches.clone(),
                    },
                    auto_base.clone(),
                ))
            }
            Self::Plain { missing } => Some((WorktreeCheckout::Plain { missing: *missing }, None)),
            Self::Unknown(_) => None,
        }
    }
}
