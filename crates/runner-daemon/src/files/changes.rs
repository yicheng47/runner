//! The Changes view's service (#1141): commits since base, changed files,
//! per-file diffs, working-tree git blob hashing, and viewed marks.

mod collect;
mod git;
mod parse;
mod working;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use runner_core::companion::TerminalBytes;
use runner_core::protocol::changes::{
    ChangedFile, ChangesBranches, ChangesCommits, ChangesDiff, ChangesFiles, ChangesTarget,
    ChangesTotals, ChangesViewed, CommitSummary, DiffBody, DiffCounts, DiffHunk, DiffOp,
    ModeChange, RemoteBranches, CHANGES_VIEWED_EVENT, MAX_COMMITS,
};
use runner_core::protocol::files::{FileStatus, BINARY_SNIFF_BYTES, MAX_READ_BYTES};

use super::fs;
use super::git::{Git, RepoPaths};
use super::FilesService;
use crate::error::{Error, Result};
use crate::AppCore;

use collect::*;
pub(crate) use git::worktree_auto_base;
use git::*;
use parse::*;
use working::*;

const EMPTY_TREE_SHA: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ViewedMarkKey {
    path: String,
    old_blob_id: String,
    new_blob_id: String,
}

/// In-memory store of viewed file marks keyed by checkout top path.
///
/// The boolean records whether the mark was made on `ChangesTarget::Uncommitted`
/// or `ChangesTarget::Branch` (sticky once true) so subsequent edits or commits
/// show as changed since viewed.
#[derive(Default)]
pub(crate) struct ViewedMarks {
    by_checkout: Mutex<HashMap<PathBuf, HashMap<ViewedMarkKey, bool>>>,
}

fn target_tracks_changes(target: &ChangesTarget) -> bool {
    matches!(target, ChangesTarget::Uncommitted | ChangesTarget::Branch)
}

impl ViewedMarks {
    pub(crate) fn set(
        &self,
        checkout_top: &Path,
        path: &str,
        old_blob_id: Option<&str>,
        new_blob_id: Option<&str>,
        tracks_changes: bool,
    ) {
        let key = ViewedMarkKey {
            path: path.to_owned(),
            old_blob_id: old_blob_id.unwrap_or("").to_owned(),
            new_blob_id: new_blob_id.unwrap_or("").to_owned(),
        };
        let mut by_checkout = self.by_checkout.lock().unwrap();
        let entry = by_checkout
            .entry(checkout_top.to_path_buf())
            .or_default()
            .entry(key)
            .or_insert(false);
        *entry |= tracks_changes;
    }

    pub(crate) fn clear(
        &self,
        checkout_top: &Path,
        path: &str,
        old_blob_id: Option<&str>,
        new_blob_id: Option<&str>,
    ) {
        let key = ViewedMarkKey {
            path: path.to_owned(),
            old_blob_id: old_blob_id.unwrap_or("").to_owned(),
            new_blob_id: new_blob_id.unwrap_or("").to_owned(),
        };
        let mut by_checkout = self.by_checkout.lock().unwrap();
        let Some(marks) = by_checkout.get_mut(checkout_top) else {
            return;
        };
        marks.remove(&key);
        if marks.is_empty() {
            by_checkout.remove(checkout_top);
        }
    }

