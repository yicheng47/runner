use super::files::WorktreeKind;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Requested git worktree for a new chat, terminal, or mission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct WorktreeSpec {
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

/// Default base ref used when creating a worktree without an explicit base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeDefaultBase {
    /// Display name, e.g. `"origin/main"`, `"main"`, or `"HEAD"`.
    pub branch: String,
    pub main_checkout: PathBuf,
}

/// Result of checking a candidate worktree branch name before creation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WorktreeNameCheck {
    NewBranch { base: String, path: PathBuf },
    ExistingBranch { commit: String, path: PathBuf },
    AlreadyCheckedOut { path: PathBuf },
    Invalid { reason: String },
}

/// A repository worktree reported by `worktree_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeItem {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub kind: WorktreeKind,
    pub locked: bool,
    pub prunable: bool,
    /// True for linked worktrees directly inside `<main_checkout>/.worktrees/`.
    pub managed: bool,
}

/// Result of `worktree_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeList {
    pub main_checkout: PathBuf,
    pub worktrees: Vec<WorktreeItem>,
}

/// Result of `worktree_create`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeCreated {
    pub path: PathBuf,
    pub branch: String,
    pub main_checkout: PathBuf,
    /// True when a new worktree folder was added; false when reusing an existing checkout.
    pub created: bool,
}

/// Result of `worktree_remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRemoved {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// How a missing worktree's branch resolves for restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingWorktreeBranchState {
    Surviving,
    Ambiguous,
    Gone,
}

/// Checkout state of a working directory, including missing worktrees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WorktreeCheckout {
    Git {
        top: PathBuf,
        main_checkout: PathBuf,
        kind: WorktreeKind,
        managed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        commit: Option<String>,
        missing: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        missing_branch: Option<MissingWorktreeBranchState>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        candidate_branches: Vec<String>,
    },
    Plain {
        missing: bool,
    },
}
