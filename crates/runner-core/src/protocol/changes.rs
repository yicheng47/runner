//! The Changes view's protocol (#1141): commits since base, changed files,
//! per-file diffs, and viewed marks served by `runnerd`.
//!
//! Every path in a commit or file list is relative to the requested root and
//! uses `/` separators on every platform, matching [`super::files`]. Events on
//! [`CHANGES_VIEWED_EVENT`] key by the checkout's top path and paths relative
//! to that checkout top so windows on different subfolders share marks.

use super::files::FileStatus;
use crate::companion::TerminalBytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Broadcast when a viewed mark is set or cleared for a checkout.
pub const CHANGES_VIEWED_EVENT: &str = "changes/viewed";

/// Maximum number of branch commits `changes_commits` returns at once.
pub const MAX_COMMITS: usize = 200;

/// What `changes_files` and `changes_diff` compare.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ChangesTarget {
    /// Working tree (staged, unstaged, and untracked files) against `HEAD`.
    Uncommitted,
    /// A single commit against its first parent, or the empty tree for a root commit.
    Commit(String),
}

/// Summary of one commit since the resolved base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitSummary {
    pub id: String,
    pub short_id: String,
    pub subject: String,
    pub author: String,
    pub date: DateTime<Utc>,
    pub parent_count: usize,
}

/// Branch state, resolved base, and commits reachable from `HEAD` since base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangesCommits {
    pub checkout_top: PathBuf,
    /// Current branch short name; `None` when `HEAD` is detached.
    pub branch: Option<String>,
    /// `HEAD`'s short commit id; `None` before the repository's first commit.
    pub head_commit: Option<String>,
    /// Base ref name (`origin/main`, `origin/master`, local `main`, etc.), if resolved.
    pub base: Option<String>,
    /// Commits reachable from `HEAD` and not from `base`, newest first, at most [`MAX_COMMITS`].
    pub commits: Vec<CommitSummary>,
    /// Total commits reachable from `HEAD` and not from `base`, including older ones not returned.
    pub total: usize,
}

/// Added and removed line counts, or a binary indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiffCounts {
    Lines { added: u64, removed: u64 },
    Binary,
}

/// One changed path returned by `changes_files`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    /// Path relative to the requested root, using `/` separators.
    pub path: String,
    pub status: FileStatus,
    pub counts: DiffCounts,
    pub old_blob_id: Option<String>,
    pub new_blob_id: Option<String>,
    #[serde(default)]
    pub viewed: bool,
    #[serde(default)]
    pub changed_since_viewed: bool,
}

/// Aggregate counts across all rows returned by `changes_files`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangesTotals {
    pub files: usize,
    pub viewed: usize,
    pub added: u64,
    pub removed: u64,
}

/// All changed files for a target under the requested root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangesFiles {
    pub checkout_top: PathBuf,
    pub files: Vec<ChangedFile>,
    pub totals: ChangesTotals,
}

/// File mode change (`100644` -> `100755`, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModeChange {
    pub old: String,
    pub new: String,
}

/// Run-length encoded line operations within a unified diff hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiffOp {
    Context { count: u32 },
    Removed { count: u32 },
    Added { count: u32 },
}

/// One `@@` hunk with its 1-based line ranges and line operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    /// Full `@@ -a,b +c,d @@ ...` row emitted by git.
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub ops: Vec<DiffOp>,
}

/// Diff contents or placeholder reason when line diffs are unavailable or inapplicable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiffBody {
    Text {
        /// Raw bytes before the change (empty when added or untracked); CRLF is preserved.
        old: TerminalBytes,
        /// Raw bytes after the change (empty when deleted); CRLF is preserved.
        new: TerminalBytes,
        hunks: Vec<DiffHunk>,
    },
    Binary {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    /// Either side exceeded [`super::files::MAX_READ_BYTES`].
    TooLarge {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    ModeOnly {
        old_mode: String,
        new_mode: String,
    },
    Submodule {
        old_commit: Option<String>,
        new_commit: Option<String>,
    },
}

/// Full diff payload for a single file returned by `changes_diff`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangesDiff {
    pub path: String,
    pub status: FileStatus,
    pub counts: DiffCounts,
    pub old_blob_id: Option<String>,
    pub new_blob_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_change: Option<ModeChange>,
    #[serde(default)]
    pub conflicted: bool,
    pub body: DiffBody,
}

/// `changes/viewed`: a viewed mark was set or cleared on a checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangesViewed {
    pub checkout_top: PathBuf,
    /// Path relative to `checkout_top`, using `/` separators.
    pub path: String,
    pub old_blob_id: Option<String>,
    pub new_blob_id: Option<String>,
    pub viewed: bool,
}