    /// Computes `(viewed, changed_since_viewed)` for each `(checkout_rel_path, old_blob, new_blob)`.
    pub(crate) fn flags_for<'a>(
        &self,
        checkout_top: &Path,
        items: impl IntoIterator<Item = (&'a str, Option<&'a str>, Option<&'a str>)>,
        tracks_changes: bool,
    ) -> Vec<(bool, bool)> {
        let by_checkout = self.by_checkout.lock().unwrap();
        let Some(marks) = by_checkout.get(checkout_top) else {
            return items.into_iter().map(|_| (false, false)).collect();
        };
        let exact: HashSet<(&str, &str, &str)> = marks
            .keys()
            .map(|k| {
                (
                    k.path.as_str(),
                    k.old_blob_id.as_str(),
                    k.new_blob_id.as_str(),
                )
            })
            .collect();
        let tracked_paths: HashSet<&str> = if tracks_changes {
            marks
                .iter()
                .filter_map(|(k, &tracked)| tracked.then_some(k.path.as_str()))
                .collect()
        } else {
            HashSet::new()
        };
        items
            .into_iter()
            .map(|(path, old_blob_id, new_blob_id)| {
                let viewed =
                    exact.contains(&(path, old_blob_id.unwrap_or(""), new_blob_id.unwrap_or("")));
                let changed_since_viewed =
                    tracks_changes && !viewed && tracked_paths.contains(path);
                (viewed, changed_since_viewed)
            })
            .collect()
    }
}

/// Lists branch state, resolved base, and commits reachable from `HEAD` since base.
pub fn changes_commits(core: &AppCore, root: &Path, base: Option<&str>) -> Result<ChangesCommits> {
    let (git, repo) = resolve_checkout(core, root)?;
    let worktrees = git.worktrees(&repo.top).unwrap_or_default();
    let branch = try_run_git(
        &git,
        &repo.top,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )?
    .map(trimmed_utf8)
    .filter(|s| !s.is_empty());

    let Some(head_full) = try_run_git(
        &git,
        &repo.top,
        &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
    )?
    .map(trimmed_utf8)
    .filter(|s| !s.is_empty()) else {
        let base_missing = match base {
            Some(requested) => verify_branch_ref(&git, &repo.top, requested)?.is_none(),
            None => false,
        };
        return Ok(ChangesCommits {
            checkout_top: repo.top,
            branch,
            head_commit: None,
            base: None,
            auto_base: None,
            base_missing,
            commits: Vec::new(),
            total: 0,
            worktrees,
        });
    };

    let head_short = try_run_git(
        &git,
        &repo.top,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--short",
            "HEAD^{commit}",
        ],
    )?
    .map(trimmed_utf8)
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| short_sha(&head_full));

    let auto = resolve_auto_base(&git, &repo.top)?;
    let auto_base = auto.as_ref().map(|(name, _)| name.clone());
    let (used_base, merge_base, base_missing) = match base {
        None => (
            auto.as_ref().map(|(name, _)| name.clone()),
            auto.map(|(_, merge_base)| merge_base),
            false,
        ),
        Some(requested) => match verify_branch_ref(&git, &repo.top, requested)? {
            Some((short_name, full_ref)) => {
                let merge_base = try_run_git(&git, &repo.top, &["merge-base", "HEAD", &full_ref])?
                    .map(trimmed_utf8)
                    .filter(|s| !s.is_empty());
                (Some(short_name), merge_base, false)
            }
            None => (
                auto.as_ref().map(|(name, _)| name.clone()),
                auto.map(|(_, merge_base)| merge_base),
                true,
            ),
        },
    };

    let Some(merge_base) = merge_base else {
        return Ok(ChangesCommits {
            checkout_top: repo.top,
            branch,
            head_commit: Some(head_short),
            base: used_base,
            auto_base,
            base_missing,
            commits: Vec::new(),
            total: 0,
            worktrees,
        });
    };

    let range = format!("{merge_base}..HEAD");
    let count_raw = run_git(&git, &repo.top, &["rev-list", "--count", &range])?;
    let total = trimmed_utf8(count_raw)
        .parse::<usize>()
        .map_err(|error| Error::msg(format!("parse commit count: {error}")))?;

    let log_limit = format!("--max-count={MAX_COMMITS}");
    let log_raw = run_git(
        &git,
        &repo.top,
        &[
            "log",
            "-z",
            &log_limit,
            "--format=%H%x1f%h%x1f%s%x1f%an%x1f%aI%x1f%P",
            &range,
        ],
    )?;
    let commits = parse_commits_z(&log_raw)?;

    Ok(ChangesCommits {
        checkout_top: repo.top,
        branch,
        head_commit: Some(head_short),
        base: used_base,
        auto_base,
        base_missing,
        commits,
        total,
        worktrees,
    })
}

