//! Pure selection, grouping, picker and keyboard helpers for Changes (#1141, #1345).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use gpui::Hsla;
use runner_core::protocol::changes::{
    ChangedFile, ChangesBranches, ChangesCommits, ChangesTarget, CommitSummary, ModeChange,
};
use runner_core::protocol::files::{
    FileStatus, FilesRoot, FilesRootSource, WorktreeEntry, WorktreeKind,
};

pub(crate) use crate::surfaces::files::picker::CheckoutSessionLabel;
use crate::surfaces::files::picker::{same_checkout_path, shorten_checkout_path};
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

pub(crate) fn is_viewing_other_checkout(root: &FilesRoot) -> bool {
    if root.source != FilesRootSource::Saved {
        return false;
    }
    match (&root.git, &root.own_checkout_top) {
        (Some(git), Some(own_top)) => !same_checkout_path(&git.top, own_top),
        (Some(_), None) => true,
        _ => false,
    }
}

pub(crate) fn live_worktrees(worktrees: &[WorktreeEntry]) -> Vec<&WorktreeEntry> {
    worktrees.iter().filter(|wt| !wt.prunable).collect()
}

pub(crate) fn display_base_ref(base_ref: &str) -> &str {
    base_ref
        .strip_prefix("refs/heads/")
        .or_else(|| base_ref.strip_prefix("refs/remotes/"))
        .unwrap_or(base_ref)
}

