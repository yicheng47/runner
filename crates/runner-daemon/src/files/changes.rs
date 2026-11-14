//! The Changes view's service (#1141): commits since base, changed files,
//! per-file diffs, working-tree git blob hashing, and viewed marks.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use runner_core::companion::TerminalBytes;
use runner_core::protocol::changes::{
    ChangedFile, ChangesCommits, ChangesDiff, ChangesFiles, ChangesTarget, ChangesTotals,
    ChangesViewed, CommitSummary, DiffBody, DiffCounts, DiffHunk, DiffOp, ModeChange,
    CHANGES_VIEWED_EVENT, MAX_COMMITS,
};
use runner_core::protocol::files::{FileStatus, BINARY_SNIFF_BYTES, MAX_READ_BYTES};

use super::fs;
use super::git::{Git, RepoPaths};
use crate::error::{Error, Result};
use crate::AppCore;

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
/// (sticky once true) so subsequent edits show as changed since viewed.
#[derive(Default)]
pub(crate) struct ViewedMarks {
    by_checkout: Mutex<HashMap<PathBuf, HashMap<ViewedMarkKey, bool>>>,
}

impl ViewedMarks {
    pub(crate) fn set(
        &self,
        checkout_top: &Path,
        path: &str,
        old_blob_id: Option<&str>,
        new_blob_id: Option<&str>,
        uncommitted: bool,
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
        *entry |= uncommitted;
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
        is_uncommitted: bool,
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
        let uncommitted_paths: HashSet<&str> = if is_uncommitted {
            marks
                .iter()
                .filter_map(|(k, &uncommitted)| uncommitted.then_some(k.path.as_str()))
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
                    is_uncommitted && !viewed && uncommitted_paths.contains(path);
                (viewed, changed_since_viewed)
            })
            .collect()
    }
}

