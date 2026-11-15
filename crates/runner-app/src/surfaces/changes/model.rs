//! Pure selection, grouping, comparison-label and keyboard helpers for Changes (#1141).

use std::collections::{BTreeMap, BTreeSet};

use gpui::Hsla;
use runner_core::protocol::changes::{
    ChangedFile, ChangesCommits, ChangesTarget, CommitSummary, ModeChange,
};
use runner_core::protocol::files::FileStatus;

use crate::theme;

/// Which row in the Commits section is active.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SelectedTarget {
    Branch,
    Uncommitted,
    Commit(String),
}

impl SelectedTarget {
    pub(crate) fn to_protocol(&self) -> ChangesTarget {
        match self {
            Self::Branch => ChangesTarget::Branch,
            Self::Uncommitted => ChangesTarget::Uncommitted,
            Self::Commit(id) => ChangesTarget::Commit(id.clone()),
        }
    }
}

/// Which of the three keyboard focus places holds focus inside Changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FocusPlace {
    Commits,
    #[default]
    Files,
    Diff,
}

impl FocusPlace {
    pub(crate) fn toggle_list_section(self) -> Self {
        match self {
            Self::Commits => Self::Files,
            Self::Files | Self::Diff => Self::Commits,
        }
    }
}

/// Whether the Commits section includes the `All changes` row.
pub(crate) fn has_all_changes(commits: &ChangesCommits) -> bool {
    commits.base.is_some() && commits.total > 0
}

/// Opening selection rule: Uncommitted changes when it has changes, otherwise
/// All changes when the branch has commits since its base, otherwise Uncommitted.
pub(crate) fn choose_initial_target(
    commits: &ChangesCommits,
    uncommitted_count: usize,
) -> SelectedTarget {
    if uncommitted_count > 0 {
        SelectedTarget::Uncommitted
    } else if has_all_changes(commits) {
        SelectedTarget::Branch
    } else {
        SelectedTarget::Uncommitted
    }
}

/// Keeps `current` across a live refresh while its row still exists in Commits.
pub(crate) fn reconcile_target(
    current: &SelectedTarget,
    commits: &ChangesCommits,
    uncommitted_count: usize,
) -> SelectedTarget {
    match current {
        SelectedTarget::Branch if has_all_changes(commits) => SelectedTarget::Branch,
        SelectedTarget::Uncommitted => SelectedTarget::Uncommitted,
        SelectedTarget::Commit(id) if commits.commits.iter().any(|c| &c.id == id) => {
            SelectedTarget::Commit(id.clone())
        }
        _ => choose_initial_target(commits, uncommitted_count),
    }
}

fn tree_segments(path: &str) -> impl Iterator<Item = (bool, &str)> + '_ {
    let mut parts = path.split('/').filter(|s| !s.is_empty()).peekable();
    std::iter::from_fn(move || {
        let part = parts.next()?;
        let is_leaf = parts.peek().is_none();
        Some((is_leaf, part))
    })
}

pub(crate) fn cmp_display_path(a: &str, b: &str, tree_mode: bool) -> std::cmp::Ordering {
    if !tree_mode {
        return a.cmp(b);
    }
    tree_segments(a).cmp(tree_segments(b))
}

/// Keeps the selected path when it still has changes; otherwise advances to the
/// next surviving file in display order (or the last surviving file when the
/// list shrank past it), even when earlier rows disappear in the same refresh.
pub(crate) fn reconcile_file_selection(
    previous_path: Option<&str>,
    files: &[ChangedFile],
    list_rows: &[ListRow],
    tree_mode: bool,
) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    if let Some(path) = previous_path
        && files.iter().any(|file| file.path == path)
    {
        return Some(path.to_owned());
    }
    let visible: Vec<&str> = list_rows
        .iter()
        .filter_map(|row| match row {
            ListRow::File { path, .. } => Some(path.as_str()),
            ListRow::Folder { .. } => None,
        })
        .collect();
    let candidates = if !visible.is_empty() {
        visible
    } else {
        let mut all: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        all.sort_by(|a, b| cmp_display_path(a, b, tree_mode));
        all
    };
    let Some(prev) = previous_path else {
        return Some(candidates[0].to_owned());
    };
    candidates
        .iter()
        .copied()
        .find(|candidate| cmp_display_path(candidate, prev, tree_mode).is_gt())
        .or_else(|| candidates.last().copied())
        .map(str::to_owned)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CommitRow {
    AllChanges { count: usize },
    Uncommitted { count: usize },
    Commit(CommitSummary),
}

