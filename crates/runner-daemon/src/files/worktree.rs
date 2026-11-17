use super::{FilesService, Git};
use crate::error::{Error, Result};
use crate::repo;
use crate::AppCore;
use runner_core::app_paths::{
    is_existing_dir, normalize_checkout_path, normalized_checkout_starts_with, same_checkout_path,
    same_normalized_checkout_path,
};
use runner_core::protocol::files::{WorktreeEntry, WorktreeKind};
use runner_core::protocol::worktree::{
    MissingWorktreeBranchState, WorktreeCheckout, WorktreeCreated, WorktreeDefaultBase,
    WorktreeItem, WorktreeList, WorktreeNameCheck, WorktreeRemoved,
};
use rusqlite::Connection;
use std::path::{Component, Path, PathBuf};

const WORKTREES_DIR: &str = ".worktrees";
const EXCLUDE_PATTERN: &str = "/.worktrees/";

pub(crate) fn flatten_branch_name(branch: &str) -> String {
    branch.replace('/', "-")
}

pub(crate) fn managed_worktrees_root(main_checkout: &Path) -> PathBuf {
    normalize_checkout_path(main_checkout).join(WORKTREES_DIR)
}

pub(crate) fn is_managed_worktree(main_checkout: &Path, entry_path: &Path) -> bool {
    let norm_entry = normalize_checkout_path(entry_path);
    let Some(parent) = norm_entry.parent() else {
        return false;
    };
    same_normalized_checkout_path(parent, &managed_worktrees_root(main_checkout))
}

fn require_git(files: &FilesService) -> Result<Git> {
    files
        .git()
        .ok_or_else(|| Error::msg("git executable was not found"))
}

fn git_error_message(output: &std::process::Output, fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !stdout.is_empty() {
        return stdout;
    }
    fallback.to_owned()
}

fn run_git_status(git: &Git, dir: &Path, args: &[&str]) -> Result<std::process::Output> {
    git.command(dir).args(args).output().map_err(|error| {
        let sub = args.first().copied().unwrap_or("");
        Error::msg(format!("failed to run git {sub}: {error}"))
    })
}

fn run_git_checked(git: &Git, dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = run_git_status(git, dir, args)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(Error::msg(git_error_message(&output, "git command failed")))
    }
}

fn validate_branch_name(git: &Git, dir: &Path, branch: &str) -> Result<()> {
    if branch.is_empty() {
        return Err(Error::msg("branch name cannot be empty"));
    }
    let output = run_git_status(git, dir, &["check-ref-format", "--branch", branch])?;
    if !output.status.success() {
        return Err(Error::msg(git_error_message(
            &output,
            "invalid branch name",
        )));
    }
    let normalized = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if normalized != branch || normalized == "HEAD" {
        return Err(Error::msg(format!("'{branch}' is not a valid branch name")));
    }
    Ok(())
}

struct RepoContext {
    main_checkout: PathBuf,
    common_dir: PathBuf,
    entries: Vec<WorktreeEntry>,
}

fn repo_context(git: &Git, dir: &Path) -> Result<RepoContext> {
    if !is_existing_dir(dir) {
        return Err(Error::msg(format!(
            "directory does not exist: {}",
            dir.display()
        )));
    }
    let Some(repo) = git.rev_parse(dir) else {
        return Err(Error::msg(format!(
            "{} is not inside a git repository",
            dir.display()
        )));
    };
    let entries = git.worktrees(&repo.top)?;
    let main = entries
        .iter()
        .find(|entry| entry.kind == WorktreeKind::Main)
        .ok_or_else(|| Error::msg("bare git repositories are not supported for worktrees"))?;
    Ok(RepoContext {
        main_checkout: normalize_checkout_path(&main.path),
        common_dir: normalize_checkout_path(&repo.common_dir),
        entries,
    })
}

/// If `dir` is inside a linked worktree, return the repository's main checkout path.
pub(crate) fn linked_main_checkout(git: &Git, dir: &Path) -> Option<PathBuf> {
    if !is_existing_dir(dir) {
        return None;
    }
    let repo = git.rev_parse(dir)?;
    let entries = git.worktrees(&repo.top).ok()?;
    let main = entries
        .iter()
        .find(|entry| entry.kind == WorktreeKind::Main)?;
    let main_path = normalize_checkout_path(&main.path);
    if same_normalized_checkout_path(&main_path, &repo.top) {
        return None;
    }
    Some(main_path)
}

fn commit_exists(git: &Git, dir: &Path, rev: &str) -> bool {
    if rev.starts_with('-') {
        return false;
    }
    let spec = format!("{rev}^{{commit}}");
    run_git_status(
        git,
        dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &spec,
        ],
    )
    .is_ok_and(|out| out.status.success())
}

