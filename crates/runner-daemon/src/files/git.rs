//! `git` as a subprocess, resolved through the login-shell PATH, and the
//! parsers for its porcelain output.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use runner_core::protocol::files::{
    FileStatus, GitStatus, IndexBudget, StatusEntry, WorktreeEntry, WorktreeKind,
    MAX_STATUS_ENTRY_BYTES, MAX_STATUS_IGNORED_BYTES, MAX_STATUS_PATHS,
};

use crate::error::{Error, Result};

/// The most of `git status`'s output read before stopping git: paths past the
/// limits are only counted, as the ignored paths come last.
const MAX_STATUS_READ_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Git {
    exe: PathBuf,
    path_env: String,
    /// Set once this git has refused `git worktree list -z`, as gits before
    /// 2.36 do; shared by every `Git` a [`super::FilesService`] hands out.
    pub(super) worktree_list_without_z: Arc<AtomicBool>,
}

/// Where a folder sits in its git checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoPaths {
    pub top: PathBuf,
    /// The checkout's own git directory: `<top>/.git`, or
    /// `<main>/.git/worktrees/<name>` for a linked worktree.
    pub git_dir: PathBuf,
    /// The directory holding the branches, shared by every worktree of the
    /// repository: `<main>/.git`, the same as `git_dir` in the main checkout.
    pub common_dir: PathBuf,
    /// The folder below the top, relative and ending in `/`; empty at the top.
    pub prefix: String,
}

/// The checkout holding `dir`, found as git finds it: the nearest folder,
/// `dir`'s physical path included, with a `.git` folder or file. Returns its
/// top and `dir`'s prefix below it (relative, ending in `/`, empty at the
/// top), as git names them through symlinks, without running git.
pub fn checkout_top(dir: &Path) -> Option<(PathBuf, String)> {
    let dir = dir.canonicalize().ok()?;
    let mut parts = Vec::new();
    let mut folder = dir.as_path();
    loop {
        if folder.join(".git").exists() {
            parts.reverse();
            let prefix = parts.iter().map(|part| format!("{part}/")).collect();
            return Some((folder.to_path_buf(), prefix));
        }
        parts.push(folder.file_name()?.to_string_lossy().into_owned());
        folder = folder.parent()?;
    }
}

impl Git {
    pub fn find(path_env: String) -> Option<Self> {
        let exe = runner_core::protocol::runtime_paths::find_executable("git", &path_env)?;
        Some(Self {
            exe,
            path_env,
            worktree_list_without_z: Default::default(),
        })
    }

    pub(super) fn command(&self, dir: &Path) -> Command {
        let mut command = Command::new(&self.exe);
        command
            .current_dir(dir)
            .env("PATH", &self.path_env)
            // This setting names a program for every status to run. It is the
            // only one turned off: a repository's own clean filters still run,
            // as they do for any git command the user runs there.
            .args(["-c", "core.fsmonitor=false"])
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_COMMON_DIR")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::session::process::prepare_headless_fork(&mut command);
        command
    }