impl CommitRow {
    pub(crate) fn target(&self) -> SelectedTarget {
        match self {
            Self::AllChanges { .. } => SelectedTarget::Branch,
            Self::Uncommitted { .. } => SelectedTarget::Uncommitted,
            Self::Commit(summary) => SelectedTarget::Commit(summary.id.clone()),
        }
    }
}

pub(crate) fn build_commit_rows(
    commits: &ChangesCommits,
    branch_file_count: usize,
    uncommitted_file_count: usize,
) -> Vec<CommitRow> {
    let mut rows = Vec::with_capacity(2 + commits.commits.len());
    if has_all_changes(commits) {
        rows.push(CommitRow::AllChanges {
            count: branch_file_count,
        });
    }
    rows.push(CommitRow::Uncommitted {
        count: uncommitted_file_count,
    });
    if commits.base.is_some() {
        for commit in &commits.commits {
            rows.push(CommitRow::Commit(commit.clone()));
        }
    }
    rows
}

pub(crate) fn older_commits_hidden(commits: &ChangesCommits) -> usize {
    if commits.base.is_some() {
        commits.total.saturating_sub(commits.commits.len())
    } else {
        0
    }
}

pub(crate) fn branch_header_label(commits: &ChangesCommits) -> String {
    if let Some(branch) = &commits.branch {
        branch.clone()
    } else if let Some(short) = &commits.head_commit {
        crate::i18n::tf("changes.detached_at", &[("sha", short.clone())])
    } else {
        crate::i18n::text("ui.detached").to_owned()
    }
}

/// Comparison label shown in the diff header.
pub(crate) fn comparison_label(target: &SelectedTarget, commits: &ChangesCommits) -> String {
    match target {
        SelectedTarget::Branch => {
            let head = commits
                .branch
                .as_deref()
                .or(commits.head_commit.as_deref())
                .unwrap_or("HEAD");
            let base = commits.base.as_deref().unwrap_or("main");
            crate::i18n::tf(
                "changes.compare_vs",
                &[("head", head.to_owned()), ("base", base.to_owned())],
            )
        }
        SelectedTarget::Uncommitted => {
            if commits.head_commit.is_none() {
                crate::i18n::text("changes.working_tree_unborn").to_owned()
            } else {
                crate::i18n::text("changes.working_tree_vs_head").to_owned()
            }
        }
        SelectedTarget::Commit(id) => {
            let summary = commits.commits.iter().find(|c| &c.id == id);
            let short = summary
                .map(|c| c.short_id.as_str())
                .unwrap_or_else(|| &id[..id.len().min(7)])
                .to_owned();
            match summary.map(|c| c.parent_count).unwrap_or(1) {
                0 => crate::i18n::tf("changes.first_commit", &[("sha", short)]),
                n if n > 1 => crate::i18n::tf("changes.commit_vs_first_parent", &[("sha", short)]),
                _ => crate::i18n::tf("changes.commit_vs_parent", &[("sha", short)]),
            }
        }
    }
}