fn short_commit(git: &Git, dir: &Path, rev: &str) -> Result<Option<String>> {
    if rev.starts_with('-') {
        return Err(Error::msg(format!("invalid ref: {rev}")));
    }
    let spec = format!("{rev}^{{commit}}");
    let output = run_git_status(
        git,
        dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--short",
            "--end-of-options",
            &spec,
        ],
    )?;
    if output.status.success() {
        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ))
    } else {
        Ok(None)
    }
}

fn local_branch_commit(git: &Git, main_checkout: &Path, branch: &str) -> Result<Option<String>> {
    let full_ref = format!("refs/heads/{branch}");
    short_commit(git, main_checkout, &full_ref)
}

fn resolve_default_base_in_main(git: &Git, main_checkout: &Path) -> Result<Option<String>> {
    if commit_exists(git, main_checkout, "refs/remotes/origin/HEAD") {
        let sym = run_git_status(
            git,
            main_checkout,
            &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
        )?;
        if sym.status.success() {
            let full = String::from_utf8_lossy(&sym.stdout).trim().to_owned();
            if let Some(short) = full.strip_prefix("refs/remotes/") {
                return Ok(Some(short.to_owned()));
            }
        }
        return Ok(Some("origin/HEAD".to_owned()));
    }

    if !commit_exists(git, main_checkout, "HEAD") {
        return Ok(None);
    }

    let head_branch = run_git_status(
        git,
        main_checkout,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )?;
    if head_branch.status.success() {
        let name = String::from_utf8_lossy(&head_branch.stdout)
            .trim()
            .to_owned();
        if !name.is_empty() {
            return Ok(Some(name));
        }
    }

    if let Some(sha) = short_commit(git, main_checkout, "HEAD")? {
        return Ok(Some(sha));
    }
    Ok(Some("HEAD".to_owned()))
}

fn verify_explicit_base(git: &Git, main_checkout: &Path, base: &str) -> Result<String> {
    let trimmed = base.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') {
        return Err(Error::msg(format!("invalid base ref: '{base}'")));
    }
    let spec = format!("{trimmed}^{{commit}}");
    run_git_checked(
        git,
        main_checkout,
        &["rev-parse", "--verify", "--end-of-options", &spec],
    )?;
    Ok(trimmed.to_owned())
}

fn ensure_worktrees_excluded(git: &Git, main_checkout: &Path, common_dir: &Path) -> Result<()> {
    let ignored = run_git_status(
        git,
        main_checkout,
        &["check-ignore", "-q", "--", ".worktrees/"],
    )?;
    if ignored.status.success() {
        return Ok(());
    }
    if ignored.status.code() != Some(1) {
        return Err(Error::msg(git_error_message(
            &ignored,
            "failed to check git ignore rules for .worktrees/",
        )));
    }

    let exclude_path = common_dir.join("info").join("exclude");
    if let Some(parent) = exclude_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = match std::fs::read_to_string(&exclude_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    if existing.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == EXCLUDE_PATTERN || trimmed == ".worktrees/" || trimmed == ".worktrees"
    }) {
        return Ok(());
    }

    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(EXCLUDE_PATTERN);
    updated.push('\n');
    std::fs::write(&exclude_path, updated)?;
    Ok(())
}

fn remove_prunable_registration(git: &Git, main_checkout: &Path, path: &Path) -> Result<()> {
    let path_str = path.to_string_lossy();
    run_git_checked(git, main_checkout, &["worktree", "remove", "--", &path_str])?;
    Ok(())
}

fn clear_prunable_conflicts(
    git: &Git,
    main_checkout: &Path,
    entries: &[WorktreeEntry],
    target_path: &Path,
    branch: &str,
) -> Result<()> {
    for entry in entries {
        if entry.prunable
            && (same_normalized_checkout_path(&entry.path, target_path)
                || entry.branch.as_deref() == Some(branch))
        {
            remove_prunable_registration(git, main_checkout, &entry.path)?;
        }
    }
    Ok(())
}

fn find_live_branch_worktree<'a>(
    entries: &'a [WorktreeEntry],
    branch: &str,
) -> Option<&'a WorktreeEntry> {
    entries.iter().find(|entry| {
        !entry.prunable && entry.branch.as_deref() == Some(branch) && is_existing_dir(&entry.path)
    })
}

fn find_unavailable_branch_worktree<'a>(
    entries: &'a [WorktreeEntry],
    branch: &str,
) -> Option<&'a WorktreeEntry> {
    entries.iter().find(|entry| {
        !entry.prunable && entry.branch.as_deref() == Some(branch) && !is_existing_dir(&entry.path)
    })
}

fn unavailable_registration_reason(entry: &WorktreeEntry) -> String {
    let branch_label = entry.branch.as_deref().unwrap_or("detached HEAD");
    if entry.locked {
        format!(
            "branch '{branch_label}' is registered to a locked missing worktree at {}; unlock or prune it first",
            entry.path.display()
        )
    } else {
        format!(
            "branch '{branch_label}' is registered to a missing worktree at {}; prune it first",
            entry.path.display()
        )
    }
}