    pub(super) fn run(&self, dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
        let output = self
            .command(dir)
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

    /// `None` outside a git checkout.
    pub fn rev_parse(&self, dir: &Path) -> Option<RepoPaths> {
        let stdout = self
            .run(
                dir,
                &[
                    "rev-parse",
                    "--show-toplevel",
                    "--absolute-git-dir",
                    "--show-prefix",
                    "--git-common-dir",
                ],
            )
            .ok()?;
        let stdout = String::from_utf8_lossy(&stdout);
        let mut lines = stdout.split('\n');
        let top = lines.next().filter(|line| !line.is_empty())?;
        let git_dir = lines.next().filter(|line| !line.is_empty())?;
        let prefix = lines.next().unwrap_or_default();
        // Relative to `dir` unless it is elsewhere, as in a linked worktree.
        let common_dir = lines.next().filter(|line| !line.is_empty())?;
        Some(RepoPaths {
            top: PathBuf::from(top),
            git_dir: PathBuf::from(git_dir),
            common_dir: dir.join(common_dir),
            prefix: prefix.to_owned(),
        })
    }

    /// The status of the folder `dir`, which is `prefix` below its checkout's
    /// top, within the limits of [`StatusBudget::default`].
    pub fn status(&self, dir: &Path, prefix: &str) -> Result<GitStatus> {
        self.status_reading(dir, prefix, MAX_STATUS_READ_BYTES)
    }

    fn status_reading(&self, dir: &Path, prefix: &str, limit: usize) -> Result<GitStatus> {
        let fail = |error: std::io::Error| Error::msg(format!("run git: {error}"));
        let mut child = self
            .command(dir)
            .args([
                "--no-optional-locks",
                "status",
                "--porcelain=v2",
                "-z",
                "--untracked-files=all",
                "--ignored=matching",
                "--",
                ".",
            ])
            .spawn()
            .map_err(fail)?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let errors = std::thread::spawn(move || {
            let mut text = Vec::new();
            let _ = (&mut stderr).take(64 * 1024).read_to_end(&mut text);
            String::from_utf8_lossy(&text).trim().to_owned()
        });
        let read = read_status(stdout, prefix, StatusBudget::default(), limit);
        if !matches!(read, Ok((_, false))) {
            // git is left mid-write, and ends only when told to.
            let _ = child.kill();
        }
        let exit = child.wait().map_err(fail)?;
        let errors = errors.join().unwrap_or_default();
        let (mut status, overflowed) = read.map_err(fail)?;
        if overflowed {
            status.truncated = true;
        } else if !exit.success() {
            return Err(Error::msg(format!("git status: {errors}")));
        }
        Ok(status)
    }

    /// Whether `dir` is, or is inside, a folder git ignores, where `ls-files`
    /// lists nothing though the tree shows everything.
    pub fn ignores(&self, dir: &Path) -> bool {
        self.command(dir)
            .args(["check-ignore", "-q", "--", "."])
            .output()
            .is_ok_and(|output| output.status.success())
    }

    /// The files under `dir` that git does not ignore, tracked or not,
    /// relative to `dir`, and whether `budget` ran out first, which stops git.
    /// Tracked files since deleted, submodules and nested repositories are
    /// not files on disk, so they are left out.
    pub fn files(&self, dir: &Path, budget: &mut IndexBudget) -> Result<(Vec<String>, bool)> {
        let fail = |error: std::io::Error| Error::msg(format!("run git: {error}"));
        let mut child = self
            .command(dir)
            .args([
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
            ])
            .stderr(Stdio::null())
            .spawn()
            .map_err(fail)?;
        let stdout = child.stdout.take().expect("stdout is piped");
        // One more than the budget, so that a record too long to fit ends the
        // read rather than filling memory.
        let mut reader = BufReader::new(stdout.take(budget.bytes() as u64 + 1));
        let mut files = Vec::new();
        let mut record = Vec::new();
        let mut truncated = false;
        let read = loop {
            record.clear();
            match reader.read_until(0, &mut record) {
                Ok(0) => break Ok(()),
                Ok(_) => {}
                Err(error) => break Err(error),
            }
            if record.pop() != Some(0) || !budget.take(&record) {
                truncated = true;
                break Ok(());
            }
            if let Some(path) = file_on_disk(dir, &record) {
                files.push(path);
            }
        };
        if truncated || read.is_err() {
            // git is left mid-write, and ends only when told to.
            let _ = child.kill();
        }
        let status = child.wait().map_err(fail)?;
        read.map_err(fail)?;
        if !truncated && !status.success() {
            return Err(Error::msg(format!("git ls-files: {status}")));
        }
        files.sort_unstable();
        files.dedup();
        Ok((files, truncated))
    }

    /// One run, except on a git before 2.36, which lacks `-z`: there the
    /// first list is refused and retried without it, and later lists run
    /// without it once (Jason, 2026-11-04).
    pub fn worktrees(&self, dir: &Path) -> Result<Vec<WorktreeEntry>> {
        let lines = |git: &Self| {
            git.run(dir, &["worktree", "list", "--porcelain"])
                .map(|stdout| parse_worktrees(&stdout, b'\n'))
        };
        if self.worktree_list_without_z.load(Ordering::Relaxed) {
            return lines(self);
        }
        match self.run(dir, &["worktree", "list", "--porcelain", "-z"]) {
            Ok(stdout) => Ok(parse_worktrees(&stdout, b'\0')),
            Err(error) => {
                let worktrees = lines(self).map_err(|_| error)?;
                self.worktree_list_without_z.store(true, Ordering::Relaxed);
                Ok(worktrees)
            }
        }
    }
}

/// A record of `git ls-files` as a path, if it names a file that is there.
fn file_on_disk(dir: &Path, record: &[u8]) -> Option<String> {
    let path = std::str::from_utf8(record).ok()?;
    if path.is_empty() || path.ends_with('/') {
        return None;
    }
    let kind = dir.join(path).symlink_metadata().ok()?.file_type();
    (!kind.is_dir()).then(|| path.to_owned())
}

/// Parses `git status` as it comes, up to `limit` bytes of it, and whether
/// there was more. One byte past the limit is read to tell.
fn read_status(
    stdout: impl Read,
    prefix: &str,
    budget: StatusBudget,
    limit: usize,
) -> std::io::Result<(GitStatus, bool)> {
    let mut read = 0;
    let mut overflowed = false;
    let mut failed = None;
    let fields = BufReader::new(stdout.take(limit as u64 + 1))
        .split(0)
        .map_while(|field| {
            let field = field.map_err(|error| failed = Some(error)).ok()?;
            read += field.len() + 1;
            overflowed = read > limit;
            (!overflowed).then_some(field)
        });
    let status = parse_status_fields(fields, prefix, budget);
    match failed {
        Some(error) => Err(error),
        None => Ok((status, overflowed)),
    }
}

/// What a [`GitStatus`] may still list. Past it, paths are only counted.
pub struct StatusBudget {
    entries: usize,
    entry_bytes: usize,
    ignored: usize,
    ignored_bytes: usize,
}

impl Default for StatusBudget {
    fn default() -> Self {
        Self::new(
            MAX_STATUS_PATHS,
            MAX_STATUS_ENTRY_BYTES,
            MAX_STATUS_IGNORED_BYTES,
        )
    }
}

impl StatusBudget {
    pub fn new(paths: usize, entry_bytes: usize, ignored_bytes: usize) -> Self {
        Self {
            entries: paths,
            entry_bytes,
            ignored: paths,
            ignored_bytes,
        }
    }