/// Lists branch state, resolved base, and commits reachable from `HEAD` since base.
pub fn changes_commits(core: &AppCore, root: &Path) -> Result<ChangesCommits> {
    let (git, repo) = resolve_checkout(core, root)?;
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
        return Ok(ChangesCommits {
            checkout_top: repo.top,
            branch,
            head_commit: None,
            base: None,
            commits: Vec::new(),
            total: 0,
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

    let Some((base_name, merge_base)) = resolve_base(&git, &repo.top)? else {
        return Ok(ChangesCommits {
            checkout_top: repo.top,
            branch,
            head_commit: Some(head_short),
            base: None,
            commits: Vec::new(),
            total: 0,
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
        base: Some(base_name),
        commits,
        total,
    })
}

/// Lists changed files under `root` for `target`, annotated with viewed marks and totals.
pub fn changes_files(core: &AppCore, root: &Path, target: &ChangesTarget) -> Result<ChangesFiles> {
    let (git, repo) = resolve_checkout(core, root)?;
    let (_, entries) = collect_changed_entries(&git, &repo, root, target)?;

    let is_uncommitted = matches!(target, ChangesTarget::Uncommitted);
    let flags = core.files.changes_viewed.flags_for(
        &repo.top,
        entries.iter().map(|entry| {
            (
                entry.checkout_rel_path.as_str(),
                entry.old_blob_id.as_deref(),
                entry.new_blob_id.as_deref(),
            )
        }),
        is_uncommitted,
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
) -> Result<ChangesDiff> {
    let (git, repo) = resolve_checkout(core, root)?;
    let (norm_rel, abs_path) = validate_relative_path(root, path)?;
    let (ctx, entries) = collect_changed_entries(&git, &repo, root, target)?;
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
        return Ok(ChangesDiff {
            path: entry.root_rel_path,
            status: entry.status,
            counts: entry.counts,
            old_blob_id: entry.old_blob_id,
            new_blob_id: entry.new_blob_id,
            mode_change,
            conflicted: entry.conflicted,
            body: DiffBody::Binary { old_size, new_size },
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
    let uncommitted = matches!(target, ChangesTarget::Uncommitted);

    if viewed {
        core.files.changes_viewed.set(
            &repo.top,
            &checkout_rel_path,
            old_blob_id,
            new_blob_id,
            uncommitted,
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

fn git_cmd(git: &Git, dir: &Path) -> Command {
    let mut cmd = git.command(dir);
    cmd.args([
        "--no-optional-locks",
        "--no-pager",
        "--literal-pathspecs",
        "-c",
        "core.quotePath=false",
        "-c",
        "diff.relative=false",
    ]);
    cmd
}

fn run_git(git: &Git, dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = git_cmd(git, dir)
        .args(args)
        .output()
        .map_err(|error| Error::msg(format!("run git: {error}")))?;
    if !output.status.success() {
        return Err(Error::msg(format!(
            "git {}: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

fn try_run_git(git: &Git, dir: &Path, args: &[&str]) -> Result<Option<Vec<u8>>> {
    let output = git_cmd(git, dir)
        .args(args)
        .output()
        .map_err(|error| Error::msg(format!("run git: {error}")))?;
    if output.status.success() {
        Ok(Some(output.stdout))
    } else {
        Ok(None)
    }
}

fn run_git_diff_allow_changes(git: &Git, dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = git_cmd(git, dir)
        .args(args)
        .output()
        .map_err(|error| Error::msg(format!("run git: {error}")))?;
    if matches!(output.status.code(), Some(0 | 1)) {
        Ok(output.stdout)
    } else {
        Err(Error::msg(format!(
            "git {}: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn write_blob_to_temp(
    git: &Git,
    top: &Path,
    oid: &str,
    tmp: &mut tempfile::NamedTempFile,
) -> Result<()> {
    let out_file = tmp
        .as_file()
        .try_clone()
        .map_err(|error| Error::msg(format!("clone temp file: {error}")))?;
    let output = git_cmd(git, top)
        .args(["cat-file", "blob", oid])
        .stdout(Stdio::from(out_file))
        .output()
        .map_err(|error| Error::msg(format!("run git: {error}")))?;
    if !output.status.success() {
        return Err(Error::msg(format!(
            "git cat-file blob {oid}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn trimmed_utf8(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).trim().to_owned()
}

fn short_sha(full: &str) -> String {
    full.chars().take(7).collect()
}

fn resolve_base(git: &Git, top: &Path) -> Result<Option<(String, String)>> {
    let ref_exists = |full_ref: &str| -> Result<bool> {
        let query = format!("{full_ref}^{{commit}}");
        Ok(try_run_git(git, top, &["rev-parse", "--verify", "--quiet", &query])?.is_some())
    };

    let mut chosen: Option<(String, String)> = None;
    if ref_exists("refs/remotes/origin/HEAD")? {
        let name = match try_run_git(
            git,
            top,
            &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
        )? {
            Some(target) => {
                let target = trimmed_utf8(target);
                target
                    .strip_prefix("refs/remotes/")
                    .unwrap_or(&target)
                    .to_owned()
            }
            None => "origin/HEAD".to_owned(),
        };
        chosen = Some((name, "refs/remotes/origin/HEAD".to_owned()));
    } else if ref_exists("refs/remotes/origin/main")? {
        chosen = Some((
            "origin/main".to_owned(),
            "refs/remotes/origin/main".to_owned(),
        ));
    } else if ref_exists("refs/remotes/origin/master")? {
        chosen = Some((
            "origin/master".to_owned(),
            "refs/remotes/origin/master".to_owned(),
        ));
    } else if !has_origin_remote(git, top)? {
        if ref_exists("refs/heads/main")? {
            chosen = Some(("main".to_owned(), "refs/heads/main".to_owned()));
        } else if ref_exists("refs/heads/master")? {
            chosen = Some(("master".to_owned(), "refs/heads/master".to_owned()));
        }
    }

    let Some((name, full_ref)) = chosen else {
        return Ok(None);
    };
    let Some(merge_base) = try_run_git(git, top, &["merge-base", "HEAD", &full_ref])? else {
        return Ok(None);
    };
    let merge_base = trimmed_utf8(merge_base);
    if merge_base.is_empty() {
        return Ok(None);
    }
    Ok(Some((name, merge_base)))
}

fn has_origin_remote(git: &Git, top: &Path) -> Result<bool> {
    let remotes = run_git(git, top, &["remote"])?;
    let text = String::from_utf8_lossy(&remotes);
    Ok(text.lines().any(|line| line.trim() == "origin"))
}

enum DiffContext {
    Uncommitted {
        base_tree: String,
        unborn: bool,
    },
    Commit {
        parent_sha: String,
        commit_sha: String,
    },
}

impl DiffContext {
    fn is_unborn(&self) -> bool {
        matches!(self, Self::Uncommitted { unborn: true, .. })
    }
}

struct InternalDiffEntry {
    checkout_rel_path: String,
    root_rel_path: String,
    checkout_old_path: Option<String>,
    status: FileStatus,
    counts: DiffCounts,
    old_mode: String,
    new_mode: String,
    old_blob_id: Option<String>,
    new_blob_id: Option<String>,
    conflicted: bool,
    in_index: bool,
    raw_diff_needed: bool,
}

fn collect_changed_entries(
    git: &Git,
    repo: &RepoPaths,
    root: &Path,
    target: &ChangesTarget,
) -> Result<(DiffContext, Vec<InternalDiffEntry>)> {
    match target {
        ChangesTarget::Uncommitted => collect_uncommitted_entries(git, repo, root),
        ChangesTarget::Commit(id) => collect_commit_entries(git, repo, root, id),
    }
}

fn collect_commit_entries(
    git: &Git,
    repo: &RepoPaths,
    root: &Path,
    commit_input: &str,
) -> Result<(DiffContext, Vec<InternalDiffEntry>)> {
    if commit_input.is_empty() || commit_input.starts_with('-') {
        return Err(Error::msg(format!("invalid commit id: {commit_input:?}")));
    }
    let query = format!("{commit_input}^{{commit}}");
    let commit_sha = trimmed_utf8(run_git(
        git,
        &repo.top,
        &["rev-parse", "--verify", "--quiet", &query],
    )?);
    if commit_sha.is_empty() {
        return Err(Error::msg(format!("unknown commit: {commit_input:?}")));
    }

    let parents_line = trimmed_utf8(run_git(
        git,
        &repo.top,
        &["rev-list", "--parents", "-n", "1", &commit_sha],
    )?);
    let parent_sha = parents_line
        .split_whitespace()
        .nth(1)
        .unwrap_or(EMPTY_TREE_SHA)
        .to_owned();

    let diff_raw = run_git(
        git,
        root,
        &[
            "diff",
            "--raw",
            "--numstat",
            "-M",
            "--no-abbrev",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-relative",
            "--ignore-submodules=dirty",
            "-z",
            &parent_sha,
            &commit_sha,
            "--",
            ".",
        ],
    )?;

    let parsed = parse_raw_and_numstat_z(&diff_raw)?;
    let mut entries = Vec::with_capacity(parsed.len());
    for item in parsed {
        let Some(root_rel_path) = relativize(&repo.prefix, &item.path) else {
            continue;
        };
        let status =
            raw_status_to_file_status(&item.status, item.old_path.as_deref(), &repo.prefix);
        entries.push(InternalDiffEntry {
            checkout_rel_path: item.path,
            root_rel_path,
            checkout_old_path: item.old_path,
            status,
            counts: item.counts,
            old_mode: item.old_mode,
            new_mode: item.new_mode,
            old_blob_id: non_zero_oid(item.old_blob),
            new_blob_id: non_zero_oid(item.new_blob),
            conflicted: false,
            in_index: true,
            raw_diff_needed: false,
        });
    }
    entries.sort_by(|a, b| a.root_rel_path.cmp(&b.root_rel_path));
    Ok((
        DiffContext::Commit {
            parent_sha,
            commit_sha,
        },
        entries,
    ))
}

struct PendingUntracked {
    checkout_rel: String,
    root_rel_path: String,
    raw: WorkingRawInfo,
    replaces_deleted_idx: Option<usize>,
}

fn collect_uncommitted_entries(
    git: &Git,
    repo: &RepoPaths,
    root: &Path,
) -> Result<(DiffContext, Vec<InternalDiffEntry>)> {
    let head_sha = try_run_git(
        git,
        &repo.top,
        &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
    )?
    .map(trimmed_utf8)
    .filter(|s| !s.is_empty());
    let unborn = head_sha.is_none();
    let base_tree = head_sha.unwrap_or_else(|| EMPTY_TREE_SHA.to_owned());

    let diff_raw = run_git(
        git,
        root,
        &[
            "diff",
            "--raw",
            "--numstat",
            "-M",
            "--no-abbrev",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-relative",
            "--ignore-submodules=dirty",
            "-z",
            &base_tree,
            "--",
            ".",
        ],
    )?;

    let status_raw = run_git(
        git,
        root,
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--ignored=no",
            "--",
            ".",
        ],
    )?;
    let (unmerged_paths, untracked_paths) = parse_unmerged_and_untracked(&status_raw, &repo.prefix);

    let parsed = parse_raw_and_numstat_z(&diff_raw)?;
    let mut entries = Vec::with_capacity(parsed.len() + untracked_paths.len());
    let mut entry_index: HashMap<String, usize> =
        HashMap::with_capacity(parsed.len() + unmerged_paths.len());
    let mut seen_checkout_paths = HashSet::with_capacity(parsed.len() + untracked_paths.len());
    let mut git_paths_to_hash: Vec<String> = Vec::new();
    let mut raw_paths_to_hash: Vec<String> = Vec::new();

    for item in parsed {
        let Some(root_rel_path) = relativize(&repo.prefix, &item.path) else {
            continue;
        };
        seen_checkout_paths.insert(item.path.clone());
        let is_unmerged = unmerged_paths.contains(&item.path);
        let status = if is_unmerged {
            FileStatus::Unmerged
        } else {
            raw_status_to_file_status(&item.status, item.old_path.as_deref(), &repo.prefix)
        };

        let old_blob_id = non_zero_oid(item.old_blob);
        let index_or_git_blob = non_zero_oid(item.new_blob);
        let new_blob_id = match index_or_git_blob {
            Some(oid) => {
                if item.new_mode != "000000" && item.new_mode != "160000" {
                    if let Some((_, None)) = inspect_working_meta(&repo.top.join(&item.path)) {
                        raw_paths_to_hash.push(item.path.clone());
                    }
                }
                Some(oid)
            }
            None if item.new_mode == "000000" => None,
            None if item.new_mode == "160000" => {
                let sub_dir = repo.top.join(&item.path);
                try_run_git(
                    git,
                    &sub_dir,
                    &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
                )?
                .map(trimmed_utf8)
                .filter(|s| !s.is_empty())
            }
            None => match inspect_working_meta(&repo.top.join(&item.path)) {
                Some((_, Some(sym_oid))) => Some(sym_oid),
                Some((_, None)) => {
                    git_paths_to_hash.push(item.path.clone());
                    raw_paths_to_hash.push(item.path.clone());
                    None
                }
                None => None,
            },
        };

        let idx = entries.len();
        entry_index.insert(item.path.clone(), idx);
        entries.push(InternalDiffEntry {
            checkout_rel_path: item.path,
            root_rel_path,
            checkout_old_path: item.old_path,
            status,
            counts: item.counts,
            old_mode: item.old_mode,
            new_mode: item.new_mode,
            old_blob_id,
            new_blob_id,
            conflicted: is_unmerged,
            in_index: true,
            raw_diff_needed: false,
        });
    }

    for checkout_rel in &unmerged_paths {
        if seen_checkout_paths.contains(checkout_rel) {
            continue;
        }
        let Some(root_rel_path) = relativize(&repo.prefix, checkout_rel) else {
            continue;
        };
        seen_checkout_paths.insert(checkout_rel.clone());
        let (old_mode, old_blob_id) = if unborn {
            ("000000".to_owned(), None)
        } else {
            lookup_head_tree_entry(git, &repo.top, checkout_rel)?
                .unwrap_or_else(|| ("000000".to_owned(), None))
        };
        let meta = inspect_working_meta(&repo.top.join(checkout_rel));
        let new_mode = meta
            .as_ref()
            .map(|(mode, _)| (*mode).to_owned())
            .unwrap_or_else(|| "000000".to_owned());
        let new_blob_id = match meta {
            Some((_, Some(sym_oid))) => Some(sym_oid),
            Some((_, None)) => {
                git_paths_to_hash.push(checkout_rel.clone());
                raw_paths_to_hash.push(checkout_rel.clone());
                None
            }
            None => None,
        };

        let idx = entries.len();
        entry_index.insert(checkout_rel.clone(), idx);
        entries.push(InternalDiffEntry {
            checkout_rel_path: checkout_rel.clone(),
            root_rel_path,
            checkout_old_path: None,
            status: FileStatus::Unmerged,
            counts: DiffCounts::Lines {
                added: 0,
                removed: 0,
            },
            old_mode,
            new_mode,
            old_blob_id,
            new_blob_id,
            conflicted: true,
            in_index: true,
            raw_diff_needed: false,
        });
    }

    let mut pending_untracked = Vec::with_capacity(untracked_paths.len());
    let mut seen_untracked: HashSet<String> = HashSet::with_capacity(untracked_paths.len());
    for checkout_rel in untracked_paths {
        if !seen_untracked.insert(checkout_rel.clone()) {
            continue;
        }
        let Some(root_rel_path) = relativize(&repo.prefix, &checkout_rel) else {
            continue;
        };
        let deleted_idx = entry_index
            .get(&checkout_rel)
            .copied()
            .filter(|&idx| entries[idx].status == FileStatus::Deleted && !entries[idx].conflicted);
        if seen_checkout_paths.contains(&checkout_rel) && deleted_idx.is_none() {
            continue;
        }
        let Some(raw) = inspect_working_raw(&repo.top.join(&checkout_rel)) else {
            continue;
        };
        if raw.symlink_blob_id.is_none() {
            git_paths_to_hash.push(checkout_rel.clone());
        }
        pending_untracked.push(PendingUntracked {
            checkout_rel,
            root_rel_path,
            raw,
            replaces_deleted_idx: deleted_idx,
        });
    }

    let git_hashes = hash_working_files_with_git(git, &repo.top, &git_paths_to_hash, false)?;
    let raw_hashes = hash_working_files_with_git(git, &repo.top, &raw_paths_to_hash, true)?;

    for entry in &mut entries {
        if entry.new_blob_id.is_none() && entry.new_mode != "000000" && entry.new_mode != "160000" {
            entry.new_blob_id = git_hashes.get(&entry.checkout_rel_path).cloned();
        }
        if entry.old_mode != "160000" && entry.new_mode != "160000" {
            if let (Some(git_oid), Some(raw_oid)) = (
                entry.new_blob_id.as_deref(),
                raw_hashes.get(&entry.checkout_rel_path).map(String::as_str),
            ) {
                if git_oid != raw_oid {
                    entry.raw_diff_needed = true;
                    entry.counts = no_index_numstat(
                        git,
                        &repo.top,
                        entry.old_blob_id.as_deref(),
                        &repo.top.join(&entry.checkout_rel_path),
                        None,
                    )?;
                }
            }
        }
    }

    let mut remove_indices = HashSet::new();
    for pending in pending_untracked {
        let abs_path = repo.top.join(&pending.checkout_rel);
        let new_blob_id = pending
            .raw
            .symlink_blob_id
            .clone()
            .or_else(|| git_hashes.get(&pending.checkout_rel).cloned());
        let Some(new_blob_id) = new_blob_id else {
            continue;
        };

        if let Some(idx) = pending.replaces_deleted_idx {
            let entry = &mut entries[idx];
            if entry.old_blob_id.as_deref() == Some(new_blob_id.as_str())
                && entry.old_mode == pending.raw.mode
            {
                remove_indices.insert(idx);
                continue;
            }
            let counts = if entry.old_mode == "160000" || pending.raw.mode == "160000" {
                DiffCounts::Lines {
                    added: 0,
                    removed: 0,
                }
            } else {
                no_index_numstat(
                    git,
                    &repo.top,
                    entry.old_blob_id.as_deref(),
                    &abs_path,
                    pending.raw.symlink_target_bytes.as_deref(),
                )?
            };
            entry.status = FileStatus::Modified;
            entry.new_mode = pending.raw.mode.to_owned();
            entry.new_blob_id = Some(new_blob_id);
            entry.counts = counts;
            entry.in_index = false;
            entry.raw_diff_needed = true;
            continue;
        }

        let status = if unborn {
            FileStatus::Added
        } else {
            FileStatus::Untracked
        };
        let counts = if pending.raw.binary {
            DiffCounts::Binary
        } else {
            DiffCounts::Lines {
                added: pending.raw.added_lines,
                removed: 0,
            }
        };
        entries.push(InternalDiffEntry {
            checkout_rel_path: pending.checkout_rel,
            root_rel_path: pending.root_rel_path,
            checkout_old_path: None,
            status,
            counts,
            old_mode: "000000".to_owned(),
            new_mode: pending.raw.mode.to_owned(),
            old_blob_id: None,
            new_blob_id: Some(new_blob_id),
            conflicted: false,
            in_index: false,
            raw_diff_needed: false,
        });
    }

    if !remove_indices.is_empty() {
        let mut idx = 0usize;
        entries.retain(|_| {
            let keep = !remove_indices.contains(&idx);
            idx += 1;
            keep
        });
    }

    entries.sort_by(|a, b| a.root_rel_path.cmp(&b.root_rel_path));
    Ok((DiffContext::Uncommitted { base_tree, unborn }, entries))
}

fn lookup_head_tree_entry(
    git: &Git,
    top: &Path,
    checkout_rel: &str,
) -> Result<Option<(String, Option<String>)>> {
    let Some(out) = try_run_git(git, top, &["ls-tree", "-z", "HEAD", "--", checkout_rel])? else {
        return Ok(None);
    };
    let text = String::from_utf8_lossy(&out);
    let Some(first) = text.split('\0').find(|part| !part.is_empty()) else {
        return Ok(None);
    };
    let Some((meta, _path)) = first.split_once('\t') else {
        return Ok(None);
    };
    let mut parts = meta.split_whitespace();
    let Some(mode) = parts.next() else {
        return Ok(None);
    };
    let _kind = parts.next();
    let Some(oid) = parts.next() else {
        return Ok(None);
    };
    Ok(Some((mode.to_owned(), non_zero_oid(oid.to_owned()))))
}

fn parse_unmerged_and_untracked(bytes: &[u8], prefix: &str) -> (HashSet<String>, Vec<String>) {
    let mut fields = bytes.split(|&b| b == 0).filter(|part| !part.is_empty());
    let mut unmerged = HashSet::new();
    let mut untracked = Vec::new();
    while let Some(field) = fields.next() {
        let field = String::from_utf8_lossy(field);
        let Some((kind, rest)) = field.split_once(' ') else {
            continue;
        };
        match kind {
            "2" => {
                // Rename/copy entries consume one extra NUL-separated `origPath`.
                let _ = fields.next();
            }
            "u" => {
                if let [_, _, _, _, _, _, _, _, _, path] =
                    rest.splitn(10, ' ').collect::<Vec<_>>()[..]
                {
                    if path.starts_with(prefix) && !path.ends_with('/') {
                        unmerged.insert(path.to_owned());
                    }
                }
            }
            // Skip trailing `/` entries (nested git repositories that are not submodules).
            "?" if rest.starts_with(prefix) && !rest.ends_with('/') => {
                untracked.push(rest.to_owned());
            }
            _ => {}
        }
    }
    (unmerged, untracked)
}

fn relativize(prefix: &str, checkout_path: &str) -> Option<String> {
    checkout_path.strip_prefix(prefix).map(str::to_owned)
}

fn non_zero_oid(oid: String) -> Option<String> {
    if oid.is_empty() || oid.bytes().all(|b| b == b'0') {
        None
    } else {
        Some(oid)
    }
}

fn raw_status_to_file_status(raw: &str, old_path: Option<&str>, prefix: &str) -> FileStatus {
    let code = raw.chars().next().unwrap_or('M');
    match code {
        'R' => {
            let from = old_path.unwrap_or_default();
            FileStatus::Renamed {
                from: relativize(prefix, from).unwrap_or_else(|| from.to_owned()),
            }
        }
        'A' | 'C' => FileStatus::Added,
        'D' => FileStatus::Deleted,
        'U' => FileStatus::Unmerged,
        _ => FileStatus::Modified,
    }
}

fn mode_change_note(old_mode: &str, new_mode: &str) -> Option<ModeChange> {
    if old_mode == "000000" || new_mode == "000000" || old_mode == new_mode {
        None
    } else {
        Some(ModeChange {
            old: old_mode.to_owned(),
            new: new_mode.to_owned(),
        })
    }
}

fn git_blob_size(git: &Git, top: &Path, oid: &str) -> Result<u64> {
    let raw = run_git(git, top, &["cat-file", "-s", oid])?;
    trimmed_utf8(raw)
        .parse::<u64>()
        .map_err(|error| Error::msg(format!("parse blob size for {oid}: {error}")))
}

fn git_cat_blob(git: &Git, top: &Path, oid: &str) -> Result<Vec<u8>> {
    run_git(git, top, &["cat-file", "blob", oid])
}

fn working_path_size(abs_path: &Path) -> Result<u64> {
    let meta = std::fs::symlink_metadata(abs_path)
        .map_err(|error| Error::msg(format!("{}: {error}", abs_path.display())))?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(abs_path)
            .map_err(|error| Error::msg(format!("{}: {error}", abs_path.display())))?;
        Ok(target.as_os_str().as_encoded_bytes().len() as u64)
    } else {
        Ok(meta.len())
    }
}

fn read_working_bytes(abs_path: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::symlink_metadata(abs_path)
        .map_err(|error| Error::msg(format!("{}: {error}", abs_path.display())))?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(abs_path)
            .map_err(|error| Error::msg(format!("{}: {error}", abs_path.display())))?;
        return Ok(target.as_os_str().as_encoded_bytes().to_vec());
    }
    let file = std::fs::File::open(abs_path)
        .map_err(|error| Error::msg(format!("{}: {error}", abs_path.display())))?;
    let mut bytes = Vec::with_capacity((meta.len().min(MAX_READ_BYTES)) as usize);
    let read = file
        .take(MAX_READ_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::msg(format!("{}: {error}", abs_path.display())))?;
    if read as u64 > MAX_READ_BYTES {
        return Err(Error::msg(format!(
            "{} grew past 10 MB while reading",
            abs_path.display()
        )));
    }
    Ok(bytes)
}

fn is_binary_bytes(bytes: &[u8]) -> bool {
    let sniff = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
    sniff.contains(&0)
}

fn hunks_for_added_text(bytes: &[u8]) -> Vec<DiffHunk> {
    let lines = count_lines(bytes);
    if lines == 0 {
        return Vec::new();
    }
    let count = u32::try_from(lines).unwrap_or(u32::MAX);
    let header = if count == 1 {
        "@@ -0,0 +1 @@".to_owned()
    } else {
        format!("@@ -0,0 +1,{count} @@")
    };
    vec![DiffHunk {
        header,
        old_start: 0,
        old_lines: 0,
        new_start: 1,
        new_lines: count,
        ops: vec![DiffOp::Added { count }],
    }]
}

pub(crate) fn count_lines(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    let newlines = bytes.iter().filter(|&&b| b == b'\n').count() as u64;
    if bytes.last() == Some(&b'\n') {
        newlines
    } else {
        newlines + 1
    }
}

struct WorkingRawInfo {
    mode: &'static str,
    symlink_blob_id: Option<String>,
    symlink_target_bytes: Option<Vec<u8>>,
    binary: bool,
    added_lines: u64,
}

fn inspect_working_meta(abs_path: &Path) -> Option<(&'static str, Option<String>)> {
    let meta = std::fs::symlink_metadata(abs_path).ok()?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(abs_path).ok()?;
        let bytes = target.as_os_str().as_encoded_bytes();
        return Some(("120000", Some(git_blob_hash(bytes))));
    }
    if meta.is_file() {
        return Some((file_mode_for_meta(&meta), None));
    }
    None
}

fn inspect_working_raw(abs_path: &Path) -> Option<WorkingRawInfo> {
    let meta = std::fs::symlink_metadata(abs_path).ok()?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(abs_path).ok()?;
        let bytes = target.as_os_str().as_encoded_bytes().to_vec();
        return Some(WorkingRawInfo {
            mode: "120000",
            symlink_blob_id: Some(git_blob_hash(&bytes)),
            binary: is_binary_bytes(&bytes),
            added_lines: count_lines(&bytes),
            symlink_target_bytes: Some(bytes),
        });
    }
    if !meta.is_file() {
        return None;
    }
    let mode = file_mode_for_meta(&meta);
    let mut file = std::fs::File::open(abs_path).ok()?;
    let mut total_bytes: u64 = 0;
    let mut sniff_left = BINARY_SNIFF_BYTES;
    let mut binary = false;
    let mut newlines: u64 = 0;
    let mut last_byte = None;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        total_bytes += n as u64;
        if sniff_left > 0 {
            let take = n.min(sniff_left);
            if chunk[..take].contains(&0) {
                binary = true;
            }
            sniff_left -= take;
        }
        if !binary {
            newlines += chunk.iter().filter(|&&b| b == b'\n').count() as u64;
        }
        last_byte = Some(chunk[n - 1]);
    }

    let added_lines = if binary || total_bytes == 0 {
        0
    } else if last_byte == Some(b'\n') {
        newlines
    } else {
        newlines + 1
    };

    Some(WorkingRawInfo {
        mode,
        symlink_blob_id: None,
        symlink_target_bytes: None,
        binary,
        added_lines,
    })
}

fn hash_working_files_with_git(
    git: &Git,
    top: &Path,
    paths: &[String],
    no_filters: bool,
) -> Result<HashMap<String, String>> {
    let mut hashes = HashMap::with_capacity(paths.len());
    if paths.is_empty() {
        return Ok(hashes);
    }

    let mut plain = Vec::new();
    let mut special = Vec::new();
    let mut seen = HashSet::with_capacity(paths.len());
    for path in paths {
        if !seen.insert(path.as_str()) {
            continue;
        }
        if path.contains(['\n', '\r', '"', '\\']) {
            special.push(path.as_str());
        } else {
            plain.push(path.as_str());
        }
    }

    if !plain.is_empty() {
        if let Some(batch) = hash_working_paths_batch(git, top, &plain, no_filters) {
            hashes.extend(batch);
        } else {
            for path in plain {
                if let Some(oid) = hash_single_working_path(git, top, path, no_filters)? {
                    hashes.insert(path.to_owned(), oid);
                }
            }
        }
    }

    for path in special {
        if let Some(oid) = hash_single_working_path(git, top, path, no_filters)? {
            hashes.insert(path.to_owned(), oid);
        }
    }

    Ok(hashes)
}

fn hash_working_paths_batch(
    git: &Git,
    top: &Path,
    paths: &[&str],
    no_filters: bool,
) -> Option<HashMap<String, String>> {
    let mut cmd = git_cmd(git, top);
    cmd.arg("hash-object");
    if no_filters {
        cmd.arg("--no-filters");
    }
    cmd.arg("--stdin-paths")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().ok()?;
    let mut stdin = child.stdin.take()?;
    let (write_err, output) = std::thread::scope(|s| {
        let writer = s.spawn(move || {
            for path in paths {
                stdin.write_all(path.as_bytes())?;
                stdin.write_all(b"\n")?;
            }
            drop(stdin);
            std::io::Result::Ok(())
        });
        let output = child.wait_with_output();
        let write_err = writer.join().ok().and_then(|res| res.err());
        (write_err, output)
    });
    if write_err.is_some() {
        return None;
    }
    let output = output.ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = std::str::from_utf8(&output.stdout).ok()?;
    let oids: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if oids.len() != paths.len()
        || !oids
            .iter()
            .all(|oid| oid.len() >= 40 && oid.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return None;
    }
    Some(
        paths
            .iter()
            .zip(oids)
            .map(|(&path, oid)| (path.to_owned(), oid.to_owned()))
            .collect(),
    )
}

fn hash_single_working_path(
    git: &Git,
    top: &Path,
    rel_path: &str,
    no_filters: bool,
) -> Result<Option<String>> {
    let abs_path = top.join(rel_path);
    let Ok(meta) = std::fs::symlink_metadata(&abs_path) else {
        return Ok(None);
    };
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(&abs_path)
            .map_err(|error| Error::msg(format!("{}: {error}", abs_path.display())))?;
        return Ok(Some(git_blob_hash(target.as_os_str().as_encoded_bytes())));
    }
    if !meta.is_file() {
        return Ok(None);
    }
    let file = match std::fs::File::open(&abs_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::msg(format!("{}: {error}", abs_path.display()))),
    };
    let mut cmd = git_cmd(git, top);
    cmd.arg("hash-object");
    let path_arg;
    if no_filters {
        cmd.args(["--no-filters", "--stdin"]);
    } else {
        path_arg = format!("--path={rel_path}");
        cmd.args(["--stdin", &path_arg]);
    }
    let output = cmd
        .stdin(Stdio::from(file))
        .output()
        .map_err(|error| Error::msg(format!("run git hash-object: {error}")))?;
    if !output.status.success() {
        return Err(Error::msg(format!(
            "git hash-object: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let oid = trimmed_utf8(output.stdout);
    Ok((!oid.is_empty()).then_some(oid))
}

fn run_git_neutral_no_index(
    git: &Git,
    neutral_dir: &Path,
    extra_args: &[&str],
    old_path: &Path,
    new_path: &Path,
) -> Result<Vec<u8>> {
    let attrs_path = neutral_dir.join(".empty-attributes");
    std::fs::write(&attrs_path, b"")
        .map_err(|error| Error::msg(format!("write neutral attributes: {error}")))?;
    let attrs_cfg = format!("core.attributesfile={}", attrs_path.display());
    let old_s = old_path.to_string_lossy();
    let new_s = new_path.to_string_lossy();
    let mut args = vec![
        "-c",
        "core.autocrlf=false",
        "-c",
        "core.safecrlf=false",
        "-c",
        attrs_cfg.as_str(),
        "diff",
        "--no-index",
        "--ignore-cr-at-eol",
    ];
    args.extend_from_slice(extra_args);
    args.extend_from_slice(&[
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--no-relative",
        "--",
        old_s.as_ref(),
        new_s.as_ref(),
    ]);
    run_git_diff_allow_changes(git, neutral_dir, &args)
}

fn no_index_numstat(
    git: &Git,
    top: &Path,
    old_blob_id: Option<&str>,
    abs_new: &Path,
    symlink_new_bytes: Option<&[u8]>,
) -> Result<DiffCounts> {
    let neutral_dir = tempfile::tempdir()
        .map_err(|error| Error::msg(format!("create neutral diff dir: {error}")))?;
    let mut old_tmp = tempfile::NamedTempFile::new_in(neutral_dir.path())
        .map_err(|error| Error::msg(format!("create temp file: {error}")))?;
    if let Some(oid) = old_blob_id {
        write_blob_to_temp(git, top, oid, &mut old_tmp)?;
    }
    let old_path = old_tmp.into_temp_path();

    let new_temp_holder = if let Some(bytes) = symlink_new_bytes {
        let mut tmp = tempfile::NamedTempFile::new_in(neutral_dir.path())
            .map_err(|error| Error::msg(format!("create temp file: {error}")))?;
        tmp.write_all(bytes)
            .and_then(|()| tmp.as_file_mut().flush())
            .map_err(|error| Error::msg(format!("write temp file: {error}")))?;
        Some(tmp.into_temp_path())
    } else {
        None
    };
    let new_path: &Path = new_temp_holder.as_deref().unwrap_or(abs_new);

    let out = run_git_neutral_no_index(
        git,
        neutral_dir.path(),
        &["--numstat", "-z"],
        &old_path,
        new_path,
    )?;
    let parsed = parse_numstat_z(&out)?;
    Ok(parsed
        .into_iter()
        .next()
        .map(|entry| entry.counts)
        .unwrap_or(DiffCounts::Lines {
            added: 0,
            removed: 0,
        }))
}

fn diff_hunks_no_index(git: &Git, old_bytes: &[u8], new_bytes: &[u8]) -> Result<Vec<DiffHunk>> {
    let dir = tempfile::tempdir()
        .map_err(|error| Error::msg(format!("create diff temp dir: {error}")))?;
    let old_path = dir.path().join("old");
    let new_path = dir.path().join("new");
    std::fs::write(&old_path, old_bytes)
        .map_err(|error| Error::msg(format!("write diff old temp: {error}")))?;
    std::fs::write(&new_path, new_bytes)
        .map_err(|error| Error::msg(format!("write diff new temp: {error}")))?;
    let raw = run_git_neutral_no_index(git, dir.path(), &["-U3"], &old_path, &new_path)?;
    parse_hunks(&raw)
}

fn file_mode_for_meta(meta: &std::fs::Metadata) -> &'static str {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 != 0 {
            return "100755";
        }
    }
    #[cfg(not(unix))]
    let _ = meta;
    "100644"
}

/// Computes the lowercase hex git blob object id (`sha1("blob <len>\0" + data)`).
pub(crate) fn git_blob_hash(data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    let header = format!("blob {}\0", data.len());
    hasher.update(header.as_bytes());
    hasher.update(data);
    hasher.hex()
}

/// Parses `git log -z --format=%H%x1f%h%x1f%s%x1f%an%x1f%aI%x1f%P` output.
pub(crate) fn parse_commits_z(bytes: &[u8]) -> Result<Vec<CommitSummary>> {
    let mut commits = Vec::new();
    for record in bytes.split(|&b| b == 0).filter(|rec| !rec.is_empty()) {
        let text = String::from_utf8_lossy(record);
        let parts: Vec<&str> = text.split('\x1f').collect();
        let [id, short_id, subject, author, date_str, parents] = parts.as_slice() else {
            return Err(Error::msg("malformed git log record"));
        };
        let date: DateTime<Utc> = DateTime::parse_from_rfc3339(date_str)
            .map_err(|error| Error::msg(format!("parse commit date {date_str:?}: {error}")))?
            .with_timezone(&Utc);
        commits.push(CommitSummary {
            id: (*id).to_owned(),
            short_id: (*short_id).to_owned(),
            subject: (*subject).to_owned(),
            author: (*author).to_owned(),
            date,
            parent_count: parents.split_whitespace().count(),
        });
    }
    Ok(commits)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NumstatEntry {
    pub path: String,
    pub old_path: Option<String>,
    pub counts: DiffCounts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawDiffEntry {
    old_mode: String,
    new_mode: String,
    old_blob: String,
    new_blob: String,
    status: String,
    path: String,
    old_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawNumstatEntry {
    pub old_mode: String,
    pub new_mode: String,
    pub old_blob: String,
    pub new_blob: String,
    pub status: String,
    pub path: String,
    pub old_path: Option<String>,
    pub counts: DiffCounts,
}

/// Parses `git diff --numstat -z` records (including `-z` rename/copy pairs, binary `- -`, and
/// paths containing spaces or newlines).
pub(crate) fn parse_numstat_z(bytes: &[u8]) -> Result<Vec<NumstatEntry>> {
    let tokens: Vec<&[u8]> = bytes.split(|&b| b == 0).collect();
    let mut cursor = 0usize;
    parse_numstat_tokens(&tokens, &mut cursor)
}

fn parse_numstat_tokens(tokens: &[&[u8]], cursor: &mut usize) -> Result<Vec<NumstatEntry>> {
    let mut out = Vec::new();
    while *cursor < tokens.len() {
        let token = tokens[*cursor];
        *cursor += 1;
        if token.is_empty() {
            continue;
        }
        let line = String::from_utf8_lossy(token);
        let mut cols = line.splitn(3, '\t');
        let (Some(added_s), Some(removed_s), Some(rest)) = (cols.next(), cols.next(), cols.next())
        else {
            return Err(Error::msg(format!("malformed numstat entry: {line:?}")));
        };
        let counts = if added_s == "-" && removed_s == "-" {
            DiffCounts::Binary
        } else {
            let added = added_s
                .parse::<u64>()
                .map_err(|error| Error::msg(format!("parse added count {added_s:?}: {error}")))?;
            let removed = removed_s.parse::<u64>().map_err(|error| {
                Error::msg(format!("parse removed count {removed_s:?}: {error}"))
            })?;
            DiffCounts::Lines { added, removed }
        };

        let (old_path, path) = if rest.is_empty() {
            let old = next_non_empty_utf8(tokens, cursor, "numstat rename old path")?;
            let new = next_non_empty_utf8(tokens, cursor, "numstat rename new path")?;
            (Some(old), new)
        } else {
            (None, rest.to_owned())
        };
        out.push(NumstatEntry {
            path,
            old_path,
            counts,
        });
    }
    Ok(out)
}

fn next_non_empty_utf8(tokens: &[&[u8]], cursor: &mut usize, what: &str) -> Result<String> {
    let token = tokens
        .get(*cursor)
        .ok_or_else(|| Error::msg(format!("unexpected end of git output for {what}")))?;
    *cursor += 1;
    Ok(String::from_utf8_lossy(token).into_owned())
}

/// Parses combined `git diff --raw --numstat -z` output into unified records.
pub(crate) fn parse_raw_and_numstat_z(bytes: &[u8]) -> Result<Vec<RawNumstatEntry>> {
    let tokens: Vec<&[u8]> = bytes.split(|&b| b == 0).collect();
    let mut cursor = 0usize;
    let mut raw_entries = Vec::new();

    while cursor < tokens.len() {
        let token = tokens[cursor];
        if token.is_empty() {
            cursor += 1;
            continue;
        }
        if !token.starts_with(b":") {
            break;
        }
        cursor += 1;
        let header = String::from_utf8_lossy(&token[1..]);
        let parts: Vec<&str> = header.split(' ').collect();
        let [old_mode, new_mode, old_blob, new_blob, status] = parts.as_slice() else {
            return Err(Error::msg(format!("malformed raw diff header: {header:?}")));
        };
        let path1 = next_non_empty_utf8(&tokens, &mut cursor, "raw diff path")?;
        let (old_path, path) = if status.starts_with('R') || status.starts_with('C') {
            let path2 = next_non_empty_utf8(&tokens, &mut cursor, "raw diff target path")?;
            (Some(path1), path2)
        } else {
            (None, path1)
        };
        raw_entries.push(RawDiffEntry {
            old_mode: (*old_mode).to_owned(),
            new_mode: (*new_mode).to_owned(),
            old_blob: (*old_blob).to_owned(),
            new_blob: (*new_blob).to_owned(),
            status: (*status).to_owned(),
            path,
            old_path,
        });
    }

    let numstat_entries = parse_numstat_tokens(&tokens, &mut cursor)?;
    let mut by_key: HashMap<(Option<String>, String), DiffCounts> =
        HashMap::with_capacity(numstat_entries.len());
    for entry in &numstat_entries {
        by_key.insert((entry.old_path.clone(), entry.path.clone()), entry.counts);
    }

    let mut out = Vec::with_capacity(raw_entries.len());
    for (idx, raw) in raw_entries.into_iter().enumerate() {
        let counts = if let Some(ns) = numstat_entries.get(idx).filter(|ns| ns.path == raw.path) {
            ns.counts
        } else if let Some(&counts) = by_key.get(&(raw.old_path.clone(), raw.path.clone())) {
            counts
        } else {
            DiffCounts::Lines {
                added: 0,
                removed: 0,
            }
        };
        out.push(RawNumstatEntry {
            old_mode: raw.old_mode,
            new_mode: raw.new_mode,
            old_blob: raw.old_blob,
            new_blob: raw.new_blob,
            status: raw.status,
            path: raw.path,
            old_path: raw.old_path,
            counts,
        });
    }
    Ok(out)
}

/// Parses `git diff -U3` output into structured hunks, handling omitted `,1` counts
/// and `\ No newline at end of file` markers cleanly.
pub(crate) fn parse_hunks(bytes: &[u8]) -> Result<Vec<DiffHunk>> {
    let trimmed = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let mut hunks = Vec::new();
    let mut current: Option<(DiffHunk, u32, u32)> = None;

    for raw_line in trimmed.split(|&b| b == b'\n') {
        if raw_line.starts_with(b"@@ ") {
            if let Some((hunk, _, _)) = current.take() {
                hunks.push(hunk);
            }
            let header = String::from_utf8_lossy(raw_line)
                .trim_end_matches('\r')
                .to_owned();
            let (old_start, old_lines, new_start, new_lines) = parse_hunk_header(&header)?;
            current = Some((
                DiffHunk {
                    header,
                    old_start,
                    old_lines,
                    new_start,
                    new_lines,
                    ops: Vec::new(),
                },
                old_lines,
                new_lines,
            ));
            continue;
        }

        let Some((hunk, rem_old, rem_new)) = current.as_mut() else {
            continue;
        };

        if raw_line.starts_with(b"\\") {
            // `\ No newline at end of file` annotates the preceding line without adding ops.
            continue;
        }

        if *rem_old == 0 && *rem_new == 0 {
            let finished = current.take().expect("current is Some").0;
            hunks.push(finished);
            continue;
        }

        match raw_line.first().copied() {
            Some(b' ') => {
                *rem_old = rem_old.saturating_sub(1);
                *rem_new = rem_new.saturating_sub(1);
                push_diff_op(&mut hunk.ops, DiffOp::Context { count: 1 });
            }
            Some(b'-') => {
                *rem_old = rem_old.saturating_sub(1);
                push_diff_op(&mut hunk.ops, DiffOp::Removed { count: 1 });
            }
            Some(b'+') => {
                *rem_new = rem_new.saturating_sub(1);
                push_diff_op(&mut hunk.ops, DiffOp::Added { count: 1 });
            }
            _ => {
                let finished = current.take().expect("current is Some").0;
                hunks.push(finished);
            }
        }
    }

    if let Some((hunk, _, _)) = current.take() {
        hunks.push(hunk);
    }
    Ok(hunks)
}

fn push_diff_op(ops: &mut Vec<DiffOp>, next: DiffOp) {
    match (ops.last_mut(), next) {
        (Some(DiffOp::Context { count }), DiffOp::Context { count: add })
        | (Some(DiffOp::Removed { count }), DiffOp::Removed { count: add })
        | (Some(DiffOp::Added { count }), DiffOp::Added { count: add }) => {
            *count = count.saturating_add(add);
        }
        _ => ops.push(next),
    }
}

fn parse_hunk_header(header: &str) -> Result<(u32, u32, u32, u32)> {
    let rest = header
        .strip_prefix("@@ ")
        .and_then(|s| s.split(" @@").next())
        .ok_or_else(|| Error::msg(format!("malformed hunk header: {header:?}")))?;
    let mut parts = rest.split_whitespace();
    let (Some(old_spec), Some(new_spec)) = (parts.next(), parts.next()) else {
        return Err(Error::msg(format!("malformed hunk range: {header:?}")));
    };
    let (old_start, old_lines) = parse_hunk_range(old_spec, '-')?;
    let (new_start, new_lines) = parse_hunk_range(new_spec, '+')?;
    Ok((old_start, old_lines, new_start, new_lines))
}

fn parse_hunk_range(spec: &str, sign: char) -> Result<(u32, u32)> {
    let body = spec
        .strip_prefix(sign)
        .ok_or_else(|| Error::msg(format!("malformed hunk range token: {spec:?}")))?;
    if let Some((start, count)) = body.split_once(',') {
        let start = start
            .parse::<u32>()
            .map_err(|error| Error::msg(format!("parse hunk start {start:?}: {error}")))?;
        let count = count
            .parse::<u32>()
            .map_err(|error| Error::msg(format!("parse hunk count {count:?}: {error}")))?;
        Ok((start, count))
    } else {
        let start = body
            .parse::<u32>()
            .map_err(|error| Error::msg(format!("parse hunk start {body:?}: {error}")))?;
        Ok((start, 1))
    }
}

/// Minimal FIPS 180-4 SHA-1 hasher for git blob object hashes (`sha1("blob <len>\0" + bytes)`).
struct Sha1 {
    state: [u32; 5],
    buf: [u8; 64],
    buf_len: usize,
    total_bytes: u64,
}

impl Sha1 {
    fn new() -> Self {
        Self {
            state: [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0],
            buf: [0u8; 64],
            buf_len: 0,
            total_bytes: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.total_bytes = self.total_bytes.saturating_add(data.len() as u64);
        if self.buf_len > 0 {
            let take = data.len().min(64 - self.buf_len);
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                self.compress(&block);
                self.buf_len = 0;
            }
        }
        while data.len() >= 64 {
            let block: [u8; 64] = data[..64].try_into().expect("64-byte slice");
            self.compress(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    fn finalize(mut self) -> [u8; 20] {
        let total_bits = self.total_bytes.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buf_len != 56 {
            if self.buf_len > 56 {
                self.buf[self.buf_len..64].fill(0);
                let block = self.buf;
                self.compress(&block);
                self.buf_len = 0;
            } else {
                self.buf[self.buf_len..56].fill(0);
                self.buf_len = 56;
            }
        }
        self.buf[56..64].copy_from_slice(&total_bits.to_be_bytes());
        let block = self.buf;
        self.compress(&block);

        let mut out = [0u8; 20];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn hex(self) -> String {
        let digest = self.finalize();
        let mut s = String::with_capacity(40);
        for b in digest {
            use std::fmt::Write;
            let _ = write!(&mut s, "{b:02x}");
        }
        s
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 80];
        for (i, chunk) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(chunk.try_into().expect("4-byte word"));
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = self.state;
        for (i, &word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..20 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..40 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..60 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
    }
}

#[cfg(test)]
#[path = "changes_tests.rs"]
mod tests;