fn path_has_non_prunable_collision(
    entries: &[WorktreeEntry],
    target_path: &Path,
) -> Option<String> {
    if let Some(existing) = entries
        .iter()
        .find(|entry| !entry.prunable && same_normalized_checkout_path(&entry.path, target_path))
    {
        if !is_existing_dir(&existing.path) {
            return Some(unavailable_registration_reason(existing));
        }
        let label = existing.branch.as_deref().unwrap_or("detached HEAD");
        return Some(format!(
            "{} is already checked out for '{label}'",
            target_path.display()
        ));
    }
    let has_prunable = entries
        .iter()
        .any(|entry| entry.prunable && same_normalized_checkout_path(&entry.path, target_path));
    if !has_prunable && target_path.exists() {
        return Some(format!("{} already exists", target_path.display()));
    }
    None
}

fn list_local_branches(git: &Git, main_checkout: &Path) -> Result<Vec<String>> {
    let raw = run_git_checked(
        git,
        main_checkout,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )?;
    Ok(String::from_utf8_lossy(&raw)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

fn head_branch_and_commit(git: &Git, checkout: &Path) -> (Option<String>, Option<String>) {
    let branch = run_git_status(
        git,
        checkout,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )
    .ok()
    .filter(|out| out.status.success())
    .and_then(|out| {
        let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        (!name.is_empty()).then_some(name)
    });
    let commit = short_commit(git, checkout, "HEAD").ok().flatten();
    (branch, commit)
}

#[derive(Debug, Clone)]
struct MissingWorktreeInfo {
    worktree_root: PathBuf,
    main_checkout: PathBuf,
    common_dir: PathBuf,
    managed: bool,
    resolution: MissingBranchResolution,
}

#[derive(Debug, Clone)]
enum MissingBranchResolution {
    Surviving(String),
    Ambiguous(Vec<String>),
    Gone,
}

fn find_worktrees_under_main_prefixes(cwd: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut matches = Vec::new();
    let mut prefix = PathBuf::new();
    let mut saw_worktrees_parent: Option<PathBuf> = None;
    for component in cwd.components() {
        prefix.push(component);
        if let Some(main_candidate) = saw_worktrees_parent.take() {
            if matches!(component, Component::Normal(_)) {
                matches.push((main_candidate, normalize_checkout_path(&prefix)));
            }
        }
        if matches!(component, Component::Normal(name) if name == WORKTREES_DIR) {
            if let Some(parent) = prefix.parent() {
                saw_worktrees_parent = Some(parent.to_path_buf());
            }
        }
    }
    matches
}

fn resolve_missing_branch(
    git: &Git,
    main_checkout: &Path,
    entries: &[WorktreeEntry],
    worktree_root: &Path,
) -> Result<MissingBranchResolution> {
    if let Some(registered) = entries.iter().find(|entry| {
        (entry.prunable || !is_existing_dir(&entry.path))
            && same_normalized_checkout_path(&entry.path, worktree_root)
    }) {
        return Ok(match registered.branch.as_deref() {
            Some(branch) if local_branch_commit(git, main_checkout, branch)?.is_some() => {
                MissingBranchResolution::Surviving(branch.to_owned())
            }
            _ => MissingBranchResolution::Gone,
        });
    }

    let Some(folder_name) = worktree_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    else {
        return Ok(MissingBranchResolution::Gone);
    };
    let matches: Vec<String> = list_local_branches(git, main_checkout)?
        .into_iter()
        .filter(|branch| flatten_branch_name(branch) == folder_name)
        .collect();
    match matches.len() {
        0 => Ok(MissingBranchResolution::Gone),
        1 => Ok(MissingBranchResolution::Surviving(
            matches.into_iter().next().unwrap(),
        )),
        _ => Ok(MissingBranchResolution::Ambiguous(matches)),
    }
}

fn detect_missing_external_by_branch(
    git: &Git,
    ctx: &RepoContext,
    norm_cwd: &Path,
) -> Result<Option<MissingWorktreeInfo>> {
    if normalized_checkout_starts_with(norm_cwd, &ctx.main_checkout)
        || normalized_checkout_starts_with(&ctx.main_checkout, norm_cwd)
    {
        return Ok(None);
    }

    // Deleted subfolders inside an existing checkout of the same repo are not external worktrees.
    if let Some(existing_ancestor) = norm_cwd
        .ancestors()
        .find(|ancestor| is_existing_dir(ancestor))
    {
        if git
            .rev_parse(existing_ancestor)
            .is_some_and(|repo| same_checkout_path(&repo.common_dir, &ctx.common_dir))
        {
            return Ok(None);
        }
    }

    let missing_ancestors: Vec<PathBuf> = norm_cwd
        .ancestors()
        .take_while(|ancestor| !is_existing_dir(ancestor))
        .filter(|ancestor| {
            ancestor.file_name().is_some()
                && !normalized_checkout_starts_with(ancestor, &ctx.main_checkout)
                && !normalized_checkout_starts_with(&ctx.main_checkout, ancestor)
        })
        .map(Path::to_path_buf)
        .collect();
    if missing_ancestors.is_empty() {
        return Ok(None);
    }

    let local_branches = list_local_branches(git, &ctx.main_checkout)?;
    let mut matched_roots: Vec<(PathBuf, Vec<String>)> = Vec::new();
    // Walk outermost -> innermost so any parent worktree root is ordered before nested paths.
    for ancestor in missing_ancestors.into_iter().rev() {
        let Some(folder_name) = ancestor
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        let branches: Vec<String> = local_branches
            .iter()
            .filter(|branch| flatten_branch_name(branch) == folder_name)
            .cloned()
            .collect();
        if !branches.is_empty() {
            matched_roots.push((ancestor, branches));
        }
    }

    match matched_roots.len() {
        // With no prunable registration and no matching branch, preserve Plain checkout state.
        0 => Ok(None),
        1 => {
            let (worktree_root, mut branches) = matched_roots.into_iter().next().unwrap();
            let resolution = if branches.len() == 1 {
                MissingBranchResolution::Surviving(branches.remove(0))
            } else {
                MissingBranchResolution::Ambiguous(branches)
            };
            Ok(Some(MissingWorktreeInfo {
                worktree_root,
                main_checkout: ctx.main_checkout.clone(),
                common_dir: ctx.common_dir.clone(),
                managed: false,
                resolution,
            }))
        }
        _ => {
            let worktree_root = matched_roots[0].0.clone();
            let mut candidates: Vec<String> = matched_roots
                .into_iter()
                .flat_map(|(_, branches)| branches)
                .collect();
            candidates.sort();
            candidates.dedup();
            Ok(Some(MissingWorktreeInfo {
                worktree_root,
                main_checkout: ctx.main_checkout.clone(),
                common_dir: ctx.common_dir.clone(),
                managed: false,
                resolution: MissingBranchResolution::Ambiguous(candidates),
            }))
        }
    }
}

fn detect_missing_worktree(
    git: &Git,
    cwd: &Path,
    project_cwd_hint: Option<&Path>,
) -> Result<Option<MissingWorktreeInfo>> {
    if is_existing_dir(cwd) {
        return Ok(None);
    }

    for (main_candidate, wt_root) in find_worktrees_under_main_prefixes(cwd).into_iter().rev() {
        if is_existing_dir(&wt_root) {
            continue;
        }
        if let Ok(ctx) = repo_context(git, &main_candidate) {
            if same_checkout_path(&ctx.main_checkout, &main_candidate) {
                let resolution =
                    resolve_missing_branch(git, &ctx.main_checkout, &ctx.entries, &wt_root)?;
                return Ok(Some(MissingWorktreeInfo {
                    managed: is_managed_worktree(&ctx.main_checkout, &wt_root),
                    worktree_root: wt_root,
                    main_checkout: ctx.main_checkout,
                    common_dir: ctx.common_dir,
                    resolution,
                }));
            }
        }
    }

    if let Some(hint) = project_cwd_hint {
        let norm_cwd = normalize_checkout_path(cwd);
        if let Ok(ctx) = repo_context(git, hint) {
            if let Some(entry) = ctx.entries.iter().find(|entry| {
                !is_existing_dir(&entry.path)
                    && normalized_checkout_starts_with(&norm_cwd, &entry.path)
            }) {
                let wt_root = normalize_checkout_path(&entry.path);
                let resolution =
                    resolve_missing_branch(git, &ctx.main_checkout, &ctx.entries, &wt_root)?;
                return Ok(Some(MissingWorktreeInfo {
                    managed: is_managed_worktree(&ctx.main_checkout, &wt_root),
                    worktree_root: wt_root,
                    main_checkout: ctx.main_checkout,
                    common_dir: ctx.common_dir,
                    resolution,
                }));
            }
            if let Some(info) = detect_missing_external_by_branch(git, &ctx, &norm_cwd)? {
                return Ok(Some(info));
            }
        }
    }

    Ok(None)
}

fn resolve_worktree_target<'a>(
    entries: &'a [WorktreeEntry],
    main_checkout: &Path,
    dir: &Path,
    target: &str,
) -> Option<&'a WorktreeEntry> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(by_branch) = entries
        .iter()
        .find(|entry| entry.branch.as_deref() == Some(trimmed))
    {
        return Some(by_branch);
    }
    let raw = Path::new(trimmed);
    if raw.is_absolute() {
        let candidate = normalize_checkout_path(raw);
        return entries
            .iter()
            .find(|entry| same_normalized_checkout_path(&entry.path, &candidate));
    }
    for base in [dir, main_checkout] {
        let candidate = normalize_checkout_path(&base.join(raw));
        if let Some(entry) = entries
            .iter()
            .find(|entry| same_normalized_checkout_path(&entry.path, &candidate))
        {
            return Some(entry);
        }
    }
    None
}