    /// Spends `value`'s size in encoded JSON, with a comma, from `bytes`, and
    /// one path from `paths`; false, spending nothing, when it does not fit.
    fn take(paths: &mut usize, bytes: &mut usize, value: &impl serde::Serialize) -> bool {
        let size =
            serde_json::to_vec(value).map_or(usize::MAX, |json| json.len().saturating_add(1));
        if *paths == 0 || size > *bytes {
            return false;
        }
        *paths -= 1;
        *bytes -= size;
        true
    }
}

/// Parses `git status --porcelain=v2 -z`, split at its NULs. Paths are relative
/// to the checkout's top; `prefix` is the root's folder below it, and paths
/// outside that folder are dropped. Every field is read whatever `budget`
/// allows, so the count stays right and the ignored paths, which come last,
/// are kept when the changes alone run out of it.
pub fn parse_status_fields(
    mut fields: impl Iterator<Item = impl AsRef<[u8]>>,
    prefix: &str,
    mut budget: StatusBudget,
) -> GitStatus {
    let mut entries = Vec::new();
    let mut changed = 0;
    let mut ignored = Vec::new();
    let mut root_ignored = false;
    let mut entries_full = false;
    let mut ignored_full = false;
    let relative = |path: &str| path.strip_prefix(prefix).map(str::to_owned);
    let mut add = |entry: StatusEntry| {
        changed += usize::from(entry.is_changed_file());
        entries_full = entries_full
            || !StatusBudget::take(&mut budget.entries, &mut budget.entry_bytes, &entry);
        if !entries_full {
            entries.push(entry);
        }
    };
    while let Some(field) = fields.next() {
        let field = String::from_utf8_lossy(field.as_ref()).into_owned();
        let Some((kind, rest)) = field.split_once(' ') else {
            continue;
        };
        let (xy, sub, path, from) = match kind {
            "1" => match rest.splitn(8, ' ').collect::<Vec<_>>()[..] {
                [xy, sub, _, _, _, _, _, path] => (xy, sub, path, None),
                _ => continue,
            },
            "2" => {
                let from = fields
                    .next()
                    .map(|from| String::from_utf8_lossy(from.as_ref()).into_owned());
                match rest.splitn(9, ' ').collect::<Vec<_>>()[..] {
                    [xy, sub, _, _, _, _, _, _, path] => (xy, sub, path, from),
                    _ => continue,
                }
            }
            "u" => {
                if let [_, _, _, _, _, _, _, _, _, path] =
                    rest.splitn(10, ' ').collect::<Vec<_>>()[..]
                {
                    if let Some(path) = relative(path) {
                        add(StatusEntry {
                            path,
                            status: FileStatus::Unmerged,
                        });
                    }
                }
                continue;
            }
            "?" => {
                if let Some(path) = relative(rest) {
                    add(StatusEntry {
                        path,
                        status: FileStatus::Untracked,
                    });
                }
                continue;
            }
            "!" => {
                // Inside an ignored folder, git names that folder, an
                // ancestor of the root or the root itself.
                if !prefix.is_empty() && rest.ends_with('/') && prefix.starts_with(rest) {
                    root_ignored = true;
                } else if let Some(path) = relative(rest) {
                    ignored_full = ignored_full
                        || !StatusBudget::take(
                            &mut budget.ignored,
                            &mut budget.ignored_bytes,
                            &path,
                        );
                    if !ignored_full {
                        ignored.push(path);
                    }
                }
                continue;
            }
            _ => continue,
        };
        let Some(mut path) = relative(path) else {
            continue;
        };
        if sub.starts_with('S') {
            path.push('/');
        }
        let mut codes = xy.chars();
        let (x, y) = (codes.next().unwrap_or('.'), codes.next().unwrap_or('.'));
        let status = if y == 'D' {
            FileStatus::Deleted
        } else if x == 'R' {
            let from = from.unwrap_or_default();
            FileStatus::Renamed {
                from: relative(&from).unwrap_or(from),
            }
        } else if x == 'A' || x == 'C' {
            FileStatus::Added
        } else if x == 'D' {
            FileStatus::Deleted
        } else {
            FileStatus::Modified
        };
        add(StatusEntry { path, status });
    }
    GitStatus {
        entries,
        changed,
        ignored,
        root_ignored,
        truncated: entries_full || ignored_full,
        ignored_index: Default::default(),
    }
}

/// Parses `git worktree list --porcelain`, whose fields end with `separator`
/// (NUL under `-z`, a newline otherwise) and whose records end with an empty
/// field. The first record is the main checkout.
pub fn parse_worktrees(bytes: &[u8], separator: u8) -> Vec<WorktreeEntry> {
    let mut worktrees = Vec::new();
    let mut current: Option<WorktreeEntry> = None;
    for field in bytes.split(|byte| *byte == separator) {
        let field = String::from_utf8_lossy(field);
        if field.is_empty() {
            worktrees.extend(current.take());
            continue;
        }
        let (key, value) = field.split_once(' ').unwrap_or((&field, ""));
        if key == "worktree" {
            worktrees.extend(current.take());
            current = Some(WorktreeEntry {
                path: PathBuf::from(value),
                branch: None,
                commit: None,
                kind: if worktrees.is_empty() {
                    WorktreeKind::Main
                } else {
                    WorktreeKind::Linked
                },
                locked: false,
                prunable: false,
            });
            continue;
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        match key {
            "HEAD" => entry.commit = short_commit(value),
            "branch" => {
                entry.branch = Some(
                    value
                        .strip_prefix("refs/heads/")
                        .unwrap_or(value)
                        .to_owned(),
                )
            }
            "locked" => entry.locked = true,
            "prunable" => entry.prunable = true,
            _ => {}
        }
    }
    worktrees.extend(current);
    worktrees
}

fn short_commit(sha: &str) -> Option<String> {
    (!sha.is_empty() && sha.bytes().any(|byte| byte != b'0')).then(|| sha.chars().take(7).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_status(bytes: &[u8], prefix: &str) -> GitStatus {
        let fields = bytes.split(|byte| *byte == 0);
        parse_status_fields(fields, prefix, StatusBudget::default())
    }

    fn entry(path: &str, status: FileStatus) -> StatusEntry {
        StatusEntry {
            path: path.into(),
            status,
        }
    }

    const H: &str = "1111111111111111111111111111111111111111";

    fn porcelain(records: &[&str]) -> Vec<u8> {
        let mut bytes = records.join("\0").into_bytes();
        bytes.push(0);
        bytes
    }

    #[test]
    fn status_parses_every_record_kind() {
        let bytes = porcelain(&[
            &format!("1 .M N... 100644 100644 100644 {H} {H} src/a.rs"),
            &format!("1 A. N... 000000 100644 100644 {H} {H} added.txt"),
            &format!("1 AM N... 000000 100644 100644 {H} {H} added then edited.txt"),
            &format!("1 .D N... 100644 100644 000000 {H} {H} gone.txt"),
            &format!("1 D. N... 100644 000000 000000 {H} {H} staged gone.txt"),
            &format!("1 MM N... 100644 100644 100644 {H} {H} both.txt"),
            &format!("2 R. N... 100644 100644 100644 {H} {H} R100 new name.txt"),
            "old name.txt",
            &format!("2 C. N... 100644 100644 100644 {H} {H} C75 copy.txt"),
            "copied from.txt",
            &format!("u UU N... 100644 100644 100644 100644 {H} {H} {H} conflict.rs"),
            "? untracked dir/file with spaces.txt",
            "? new\nline.txt",
            "! target/",
            "! z.log",
        ]);
        let status = parse_status(&bytes, "");
        assert_eq!(
            status.entries,
            vec![
                entry("src/a.rs", FileStatus::Modified),
                entry("added.txt", FileStatus::Added),
                entry("added then edited.txt", FileStatus::Added),
                entry("gone.txt", FileStatus::Deleted),
                entry("staged gone.txt", FileStatus::Deleted),
                entry("both.txt", FileStatus::Modified),
                entry(
                    "new name.txt",
                    FileStatus::Renamed {
                        from: "old name.txt".into()
                    }
                ),
                entry("copy.txt", FileStatus::Added),
                entry("conflict.rs", FileStatus::Unmerged),
                entry("untracked dir/file with spaces.txt", FileStatus::Untracked),
                entry("new\nline.txt", FileStatus::Untracked),
            ]
        );
        assert_eq!(status.ignored, ["target/", "z.log"]);
        assert_eq!(status.changed, 9);
    }

    #[test]
    fn status_paths_are_made_relative_to_the_root_folder() {
        let bytes = porcelain(&[
            &format!("1 .M N... 100644 100644 100644 {H} {H} src/deep/x.rs"),
            &format!("2 R. N... 100644 100644 100644 {H} {H} R100 src/b.rs"),
            "src/a.rs",
            &format!("1 .M N... 100644 100644 100644 {H} {H} other/y.rs"),
            "? src/new.rs",
            "! src/gen/",
        ]);
        let status = parse_status(&bytes, "src/");
        assert_eq!(
            status.entries,
            vec![
                entry("deep/x.rs", FileStatus::Modified),
                entry(
                    "b.rs",
                    FileStatus::Renamed {
                        from: "a.rs".into()
                    }
                ),
                entry("new.rs", FileStatus::Untracked),
            ]
        );
        assert_eq!(status.ignored, ["gen/"]);
    }

    #[test]
    fn a_root_inside_an_ignored_folder_is_ignored() {
        for prefix in ["target/", "target/debug/"] {
            let status = parse_status(&porcelain(&["! target/"]), prefix);
            assert!(status.root_ignored, "{prefix}");
            assert!(status.ignored.is_empty());
        }
        let status = parse_status(&porcelain(&["! target/", "! src/gen/"]), "src/");
        assert!(!status.root_ignored);
        assert_eq!(status.ignored, ["gen/"]);
    }

    #[test]
    fn nested_repositories_and_submodules_are_folders() {
        let bytes = porcelain(&[
            &format!("1 .M S..U 160000 160000 160000 {H} {H} vendor/sub"),
            "? nested/",
            &format!("1 .M N... 100644 100644 100644 {H} {H} a.rs"),
        ]);
        let status = parse_status(&bytes, "");
        assert_eq!(
            status.entries,
            vec![
                entry("vendor/sub/", FileStatus::Modified),
                entry("nested/", FileStatus::Untracked),
                entry("a.rs", FileStatus::Modified),
            ]
        );
        assert_eq!(status.changed, 1);
    }

    #[test]
    fn a_status_past_its_limits_lists_what_fits_and_counts_the_rest() {
        let bytes = porcelain(&[
            &format!("1 .M N... 100644 100644 100644 {H} {H} a.rs"),
            "? b.rs",
            "? c.rs",
            "! target/",
            "! z.log",
        ]);
        let parse = |budget| parse_status_fields(bytes.split(|byte| *byte == 0), "", budget);

        let status = parse(StatusBudget::new(2, 1024, 1024));
        assert_eq!(
            status.entries,
            vec![
                entry("a.rs", FileStatus::Modified),
                entry("b.rs", FileStatus::Untracked),
            ]
        );
        assert_eq!(status.changed, 3);
        assert_eq!(status.ignored, ["target/", "z.log"], "their own budget");
        assert!(status.truncated);

        let status = parse(StatusBudget::new(1, 1024, 1024));
        assert_eq!(status.ignored, ["target/"]);
        assert!(status.truncated);

        let fits = parse(StatusBudget::new(3, 1024, 1024));
        assert_eq!(fits.entries.len(), 3);
        assert!(!parse(StatusBudget::new(9, 1024, 1024)).truncated);

        // The byte budget is the entry's size as JSON, comma included.
        let one = serde_json::to_vec(&entry("a.rs", FileStatus::Modified))
            .unwrap()
            .len()
            + 1;
        assert_eq!(parse(StatusBudget::new(9, one, 1024)).entries.len(), 1);
        assert_eq!(parse(StatusBudget::new(9, one - 1, 1024)).entries.len(), 0);
    }

    #[test]
    fn reading_stops_at_the_limit_wherever_it_falls() {
        // Each record is 6 bytes with its NUL.
        let bytes = porcelain(&["? a.r", "? b.r", "? c.r"]);
        let read = |limit| {
            let (status, overflowed) =
                read_status(&bytes[..], "", StatusBudget::default(), limit).unwrap();
            let paths = status.entries.into_iter().map(|entry| entry.path);
            (paths.collect::<Vec<_>>(), overflowed)
        };
        assert_eq!(
            read(100),
            (vec!["a.r".into(), "b.r".into(), "c.r".into()], false)
        );
        assert_eq!(
            read(bytes.len()),
            (vec!["a.r".into(), "b.r".into(), "c.r".into()], false),
            "output ending exactly at the limit is complete"
        );
        assert_eq!(
            read(bytes.len() - 6),
            (vec!["a.r".into(), "b.r".into()], true),
            "the limit on a NUL, with more to come"
        );
        assert_eq!(
            read(bytes.len() - 7),
            (vec!["a.r".into()], true),
            "the limit inside a record, which is not half read"
        );
        assert_eq!(read(0), (vec![], true));
    }

    #[test]
    fn a_status_cut_off_at_the_limit_is_truncated_rather_than_failed() {
        let fixture = crate::files::fixture::repo();
        let path = fixture.path();
        for index in 0..2_000 {
            std::fs::write(path.join(format!("new-{index}.txt")), "n").unwrap();
        }
        let whole = fixture.git.status(path, "").unwrap();
        assert!(!whole.truncated);
        assert_eq!(whole.changed, 2_000);
        for limit in [0, 64, 4_096] {
            let status = fixture.git.status_reading(path, "", limit).unwrap();
            assert!(status.truncated, "{limit}");
            assert!(status.changed < 2_000, "{limit}");
        }
    }

    #[test]
    fn a_status_at_its_limits_fits_a_wire_frame() {
        // The longest record per entry byte: every byte a control character,
        // which JSON writes as six.
        let path = "\u{1}".repeat(4096);
        let mut bytes = Vec::new();
        for index in 0..2_000 {
            bytes.extend_from_slice(format!("? {index}{path}\0").as_bytes());
        }
        for index in 0..1_000 {
            bytes.extend_from_slice(format!("! {index}{path}\0").as_bytes());
        }
        let status = parse_status(&bytes, "");
        assert!(status.truncated);
        assert_eq!(status.changed, 2_000);
        let event = runner_core::protocol::files::FilesStatusChanged {
            root: "/repo".into(),
            status,
        };
        let frame =
            runner_core::protocol::wire::Frame::json(runner_core::protocol::wire::EVENT, &event)
                .unwrap();
        frame.check_size().unwrap();
    }

    #[test]
    fn empty_status_is_clean() {
        assert_eq!(parse_status(b"", ""), GitStatus::default());
    }

    #[test]
    fn worktrees_parse_main_linked_detached_locked_and_prunable() {
        let records = [
            "worktree /repo/main",
            "HEAD 0123456789abcdef0123456789abcdef01234567",
            "branch refs/heads/main",
            "",
            "worktree /repo/.worktrees/feat with space",
            "HEAD 89abcdef0123456789abcdef0123456789abcdef",
            "branch refs/heads/feat/634-x",
            "",
            "worktree /repo/det",
            "HEAD 89abcdef0123456789abcdef0123456789abcdef",
            "detached",
            "",
            "worktree /repo/lk",
            "HEAD 89abcdef0123456789abcdef0123456789abcdef",
            "branch refs/heads/lk",
            "locked busy now",
            "",
            "worktree /repo/gone",
            "HEAD 89abcdef0123456789abcdef0123456789abcdef",
            "branch refs/heads/gone",
            "prunable gitdir file points to non-existent location",
            "",
        ];
        let expected = vec![
            WorktreeEntry {
                path: "/repo/main".into(),
                branch: Some("main".into()),
                commit: Some("0123456".into()),
                kind: WorktreeKind::Main,
                locked: false,
                prunable: false,
            },
            WorktreeEntry {
                path: "/repo/.worktrees/feat with space".into(),
                branch: Some("feat/634-x".into()),
                commit: Some("89abcde".into()),
                kind: WorktreeKind::Linked,
                locked: false,
                prunable: false,
            },
            WorktreeEntry {
                path: "/repo/det".into(),
                branch: None,
                commit: Some("89abcde".into()),
                kind: WorktreeKind::Linked,
                locked: false,
                prunable: false,
            },
            WorktreeEntry {
                path: "/repo/lk".into(),
                branch: Some("lk".into()),
                commit: Some("89abcde".into()),
                kind: WorktreeKind::Linked,
                locked: true,
                prunable: false,
            },
            WorktreeEntry {
                path: "/repo/gone".into(),
                branch: Some("gone".into()),
                commit: Some("89abcde".into()),
                kind: WorktreeKind::Linked,
                locked: false,
                prunable: true,
            },
        ];
        let mut nul = records.join("\0").into_bytes();
        nul.push(0);
        assert_eq!(parse_worktrees(&nul, b'\0'), expected);
        let mut lines = records.join("\n").into_bytes();
        lines.push(b'\n');
        assert_eq!(parse_worktrees(&lines, b'\n'), expected);
    }

    #[test]
    fn a_worktree_before_its_first_commit_has_no_commit() {
        let bytes = b"worktree /new\0HEAD 0000000000000000000000000000000000000000\0branch refs/heads/main\0\0";
        let worktrees = parse_worktrees(bytes, b'\0');
        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].commit, None);
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
    }
}
