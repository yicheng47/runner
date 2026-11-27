use super::*;

pub(super) fn working_path_size(abs_path: &Path) -> Result<u64> {
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

pub(super) fn read_working_bytes(abs_path: &Path) -> Result<Vec<u8>> {
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

pub(super) fn is_binary_bytes(bytes: &[u8]) -> bool {
    let sniff = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
    sniff.contains(&0)
}

pub(super) fn hunks_for_added_text(bytes: &[u8]) -> Vec<DiffHunk> {
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

pub(super) struct WorkingRawInfo {
    pub(super) mode: &'static str,
    pub(super) symlink_blob_id: Option<String>,
    pub(super) symlink_target_bytes: Option<Vec<u8>>,
    pub(super) binary: bool,
    pub(super) added_lines: u64,
}

pub(super) fn inspect_working_meta(abs_path: &Path) -> Option<(&'static str, Option<String>)> {
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

pub(super) fn inspect_working_raw(abs_path: &Path) -> Option<WorkingRawInfo> {
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

pub(super) fn hash_working_files_with_git(
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

pub(super) fn hash_single_working_path(
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

pub(super) fn no_index_numstat(
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

pub(super) fn diff_hunks_no_index(
    git: &Git,
    old_bytes: &[u8],
    new_bytes: &[u8],
) -> Result<Vec<DiffHunk>> {
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