fn find_blocking_worktree_occupant(
    conn: &Connection,
    worktree_path: &Path,
) -> Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.title, s.cwd
         FROM sessions s
         WHERE s.cwd IS NOT NULL
           AND s.archived_at IS NULL
           AND (
             s.mission_id IS NULL
             OR EXISTS (SELECT 1 FROM missions m WHERE m.id = s.mission_id AND m.archived_at IS NULL)
           )",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let title: Option<String> = row.get(1)?;
        let cwd: String = row.get(2)?;
        if normalized_checkout_starts_with(&normalize_checkout_path(Path::new(&cwd)), worktree_path)
        {
            let label = title
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| id.clone());
            return Ok(Some(format!("active session '{label}' ({id})")));
        }
    }

    let mut m_stmt = conn.prepare(
        "SELECT id, title, cwd FROM missions WHERE archived_at IS NULL AND cwd IS NOT NULL",
    )?;
    let mut m_rows = m_stmt.query([])?;
    while let Some(row) = m_rows.next()? {
        let id: String = row.get(0)?;
        let title: String = row.get(1)?;
        let cwd: String = row.get(2)?;
        if normalized_checkout_starts_with(&normalize_checkout_path(Path::new(&cwd)), worktree_path)
        {
            return Ok(Some(format!("active mission '{title}' ({id})")));
        }
    }

    Ok(None)
}

