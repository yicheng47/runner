use super::*;

pub(super) fn git_cmd(git: &Git, dir: &Path) -> Command {
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

pub(super) fn run_git(git: &Git, dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
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

pub(super) fn try_run_git(git: &Git, dir: &Path, args: &[&str]) -> Result<Option<Vec<u8>>> {
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

pub(super) fn run_git_diff_allow_changes(git: &Git, dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
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

pub(super) fn write_blob_to_temp(
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

pub(super) fn trimmed_utf8(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).trim().to_owned()
}

pub(super) fn short_sha(full: &str) -> String {
    full.chars().take(7).collect()
}

pub(super) fn resolve_auto_base(git: &Git, top: &Path) -> Result<Option<(String, String)>> {
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

pub(crate) fn worktree_auto_base(files: &super::FilesService, top: &Path) -> Option<String> {
    let git = files.git()?;
    resolve_auto_base(&git, top)
        .ok()
        .flatten()
        .map(|(name, _)| name)
}

/// Verifies `requested` as a real local (`refs/heads/…`) or remote-tracking
/// (`refs/remotes/…`) branch ref, refusing tags, commit ids, revision suffixes
/// and `HEAD` symrefs. Returns `(short_name, full_ref)` when valid.
pub(super) fn verify_branch_ref(
    git: &Git,
    top: &Path,
    requested: &str,
) -> Result<Option<(String, String)>> {
    if requested.is_empty()
        || requested.starts_with('-')
        || requested == "HEAD"
        || requested.bytes().any(|b| b <= 0x20 || b == 0x7f)
    {
        return Ok(None);
    }

    let mut candidates: Vec<(String, String)> = Vec::new();
    if let Some(short) = requested.strip_prefix("refs/heads/") {
        if !short.is_empty() {
            candidates.push((short.to_owned(), requested.to_owned()));
        }
    } else if let Some(short) = requested.strip_prefix("refs/remotes/") {
        if short.contains('/') && !short.ends_with("/HEAD") {
            candidates.push((short.to_owned(), requested.to_owned()));
        }
    } else if requested.starts_with("refs/") {
        return Ok(None);
    } else if requested.contains('/') && !requested.ends_with("/HEAD") {
        let remotes = list_remotes(git, top)?;
        let starts_with_remote = remotes.iter().any(|remote| {
            requested
                .strip_prefix(remote.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        });
        let local_candidate = (requested.to_owned(), format!("refs/heads/{requested}"));
        let remote_candidate = (requested.to_owned(), format!("refs/remotes/{requested}"));
        if starts_with_remote {
            candidates.push(remote_candidate);
            candidates.push(local_candidate);
        } else {
            candidates.push(local_candidate);
            candidates.push(remote_candidate);
        }
    } else if !requested.contains('/') {
        candidates.push((requested.to_owned(), format!("refs/heads/{requested}")));
    }

    for (short_name, full_ref) in candidates {
        let Some(sym) = try_run_git(
            git,
            top,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                "--symbolic-full-name",
                &full_ref,
            ],
        )?
        else {
            continue;
        };
        if trimmed_utf8(sym) != full_ref {
            continue;
        }
        let commit_query = format!("{full_ref}^{{commit}}");
        let Some(sha) = try_run_git(
            git,
            top,
            &["rev-parse", "--verify", "--quiet", &commit_query],
        )?
        else {
            continue;
        };
        if !trimmed_utf8(sha).is_empty() {
            return Ok(Some((short_name, full_ref)));
        }
    }
    Ok(None)
}

pub(super) fn resolve_branch_merge_base(
    git: &Git,
    top: &Path,
    base: Option<&str>,
) -> Result<Option<String>> {
    if let Some(requested) = base
        && let Some((_, full_ref)) = verify_branch_ref(git, top, requested)?
    {
        return Ok(try_run_git(git, top, &["merge-base", "HEAD", &full_ref])?
            .map(trimmed_utf8)
            .filter(|s| !s.is_empty()));
    }
    Ok(resolve_auto_base(git, top)?.map(|(_, merge_base)| merge_base))
}

pub(super) fn list_remotes(git: &Git, top: &Path) -> Result<Vec<String>> {
    let remotes = run_git(git, top, &["remote"])?;
    let text = String::from_utf8_lossy(&remotes);
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

fn has_origin_remote(git: &Git, top: &Path) -> Result<bool> {
    Ok(list_remotes(git, top)?
        .iter()
        .any(|remote| remote == "origin"))
}

pub(super) fn git_blob_size(git: &Git, top: &Path, oid: &str) -> Result<u64> {
    let raw = run_git(git, top, &["cat-file", "-s", oid])?;
    trimmed_utf8(raw)
        .parse::<u64>()
        .map_err(|error| Error::msg(format!("parse blob size for {oid}: {error}")))
}

pub(super) fn git_cat_blob(git: &Git, top: &Path, oid: &str) -> Result<Vec<u8>> {
    run_git(git, top, &["cat-file", "blob", oid])
}