/// Returns `(chip_text, is_custom_ref)`. When `is_custom_ref` is true, the chip
/// draws in the primary text color; Auto draws in muted text.
pub(crate) fn base_chip_label(
    chosen_base: Option<&str>,
    commits: Option<&ChangesCommits>,
) -> (String, bool) {
    if let Some(chosen) = chosen_base.filter(|b| !b.is_empty())
        && !commits.is_some_and(|c| c.base_missing)
    {
        return (display_base_ref(chosen).to_owned(), true);
    }
    let resolved = commits.and_then(|c| c.auto_base.as_deref().or(c.base.as_deref()));
    match resolved {
        Some(base) => (
            crate::i18n::tf(
                "changes.base_auto_resolved",
                &[("base", display_base_ref(base).to_owned())],
            ),
            false,
        ),
        None => (
            crate::i18n::text("changes.base_auto_none").to_owned(),
            false,
        ),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BasePickerSection {
    Auto,
    Local,
    Remote(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BasePickerItem {
    pub(crate) section: BasePickerSection,
    /// `None` for Auto; `Some(full_ref)` (`refs/heads/…` or `refs/remotes/…`) for a branch.
    pub(crate) base_ref: Option<String>,
    pub(crate) label: String,
    pub(crate) hint: Option<String>,
    pub(crate) current: bool,
}

fn canonicalize_legacy_chosen_ref(chosen: &str, branches: &ChangesBranches) -> String {
    if chosen.starts_with("refs/") {
        return chosen.to_owned();
    }
    let starts_with_remote = branches.remotes.iter().any(|group| {
        chosen
            .strip_prefix(group.remote.as_str())
            .is_some_and(|rest| rest.starts_with('/'))
    });
    let has_local = branches.local.iter().any(|b| b == chosen);
    let has_remote = branches
        .remotes
        .iter()
        .any(|group| group.branches.iter().any(|b| b == chosen));
    if starts_with_remote && has_remote {
        format!("refs/remotes/{chosen}")
    } else if has_local {
        format!("refs/heads/{chosen}")
    } else if has_remote {
        format!("refs/remotes/{chosen}")
    } else {
        chosen.to_owned()
    }
}

pub(crate) fn build_base_picker_items(
    chosen_base: Option<&str>,
    auto_base: Option<&str>,
    current_branch: Option<&str>,
    branches: &ChangesBranches,
    auto_label: &str,
    none_label: &str,
) -> Vec<BasePickerItem> {
    let chosen_canonical = chosen_base
        .filter(|b| !b.is_empty())
        .map(|b| canonicalize_legacy_chosen_ref(b, branches));
    let upstream_canonical = branches
        .upstream
        .as_deref()
        .filter(|u| !u.is_empty())
        .map(|u| canonicalize_legacy_chosen_ref(u, branches));
    let mut items = Vec::new();
    items.push(BasePickerItem {
        section: BasePickerSection::Auto,
        base_ref: None,
        label: auto_label.to_owned(),
        hint: Some(auto_base.unwrap_or(none_label).to_owned()),
        current: chosen_canonical.is_none(),
    });
    for branch in &branches.local {
        let full_ref = format!("refs/heads/{branch}");
        if Some(branch.as_str()) == current_branch
            || upstream_canonical.as_deref() == Some(full_ref.as_str())
        {
            continue;
        }
        items.push(BasePickerItem {
            section: BasePickerSection::Local,
            current: chosen_canonical.as_deref() == Some(full_ref.as_str()),
            base_ref: Some(full_ref),
            label: branch.clone(),
            hint: None,
        });
    }
    for group in &branches.remotes {
        for branch in &group.branches {
            let full_ref = format!("refs/remotes/{branch}");
            if upstream_canonical.as_deref() == Some(full_ref.as_str()) {
                continue;
            }
            items.push(BasePickerItem {
                section: BasePickerSection::Remote(group.remote.clone()),
                current: chosen_canonical.as_deref() == Some(full_ref.as_str()),
                base_ref: Some(full_ref),
                label: branch.clone(),
                hint: None,
            });
        }
    }
    items
}

pub(crate) fn filter_base_picker_items(items: &[BasePickerItem], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return (0..items.len()).collect();
    }
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.label.to_lowercase().contains(&query)
                || item
                    .hint
                    .as_deref()
                    .is_some_and(|hint| hint.to_lowercase().contains(&query))
        })
        .map(|(index, _)| index)
        .collect()
}

pub(crate) fn default_base_highlight(
    items: &[BasePickerItem],
    visible: &[usize],
    query: &str,
) -> usize {
    if !query.trim().is_empty() {
        return 0;
    }
    visible
        .iter()
        .position(|&index| items[index].current)
        .unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CheckoutPickerItem {
    pub(crate) path: PathBuf,
    pub(crate) kind: WorktreeKind,
    pub(crate) branch_label: String,
    pub(crate) commit: Option<String>,
    pub(crate) uncommitted: Option<usize>,
    pub(crate) short_path: String,
    pub(crate) full_path: String,
    pub(crate) session_label: Option<CheckoutSessionLabel>,
    pub(crate) current: bool,
}

pub(crate) fn build_checkout_picker_items(
    current_top: &Path,
    worktrees: &[WorktreeEntry],
    session_labels: &BTreeMap<PathBuf, CheckoutSessionLabel>,
    home: Option<&Path>,
    detached_label: &str,
) -> Vec<CheckoutPickerItem> {
    let mut live = live_worktrees(worktrees);
    live.sort_by(|a, b| {
        (a.kind != WorktreeKind::Main)
            .cmp(&(b.kind != WorktreeKind::Main))
            .then_with(|| {
                a.branch
                    .as_deref()
                    .unwrap_or("\u{ffff}")
                    .cmp(b.branch.as_deref().unwrap_or("\u{ffff}"))
            })
            .then_with(|| a.path.cmp(&b.path))
    });
    live.into_iter()
        .map(|wt| {
            let branch_label = match (&wt.branch, &wt.commit) {
                (Some(branch), _) => branch.clone(),
                (None, Some(sha)) => {
                    crate::i18n::tf("changes.detached_at", &[("sha", sha.clone())])
                }
                (None, None) => detached_label.to_owned(),
            };
            CheckoutPickerItem {
                path: wt.path.clone(),
                kind: wt.kind,
                branch_label,
                commit: wt.commit.clone(),
                uncommitted: wt.uncommitted,
                short_path: shorten_checkout_path(&wt.path, home),
                full_path: wt.path.to_string_lossy().into_owned(),
                session_label: session_labels.get(&wt.path).cloned(),
                current: same_checkout_path(&wt.path, current_top),
            }
        })
        .collect()
}

pub(crate) fn filter_checkout_picker_items(
    items: &[CheckoutPickerItem],
    query: &str,
) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return (0..items.len()).collect();
    }
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.branch_label.to_lowercase().contains(&query)
                || item.short_path.to_lowercase().contains(&query)
                || item.full_path.to_lowercase().contains(&query)
        })
        .map(|(index, _)| index)
        .collect()
}

pub(crate) fn default_checkout_highlight(
    items: &[CheckoutPickerItem],
    visible: &[usize],
    query: &str,
) -> usize {
    if !query.trim().is_empty() {
        return 0;
    }
    visible
        .iter()
        .position(|&index| items[index].current)
        .unwrap_or(0)
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