#[derive(Default)]
pub(crate) struct WorktreeGate {
    lock: std::sync::RwLock<()>,
    #[cfg(test)]
    before_registration:
        std::sync::Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
    #[cfg(test)]
    before_git_remove:
        std::sync::Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
    #[cfg(test)]
    read_contended: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
    #[cfg(test)]
    write_contended: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl WorktreeGate {
    pub(crate) fn read(&self) -> std::sync::RwLockReadGuard<'_, ()> {
        #[cfg(test)]
        match self.lock.try_read() {
            Ok(guard) => return guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                if let Some(tx) = self.read_contended.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
            Err(std::sync::TryLockError::Poisoned(poison)) => return poison.into_inner(),
        }
        self.lock
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub(crate) fn write(&self) -> std::sync::RwLockWriteGuard<'_, ()> {
        #[cfg(test)]
        match self.lock.try_write() {
            Ok(guard) => return guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                if let Some(tx) = self.write_contended.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
            Err(std::sync::TryLockError::Poisoned(poison)) => return poison.into_inner(),
        }
        self.lock
            .write()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    #[cfg(test)]
    pub(crate) fn pause_next_registration(
        &self,
        ready: std::sync::mpsc::Sender<()>,
        proceed: std::sync::mpsc::Receiver<()>,
    ) {
        *self.before_registration.lock().unwrap() = Some((ready, proceed));
    }

    #[cfg(test)]
    pub(crate) fn pause_next_git_remove(
        &self,
        ready: std::sync::mpsc::Sender<()>,
        proceed: std::sync::mpsc::Receiver<()>,
    ) {
        *self.before_git_remove.lock().unwrap() = Some((ready, proceed));
    }

    #[cfg(test)]
    pub(crate) fn notify_write_contended(&self, tx: std::sync::mpsc::Sender<()>) {
        *self.write_contended.lock().unwrap() = Some(tx);
    }

    #[cfg(test)]
    pub(crate) fn notify_read_contended(&self, tx: std::sync::mpsc::Sender<()>) {
        *self.read_contended.lock().unwrap() = Some(tx);
    }

    #[cfg(test)]
    pub(crate) fn fire_before_registration(&self) {
        if let Some((ready, proceed)) = self.before_registration.lock().unwrap().take() {
            let _ = ready.send(());
            let _ = proceed.recv();
        }
    }

    #[cfg(test)]
    fn fire_before_git_remove(&self) {
        if let Some((ready, proceed)) = self.before_git_remove.lock().unwrap().take() {
            let _ = ready.send(());
            let _ = proceed.recv();
        }
    }
}

impl FilesService {
    pub(crate) fn worktree_gate(&self) -> &WorktreeGate {
        &self.worktree_gate
    }