/// Abbreviates a multi-segment folder path as `first/…/last` when it has three
/// or more segments (e.g. `crates/runner-daemon/src` -> `crates/…/src`).
pub(crate) fn abbreviate_folder(path: &str) -> Option<String> {
    let (folder, _) = path.rsplit_once('/')?;
    if folder.is_empty() {
        return None;
    }
    let parts: Vec<&str> = folder.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 3 {
        Some(format!("{}/…/{}", parts[0], parts[parts.len() - 1]))
    } else {
        Some(folder.to_owned())
    }
}

pub(crate) fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

pub(crate) fn status_letter_and_color(status: &FileStatus) -> (&'static str, Hsla) {
    match status {
        FileStatus::Modified => ("M", theme::warning()),
        FileStatus::Added => ("A", theme::accent()),
        FileStatus::Renamed { .. } => ("R", theme::info()),
        FileStatus::Deleted => ("D", theme::danger()),
        FileStatus::Untracked => ("U", theme::accent()),
        FileStatus::Unmerged => ("!", theme::danger()),
    }
}

pub(crate) fn is_crlf_only_change(
    old: &[u8],
    new: &[u8],
    mode_change: Option<&ModeChange>,
    conflicted: bool,
) -> bool {
    if conflicted || mode_change.is_some() || old == new {
        return false;
    }
    normalize_crlf(old) == normalize_crlf(new)
}

fn normalize_crlf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            out.push(b'\n');
            i += 2;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// One visible row in the files list (either flat or grouped into a folder tree).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ListRow {
    Folder {
        path: String,
        name: String,
        depth: usize,
        expanded: bool,
    },
    File {
        file_index: usize,
        path: String,
        name: String,
        folder_hint: Option<String>,
        depth: usize,
    },
}

impl ListRow {
    pub(crate) fn path(&self) -> &str {
        match self {
            Self::Folder { path, .. } | Self::File { path, .. } => path,
        }
    }
}

pub(crate) fn build_flat_rows(files: &[ChangedFile]) -> Vec<ListRow> {
    files
        .iter()
        .enumerate()
        .map(|(file_index, file)| ListRow::File {
            file_index,
            path: file.path.clone(),
            name: file_name(&file.path).to_owned(),
            folder_hint: abbreviate_folder(&file.path),
            depth: 0,
        })
        .collect()
}

#[derive(Default)]
struct TreeNode {
    folders: BTreeMap<String, TreeNode>,
    files: Vec<(usize, String)>,
}

pub(crate) fn build_tree_rows(
    files: &[ChangedFile],
    collapsed_folders: &BTreeSet<String>,
) -> Vec<ListRow> {
    let mut root = TreeNode::default();
    for (file_index, file) in files.iter().enumerate() {
        let mut parts = file.path.split('/').filter(|p| !p.is_empty()).peekable();
        let mut node = &mut root;
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                node.files.push((file_index, part.to_owned()));
            } else {
                node = node.folders.entry(part.to_owned()).or_default();
            }
        }
    }
    let mut rows = Vec::with_capacity(files.len() * 2);
    emit_tree_node(&root, "", 0, collapsed_folders, files, &mut rows);
    rows
}

fn emit_tree_node(
    node: &TreeNode,
    prefix: &str,
    depth: usize,
    collapsed: &BTreeSet<String>,
    files: &[ChangedFile],
    rows: &mut Vec<ListRow>,
) {
    for (folder_name, child) in &node.folders {
        let folder_path = if prefix.is_empty() {
            folder_name.clone()
        } else {
            format!("{prefix}/{folder_name}")
        };
        let expanded = !collapsed.contains(&folder_path);
        rows.push(ListRow::Folder {
            path: folder_path.clone(),
            name: folder_name.clone(),
            depth,
            expanded,
        });
        if expanded {
            emit_tree_node(child, &folder_path, depth + 1, collapsed, files, rows);
        }
    }
    for (file_index, name) in &node.files {
        rows.push(ListRow::File {
            file_index: *file_index,
            path: files[*file_index].path.clone(),
            name: name.clone(),
            folder_hint: None,
            depth,
        });
    }
}
