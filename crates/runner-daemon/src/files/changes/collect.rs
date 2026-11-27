use super::*;

pub(super) enum DiffContext {
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
    pub(super) fn is_unborn(&self) -> bool {
        matches!(self, Self::Uncommitted { unborn: true, .. })
    }
}

pub(super) struct InternalDiffEntry {
    pub(super) checkout_rel_path: String,
    pub(super) root_rel_path: String,
    pub(super) checkout_old_path: Option<String>,
    pub(super) status: FileStatus,
    pub(super) counts: DiffCounts,
    pub(super) old_mode: String,
    pub(super) new_mode: String,
    pub(super) old_blob_id: Option<String>,
    pub(super) new_blob_id: Option<String>,
    pub(super) conflicted: bool,
    pub(super) in_index: bool,
    pub(super) raw_diff_needed: bool,
}

pub(super) fn collect_changed_entries(
    git: &Git,
    repo: &RepoPaths,
    root: &Path,
    target: &ChangesTarget,
    base: Option<&str>,
) -> Result<(DiffContext, Vec<InternalDiffEntry>)> {
    match target {
        ChangesTarget::Uncommitted => collect_uncommitted_entries(git, repo, root),
        ChangesTarget::Branch => collect_branch_entries(git, repo, root, base),
        ChangesTarget::Commit(id) => collect_commit_entries(git, repo, root, id),
    }
}

fn collect_branch_entries(
    git: &Git,
    repo: &RepoPaths,
    root: &Path,
    base: Option<&str>,
) -> Result<(DiffContext, Vec<InternalDiffEntry>)> {
    let Some(head_sha) = try_run_git(
        git,
        &repo.top,
        &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
    )?
    .map(trimmed_utf8)
    .filter(|s| !s.is_empty()) else {
        return Ok((
            DiffContext::Commit {
                parent_sha: EMPTY_TREE_SHA.to_owned(),
                commit_sha: EMPTY_TREE_SHA.to_owned(),
            },
            Vec::new(),
        ));
    };

    let Some(merge_base) = resolve_branch_merge_base(git, &repo.top, base)? else {
        return Ok((
            DiffContext::Commit {
                parent_sha: head_sha.clone(),
                commit_sha: head_sha,
            },
            Vec::new(),
        ));
    };

    if merge_base == head_sha {
        return Ok((
            DiffContext::Commit {
                parent_sha: merge_base,
                commit_sha: head_sha,
            },
            Vec::new(),
        ));
    }

    diff_tree_entries(git, repo, root, merge_base, head_sha)
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

    diff_tree_entries(git, repo, root, parent_sha, commit_sha)
}

fn diff_tree_entries(
    git: &Git,
    repo: &RepoPaths,
    root: &Path,
    parent_sha: String,
    commit_sha: String,
) -> Result<(DiffContext, Vec<InternalDiffEntry>)> {
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

pub(super) fn mode_change_note(old_mode: &str, new_mode: &str) -> Option<ModeChange> {
    if old_mode == "000000" || new_mode == "000000" || old_mode == new_mode {
        None
    } else {
        Some(ModeChange {
            old: old_mode.to_owned(),
            new: new_mode.to_owned(),
        })
    }
}