    fn repo_lock(&self, main_checkout: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
        let key = normalize_checkout_path(main_checkout);
        let mut locks = self
            .worktree_locks
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        locks
            .entry(key)
            .or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(())))
            .clone()
    }

    pub fn worktree_default_base(&self, dir: &Path) -> Result<Option<WorktreeDefaultBase>> {
        let git = require_git(self)?;
        let ctx = repo_context(&git, dir)?;
        Ok(
            resolve_default_base_in_main(&git, &ctx.main_checkout)?.map(|branch| {
                WorktreeDefaultBase {
                    branch,
                    main_checkout: ctx.main_checkout,
                }
            }),
        )
    }

    pub fn worktree_check(
        &self,
        dir: &Path,
        branch: &str,
        base: Option<&str>,
    ) -> Result<WorktreeNameCheck> {
        let git = require_git(self)?;
        let ctx = repo_context(&git, dir)?;
        if let Err(error) = validate_branch_name(&git, &ctx.main_checkout, branch) {
            return Ok(WorktreeNameCheck::Invalid {
                reason: error.to_string(),
            });
        }
        if let Some(existing) = find_live_branch_worktree(&ctx.entries, branch) {
            return Ok(WorktreeNameCheck::AlreadyCheckedOut {
                path: normalize_checkout_path(&existing.path),
            });
        }
        if let Some(unavailable) = find_unavailable_branch_worktree(&ctx.entries, branch) {
            return Ok(WorktreeNameCheck::Invalid {
                reason: unavailable_registration_reason(unavailable),
            });
        }

        let target_path =
            managed_worktrees_root(&ctx.main_checkout).join(flatten_branch_name(branch));
        if let Some(reason) = path_has_non_prunable_collision(&ctx.entries, &target_path) {
            return Ok(WorktreeNameCheck::Invalid { reason });
        }

        if let Some(commit) = local_branch_commit(&git, &ctx.main_checkout, branch)? {
            return Ok(WorktreeNameCheck::ExistingBranch {
                commit,
                path: target_path,
            });
        }

        let resolved_base = match base {
            Some(raw) => match verify_explicit_base(&git, &ctx.main_checkout, raw) {
                Ok(b) => b,
                Err(error) => {
                    return Ok(WorktreeNameCheck::Invalid {
                        reason: error.to_string(),
                    });
                }
            },
            None => match resolve_default_base_in_main(&git, &ctx.main_checkout)? {
                Some(b) => b,
                None => {
                    return Ok(WorktreeNameCheck::Invalid {
                        reason: "repository has no commits yet".to_owned(),
                    });
                }
            },
        };
        Ok(WorktreeNameCheck::NewBranch {
            base: resolved_base,
            path: target_path,
        })
    }

    pub fn worktree_create(
        &self,
        dir: &Path,
        branch: &str,
        base: Option<&str>,
    ) -> Result<WorktreeCreated> {
        let git = require_git(self)?;
        let initial_ctx = repo_context(&git, dir)?;
        validate_branch_name(&git, &initial_ctx.main_checkout, branch)?;

        let lock = self.repo_lock(&initial_ctx.main_checkout);
        let _guard = lock.lock().unwrap_or_else(|poison| poison.into_inner());

        let ctx = repo_context(&git, &initial_ctx.main_checkout)?;
        let target_path =
            managed_worktrees_root(&ctx.main_checkout).join(flatten_branch_name(branch));
        clear_prunable_conflicts(&git, &ctx.main_checkout, &ctx.entries, &target_path, branch)?;

        let refreshed_entries = git.worktrees(&ctx.main_checkout)?;
        if let Some(existing) = find_live_branch_worktree(&refreshed_entries, branch) {
            return Ok(WorktreeCreated {
                path: normalize_checkout_path(&existing.path),
                branch: branch.to_owned(),
                main_checkout: ctx.main_checkout,
                created: false,
            });
        }
        if let Some(unavailable) = find_unavailable_branch_worktree(&refreshed_entries, branch) {
            return Err(Error::msg(unavailable_registration_reason(unavailable)));
        }
        if let Some(reason) = path_has_non_prunable_collision(&refreshed_entries, &target_path) {
            return Err(Error::msg(reason));
        }

        let target_str = target_path.to_string_lossy().into_owned();
        if local_branch_commit(&git, &ctx.main_checkout, branch)?.is_some() {
            run_git_checked(
                &git,
                &ctx.main_checkout,
                &["worktree", "add", "--", &target_str, branch],
            )?;
        } else {
            let base_ref = match base {
                Some(raw) => verify_explicit_base(&git, &ctx.main_checkout, raw)?,
                None => resolve_default_base_in_main(&git, &ctx.main_checkout)?.ok_or_else(|| {
                    Error::msg("repository has no commits yet; pass --base or create an initial commit first")
                })?,
            };
            run_git_checked(
                &git,
                &ctx.main_checkout,
                &[
                    "worktree",
                    "add",
                    "-b",
                    branch,
                    "--",
                    &target_str,
                    &base_ref,
                ],
            )?;
        }

        ensure_worktrees_excluded(&git, &ctx.main_checkout, &ctx.common_dir)?;
        Ok(WorktreeCreated {
            path: normalize_checkout_path(&target_path),
            branch: branch.to_owned(),
            main_checkout: ctx.main_checkout,
            created: true,
        })
    }

    pub fn worktree_list(&self, dir: &Path) -> Result<WorktreeList> {
        let git = require_git(self)?;
        let ctx = repo_context(&git, dir)?;
        let worktrees = ctx
            .entries
            .into_iter()
            .map(|entry| WorktreeItem {
                managed: entry.kind == WorktreeKind::Linked
                    && is_managed_worktree(&ctx.main_checkout, &entry.path),
                path: normalize_checkout_path(&entry.path),
                branch: entry.branch,
                commit: entry.commit,
                kind: entry.kind,
                locked: entry.locked,
                prunable: entry.prunable,
            })
            .collect();
        Ok(WorktreeList {
            main_checkout: ctx.main_checkout,
            worktrees,
        })
    }

    #[cfg(test)]
    pub(crate) fn worktree_remove_checked(
        &self,
        conn: &Connection,
        dir: &Path,
        target: &str,
    ) -> Result<WorktreeRemoved> {
        let _gate = self.worktree_gate.write();
        let git = require_git(self)?;
        let initial_ctx = repo_context(&git, dir)?;
        let lock = self.repo_lock(&initial_ctx.main_checkout);
        let _guard = lock.lock().unwrap_or_else(|poison| poison.into_inner());

        self.remove_checked_with_conn(&git, conn, &initial_ctx.main_checkout, dir, target)
    }

    pub(crate) fn worktree_remove_from_pool(
        &self,
        pool: &crate::db::DbPool,
        dir: &Path,
        target: &str,
    ) -> Result<WorktreeRemoved> {
        // Lock order: worktree_gate -> repo_lock -> db pool, matching start/unarchive paths.
        let _gate = self.worktree_gate.write();
        let git = require_git(self)?;
        let initial_ctx = repo_context(&git, dir)?;
        let lock = self.repo_lock(&initial_ctx.main_checkout);
        let _guard = lock.lock().unwrap_or_else(|poison| poison.into_inner());
        let conn = pool.get()?;

        self.remove_checked_with_conn(&git, &conn, &initial_ctx.main_checkout, dir, target)
    }

    fn remove_checked_with_conn(
        &self,
        git: &Git,
        conn: &Connection,
        main_checkout: &Path,
        dir: &Path,
        target: &str,
    ) -> Result<WorktreeRemoved> {
        let ctx = repo_context(git, main_checkout)?;
        let entry = resolve_worktree_target(&ctx.entries, &ctx.main_checkout, dir, target)
            .cloned()
            .ok_or_else(|| Error::msg(format!("worktree not found: {target}")))?;

        if entry.kind == WorktreeKind::Main {
            return Err(Error::msg("cannot remove the main checkout"));
        }
        if !is_managed_worktree(&ctx.main_checkout, &entry.path) {
            return Err(Error::msg(format!(
                "only linked worktrees under {} can be removed",
                managed_worktrees_root(&ctx.main_checkout).display()
            )));
        }
        let norm_path = normalize_checkout_path(&entry.path);
        if let Some(occupant) = find_blocking_worktree_occupant(conn, &norm_path)? {
            return Err(Error::msg(format!(
                "worktree {} is in use by {occupant}; archive or close it first",
                norm_path.display()
            )));
        }
        #[cfg(test)]
        self.worktree_gate.fire_before_git_remove();

        let path_str = norm_path.to_string_lossy().into_owned();
        run_git_checked(
            git,
            &ctx.main_checkout,
            &["worktree", "remove", "--", &path_str],
        )?;
        Ok(WorktreeRemoved {
            path: norm_path,
            branch: entry.branch,
        })
    }

    pub(crate) fn worktree_checkout_state(
        &self,
        cwd: &Path,
        project_cwd_hint: Option<&Path>,
    ) -> Result<WorktreeCheckout> {
        let Some(git) = self.git() else {
            return Ok(WorktreeCheckout::Plain {
                missing: !is_existing_dir(cwd),
            });
        };

        if is_existing_dir(cwd) {
            let Some(repo) = git.rev_parse(cwd) else {
                return Ok(WorktreeCheckout::Plain { missing: false });
            };
            let entries = git.worktrees(&repo.top)?;
            let Some(main) = entries.iter().find(|e| e.kind == WorktreeKind::Main) else {
                return Ok(WorktreeCheckout::Plain { missing: false });
            };
            let main_checkout = normalize_checkout_path(&main.path);
            let current = entries
                .iter()
                .find(|entry| same_normalized_checkout_path(&entry.path, &repo.top));
            let kind = current.map(|e| e.kind).unwrap_or_else(|| {
                if same_normalized_checkout_path(&main_checkout, &repo.top) {
                    WorktreeKind::Main
                } else {
                    WorktreeKind::Linked
                }
            });
            let (head_branch, head_commit) = head_branch_and_commit(&git, &repo.top);
            let branch = head_branch.or_else(|| current.and_then(|e| e.branch.clone()));
            let commit = head_commit.or_else(|| current.and_then(|e| e.commit.clone()));
            let managed =
                kind == WorktreeKind::Linked && is_managed_worktree(&main_checkout, &repo.top);
            return Ok(WorktreeCheckout::Git {
                top: normalize_checkout_path(&repo.top),
                main_checkout,
                kind,
                managed,
                branch,
                commit,
                missing: false,
                missing_branch: None,
                candidate_branches: Vec::new(),
            });
        }

        if let Some(info) = detect_missing_worktree(&git, cwd, project_cwd_hint)? {
            let (branch, missing_branch, candidate_branches) = match info.resolution {
                MissingBranchResolution::Surviving(branch) => (
                    Some(branch),
                    Some(MissingWorktreeBranchState::Surviving),
                    Vec::new(),
                ),
                MissingBranchResolution::Ambiguous(candidates) => (
                    None,
                    Some(MissingWorktreeBranchState::Ambiguous),
                    candidates,
                ),
                MissingBranchResolution::Gone => {
                    (None, Some(MissingWorktreeBranchState::Gone), Vec::new())
                }
            };
            return Ok(WorktreeCheckout::Git {
                top: info.worktree_root,
                main_checkout: info.main_checkout,
                kind: WorktreeKind::Linked,
                managed: info.managed,
                branch,
                commit: None,
                missing: true,
                missing_branch,
                candidate_branches,
            });
        }

        Ok(WorktreeCheckout::Plain { missing: true })
    }

    pub(crate) fn is_missing_worktree_path(
        &self,
        cwd: &Path,
        project_cwd_hint: Option<&Path>,
    ) -> bool {
        if is_existing_dir(cwd) {
            return false;
        }
        let Some(git) = self.git() else {
            return false;
        };
        detect_missing_worktree(&git, cwd, project_cwd_hint)
            .ok()
            .flatten()
            .is_some()
    }

    pub(crate) fn restore_if_missing(
        &self,
        cwd: &Path,
        project_cwd_hint: Option<&Path>,
    ) -> Result<Option<PathBuf>> {
        if is_existing_dir(cwd) {
            return Ok(None);
        }
        let Some(git) = self.git() else {
            return Ok(None);
        };
        let Some(initial) = detect_missing_worktree(&git, cwd, project_cwd_hint)? else {
            return Ok(None);
        };

        let lock = self.repo_lock(&initial.main_checkout);
        let _guard = lock.lock().unwrap_or_else(|poison| poison.into_inner());

        if is_existing_dir(cwd) {
            return Ok(None);
        }
        let Some(info) = detect_missing_worktree(&git, cwd, project_cwd_hint)? else {
            return Ok(None);
        };

        let branch = match info.resolution {
            MissingBranchResolution::Surviving(branch) => branch,
            MissingBranchResolution::Ambiguous(candidates) => {
                return Err(Error::msg(format!(
                    "cannot recreate missing worktree {} automatically: multiple local branches ({}) match folder name; restore in main checkout {}",
                    info.worktree_root.display(),
                    candidates.join(", "),
                    info.main_checkout.display()
                )));
            }
            MissingBranchResolution::Gone => {
                return Err(Error::msg(format!(
                    "cannot recreate missing worktree {}: branch no longer exists; restore in main checkout {}",
                    info.worktree_root.display(),
                    info.main_checkout.display()
                )));
            }
        };

        let entries = git.worktrees(&info.main_checkout)?;
        clear_prunable_conflicts(
            &git,
            &info.main_checkout,
            &entries,
            &info.worktree_root,
            &branch,
        )?;

        let target_str = info.worktree_root.to_string_lossy().into_owned();
        if let Err(error) = run_git_checked(
            &git,
            &info.main_checkout,
            &["worktree", "add", "--", &target_str, &branch],
        ) {
            return Err(Error::msg(format!(
                "cannot recreate missing worktree {}: {error}; restore in main checkout {}",
                info.worktree_root.display(),
                info.main_checkout.display()
            )));
        }
        ensure_worktrees_excluded(&git, &info.main_checkout, &info.common_dir)?;
        Ok(Some(info.worktree_root))
    }
}

pub(crate) fn worktree_remove(core: &AppCore, dir: &Path, target: &str) -> Result<WorktreeRemoved> {
    core.files.worktree_remove_from_pool(&core.db, dir, target)
}

pub(crate) fn worktree_checkout(
    core: &AppCore,
    cwd: &Path,
    project_id: Option<&str>,
) -> Result<WorktreeCheckout> {
    let hint = if let Some(id) = project_id {
        let conn = core.db.get()?;
        repo::project::get(&conn, id)?.map(|p| PathBuf::from(p.cwd))
    } else {
        let conn = core.db.get()?;
        repo::project::find_for_path(&conn, &cwd.to_string_lossy())?.map(|p| PathBuf::from(p.cwd))
    };
    core.files.worktree_checkout_state(cwd, hint.as_deref())
}

pub(crate) fn resolve_worktree_base_dir<'a>(
    explicit_cwd: Option<&'a str>,
    project_cwd: Option<&'a str>,
    role_working_dir: Option<&'a str>,
) -> Result<&'a Path> {
    explicit_cwd
        .or(project_cwd)
        .or(role_working_dir)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(Path::new)
        .ok_or_else(|| {
            Error::msg("cannot create a worktree without a folder or project repository")
        })
}