/// Lists local branches and remote-tracking branches grouped by remote, without `HEAD` symrefs.
pub fn changes_branches(core: &AppCore, root: &Path) -> Result<ChangesBranches> {
    let (git, repo) = resolve_checkout(core, root)?;
    let configured_remotes = list_remotes(&git, &repo.top)?;
    let mut remotes_by_len = configured_remotes;
    remotes_by_len.sort_by_key(|b| std::cmp::Reverse(b.len()));

    let raw = run_git(
        &git,
        &repo.top,
        &[
            "for-each-ref",
            "--format=%(HEAD)%00%(refname)%00%(symref)%00%(upstream)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;

    let mut local = Vec::new();
    let mut by_remote: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut upstream: Option<String> = None;

    for line in raw.split(|&b| b == b'\n').filter(|line| !line.is_empty()) {
        let mut parts = line.split(|&b| b == 0);
        let head = String::from_utf8_lossy(parts.next().unwrap_or_default());
        let refname = String::from_utf8_lossy(parts.next().unwrap_or_default());
        let symref = String::from_utf8_lossy(parts.next().unwrap_or_default());
        let up = String::from_utf8_lossy(parts.next().unwrap_or_default());

        if let Some(short) = refname.strip_prefix("refs/heads/") {
            if !short.is_empty() {
                local.push(short.to_owned());
            }
            if head.trim() == "*" && !up.is_empty() {
                upstream = Some(up.into_owned());
            }
        } else if let Some(short) = refname.strip_prefix("refs/remotes/") {
            if !symref.is_empty() {
                continue;
            }
            let matched = remotes_by_len
                .iter()
                .find_map(|remote| {
                    short
                        .strip_prefix(remote.as_str())
                        .and_then(|rest| rest.strip_prefix('/'))
                        .map(|branch| (remote.as_str(), branch))
                })
                .or_else(|| short.split_once('/'));
            let Some((remote, branch_part)) = matched else {
                continue;
            };
            if branch_part.is_empty() || branch_part == "HEAD" {
                continue;
            }
            by_remote
                .entry(remote.to_owned())
                .or_default()
                .push(short.to_owned());
        }
    }

    let mut remotes = Vec::with_capacity(by_remote.len());
    if let Some(origin_branches) = by_remote.remove("origin") {
        remotes.push(RemoteBranches {
            remote: "origin".to_owned(),
            branches: origin_branches,
        });
    }
    for (remote, branches) in by_remote {
        remotes.push(RemoteBranches { remote, branches });
    }

    Ok(ChangesBranches {
        local,
        remotes,
        upstream,
    })
}

/// Lists changed files under `root` for `target`, annotated with viewed marks and totals.
pub fn changes_files(
    core: &AppCore,
    root: &Path,
    target: &ChangesTarget,
    base: Option<&str>,
) -> Result<ChangesFiles> {
    let (git, repo) = resolve_checkout(core, root)?;
    let (_, entries) = collect_changed_entries(&git, &repo, root, target, base)?;

    let tracks_changes = target_tracks_changes(target);
    let flags = core.files.changes_viewed.flags_for(
        &repo.top,
        entries.iter().map(|entry| {
            (
                entry.checkout_rel_path.as_str(),
                entry.old_blob_id.as_deref(),
                entry.new_blob_id.as_deref(),
            )
        }),
        tracks_changes,
    );

    let mut totals = ChangesTotals::default();
    let mut files = Vec::with_capacity(entries.len());
    for (entry, (viewed, changed_since_viewed)) in entries.into_iter().zip(flags) {
        totals.files += 1;
        if viewed {
            totals.viewed += 1;
        }
        if let DiffCounts::Lines { added, removed } = entry.counts {
            totals.added = totals.added.saturating_add(added);
            totals.removed = totals.removed.saturating_add(removed);
        }

        files.push(ChangedFile {
            path: entry.root_rel_path,
            status: entry.status,
            counts: entry.counts,
            old_blob_id: entry.old_blob_id,
            new_blob_id: entry.new_blob_id,
            viewed,
            changed_since_viewed,
        });
    }

    Ok(ChangesFiles {
        checkout_top: repo.top,
        files,
        totals,
    })
}

/// Returns hunks, raw side bytes, or placeholder details for a single changed path inside `root`.
pub fn changes_diff(
    core: &AppCore,
    root: &Path,
    target: &ChangesTarget,
    path: &str,
    base: Option<&str>,
    want_bytes: bool,
) -> Result<ChangesDiff> {
    let (git, repo) = resolve_checkout(core, root)?;
    let (norm_rel, abs_path) = validate_relative_path(root, path)?;
    let (ctx, entries) = collect_changed_entries(&git, &repo, root, target, base)?;
    let entry = entries
        .into_iter()
        .find(|entry| entry.root_rel_path == norm_rel)
        .ok_or_else(|| Error::msg(format!("{norm_rel:?} has no changes in target")))?;

    if entry.old_mode == "160000" || entry.new_mode == "160000" {
        return Ok(ChangesDiff {
            path: entry.root_rel_path,
            status: entry.status,
            counts: entry.counts,
            old_blob_id: entry.old_blob_id.clone(),
            new_blob_id: entry.new_blob_id.clone(),
            mode_change: None,
            conflicted: entry.conflicted,
            body: DiffBody::Submodule {
                old_commit: entry.old_blob_id,
                new_commit: entry.new_blob_id,
            },
        });
    }

    let mode_change = mode_change_note(&entry.old_mode, &entry.new_mode);
    let old_size = match entry.old_blob_id.as_deref() {
        Some(oid) => Some(git_blob_size(&git, &repo.top, oid)?),
        None => None,
    };
    let new_size = if entry.new_blob_id.is_none() {
        None
    } else if matches!(target, ChangesTarget::Uncommitted) {
        Some(working_path_size(&abs_path)?)
    } else if let Some(oid) = entry.new_blob_id.as_deref() {
        Some(git_blob_size(&git, &repo.top, oid)?)
    } else {
        None
    };

    if old_size.is_some_and(|s| s > MAX_READ_BYTES) || new_size.is_some_and(|s| s > MAX_READ_BYTES)
    {
        return Ok(ChangesDiff {
            path: entry.root_rel_path,
            status: entry.status,
            counts: entry.counts,
            old_blob_id: entry.old_blob_id,
            new_blob_id: entry.new_blob_id,
            mode_change,
            conflicted: entry.conflicted,
            body: DiffBody::TooLarge { old_size, new_size },
        });
    }

    let old_bytes = match entry.old_blob_id.as_deref() {
        Some(oid) => git_cat_blob(&git, &repo.top, oid)?,
        None => Vec::new(),
    };
    let new_bytes = if entry.new_blob_id.is_none() {
        Vec::new()
    } else if matches!(target, ChangesTarget::Uncommitted) {
        read_working_bytes(&abs_path)?
    } else if let Some(oid) = entry.new_blob_id.as_deref() {
        git_cat_blob(&git, &repo.top, oid)?
    } else {
        Vec::new()
    };

    if old_bytes.len() as u64 > MAX_READ_BYTES || new_bytes.len() as u64 > MAX_READ_BYTES {
        return Ok(ChangesDiff {
            path: entry.root_rel_path,
            status: entry.status,
            counts: entry.counts,
            old_blob_id: entry.old_blob_id,
            new_blob_id: entry.new_blob_id,
            mode_change,
            conflicted: entry.conflicted,
            body: DiffBody::TooLarge {
                old_size: old_size
                    .or_else(|| (!old_bytes.is_empty()).then_some(old_bytes.len() as u64)),
                new_size: if new_bytes.len() as u64 > MAX_READ_BYTES {
                    Some(new_bytes.len() as u64)
                } else {
                    new_size
                },
            },
        });
    }

    if let Some(mc) = mode_change.as_ref() {
        if entry.old_blob_id.is_some()
            && entry.old_blob_id == entry.new_blob_id
            && !entry.conflicted
            && matches!(
                entry.counts,
                DiffCounts::Lines {
                    added: 0,
                    removed: 0
                }
            )
            && old_bytes == new_bytes
        {
            return Ok(ChangesDiff {
                path: entry.root_rel_path,
                status: entry.status,
                counts: entry.counts,
                old_blob_id: entry.old_blob_id,
                new_blob_id: entry.new_blob_id,
                mode_change: None,
                conflicted: false,
                body: DiffBody::ModeOnly {
                    old_mode: mc.old.clone(),
                    new_mode: mc.new.clone(),
                },
            });
        }
    }

    if matches!(entry.counts, DiffCounts::Binary)
        || is_binary_bytes(&old_bytes)
        || is_binary_bytes(&new_bytes)
    {
        let include_bytes =
            want_bytes && runner_core::protocol::files::is_image_path(&entry.root_rel_path);
        let old =
            (include_bytes && entry.old_blob_id.is_some()).then_some(TerminalBytes(old_bytes));
        let new =
            (include_bytes && entry.new_blob_id.is_some()).then_some(TerminalBytes(new_bytes));
        return Ok(ChangesDiff {
            path: entry.root_rel_path,
            status: entry.status,
            counts: entry.counts,
            old_blob_id: entry.old_blob_id,
            new_blob_id: entry.new_blob_id,
            mode_change,
            conflicted: entry.conflicted,
            body: DiffBody::Binary {
                old_size,
                new_size,
                old,
                new,
            },
        });
    }

    let hunks = match ctx {
        DiffContext::Uncommitted { .. } if entry.status == FileStatus::Untracked => {
            hunks_for_added_text(&new_bytes)
        }
        DiffContext::Uncommitted { .. }
            if entry.old_blob_id.is_none()
                && entry.status == FileStatus::Added
                && ctx.is_unborn()
                && !entry.in_index =>
        {
            hunks_for_added_text(&new_bytes)
        }
        DiffContext::Uncommitted { .. } if entry.raw_diff_needed => {
            diff_hunks_no_index(&git, &old_bytes, &new_bytes)?
        }
        DiffContext::Uncommitted { base_tree, .. } => {
            let mut args = vec![
                "diff",
                "-U3",
                "-M",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--no-relative",
                "--ignore-submodules=dirty",
                base_tree.as_str(),
                "--",
            ];
            if let Some(old_path) = entry.checkout_old_path.as_deref() {
                args.push(old_path);
            }
            args.push(&entry.checkout_rel_path);
            let raw = run_git(&git, &repo.top, &args)?;
            parse_hunks(&raw)?
        }
        DiffContext::Commit {
            parent_sha,
            commit_sha,
        } => {
            let mut args = vec![
                "diff",
                "-U3",
                "-M",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--no-relative",
                "--ignore-submodules=dirty",
                parent_sha.as_str(),
                commit_sha.as_str(),
                "--",
            ];
            if let Some(old_path) = entry.checkout_old_path.as_deref() {
                args.push(old_path);
            }
            args.push(&entry.checkout_rel_path);
            let raw = run_git(&git, &repo.top, &args)?;
            parse_hunks(&raw)?
        }
    };

    Ok(ChangesDiff {
        path: entry.root_rel_path,
        status: entry.status,
        counts: entry.counts,
        old_blob_id: entry.old_blob_id,
        new_blob_id: entry.new_blob_id,
        mode_change,
        conflicted: entry.conflicted,
        body: DiffBody::Text {
            old: TerminalBytes(old_bytes),
            new: TerminalBytes(new_bytes),
            hunks,
        },
    })
}

/// Sets or clears a viewed mark for a path and broadcasts [`CHANGES_VIEWED_EVENT`].
pub fn changes_set_viewed(
    core: &AppCore,
    root: &Path,
    target: &ChangesTarget,
    path: &str,
    old_blob_id: Option<&str>,
    new_blob_id: Option<&str>,
    viewed: bool,
) -> Result<()> {
    let (_git, repo) = resolve_checkout(core, root)?;
    let (norm_rel, _) = validate_relative_path(root, path)?;
    let checkout_rel_path = checkout_relative(&repo.prefix, &norm_rel);
    let tracks_changes = target_tracks_changes(target);

    if viewed {
        core.files.changes_viewed.set(
            &repo.top,
            &checkout_rel_path,
            old_blob_id,
            new_blob_id,
            tracks_changes,
        );
    } else {
        core.files
            .changes_viewed
            .clear(&repo.top, &checkout_rel_path, old_blob_id, new_blob_id);
    }

    core.events.emit(
        CHANGES_VIEWED_EVENT,
        &ChangesViewed {
            checkout_top: repo.top,
            path: checkout_rel_path,
            old_blob_id: old_blob_id.map(str::to_owned),
            new_blob_id: new_blob_id.map(str::to_owned),
            viewed,
        },
    );
    Ok(())
}

fn resolve_checkout(core: &AppCore, root: &Path) -> Result<(Git, RepoPaths)> {
    fs::check_root(root)?;
    let git = core
        .files
        .git()
        .ok_or_else(|| Error::msg("git is not on PATH"))?;
    let repo = git
        .rev_parse(root)
        .ok_or_else(|| Error::msg(format!("{} is not inside a git checkout", root.display())))?;
    Ok((git, repo))
}

fn checkout_relative(prefix: &str, root_rel: &str) -> String {
    if prefix.is_empty() {
        root_rel.to_owned()
    } else {
        format!("{prefix}{root_rel}")
    }
}

/// Validates a file path relative to `root` without following a leaf symlink so
/// tracked symlinks pointing outside the root remain inspectable.
pub(crate) fn validate_relative_path(root: &Path, relative: &str) -> Result<(String, PathBuf)> {
    fs::check_root(root)?;
    let refused = || Error::msg(format!("{relative:?} is outside the root"));
    if relative.is_empty() || relative.contains('\0') {
        return Err(refused());
    }
    #[cfg(windows)]
    let normalized = relative.replace('\\', "/");
    #[cfg(not(windows))]
    let normalized = relative.to_owned();
    if normalized.starts_with('/') {
        return Err(refused());
    }

    let mut path = root.to_path_buf();
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        let mut components = Path::new(part).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(name)), None) => {
                parts.push(part);
                path.push(name);
            }
            _ => return Err(refused()),
        }
    }
    if parts.is_empty() {
        return Err(refused());
    }

    let canonical_root = root
        .canonicalize()
        .map_err(|error| Error::msg(format!("{}: {error}", root.display())))?;
    let mut existing = path.parent().ok_or_else(refused)?;
    let canonical_parent = loop {
        match existing.canonicalize() {
            Ok(canonical) => break canonical,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                match existing.parent() {
                    Some(parent) if parent.starts_with(root) => existing = parent,
                    _ => return Err(refused()),
                }
            }
            Err(error) => {
                return Err(Error::msg(format!("{}: {error}", existing.display())));
            }
        }
    };
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(refused());
    }
    Ok((parts.join("/"), path))
}

#[cfg(test)]
#[path = "changes_tests.rs"]
mod tests;
