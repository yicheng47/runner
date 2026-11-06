//! The Files view's protocol (#634): roots, folder listings, git status,
//! worktrees and file contents served by `runnerd`.
//!
//! Every path inside a root is relative to it, uses `/` separators on every
//! platform, and is empty for the root itself.

use crate::companion::TerminalBytes;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

/// The largest file `files_read` returns: 10 MB.
pub const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;
/// A file whose first 8 KB holds a NUL byte is binary.
pub const BINARY_SNIFF_BYTES: usize = 8 * 1024;
/// How many chosen folders `files_recent_folders` remembers.
pub const RECENT_FOLDERS: usize = 10;
/// The most files `files_index` returns.
pub const MAX_INDEX_FILES: usize = 200_000;
/// The most `files_index` may read and return, in bytes of encoded JSON: half a
/// wire frame, leaving ample room for the response around the paths.
pub const MAX_INDEX_BYTES: usize = 32 * 1024 * 1024;
/// The most changed files and the most ignored paths a [`GitStatus`] lists.
pub const MAX_STATUS_PATHS: usize = 200_000;
/// The most its changed files and its ignored paths may take, in bytes of
/// encoded JSON: half a wire frame between them, so that a status always fits
/// in a `files/status` event, with the ignored paths' share kept for them.
pub const MAX_STATUS_ENTRY_BYTES: usize = 24 * 1024 * 1024;
pub const MAX_STATUS_IGNORED_BYTES: usize = 8 * 1024 * 1024;

pub const FILES_CHANGED_EVENT: &str = "files/changed";
pub const FILES_STATUS_EVENT: &str = "files/status";

/// What a Files view belongs to: a chat or terminal session, or a mission.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum FilesSubject {
    Session(String),
    Mission(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeKind {
    Main,
    Linked,
}

/// A root inside a git checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCheckout {
    /// The checkout's top, which is the root unless a folder inside it was chosen.
    pub top: PathBuf,
    pub worktree: WorktreeKind,
    /// The branch's short name; `None` when HEAD is detached.
    pub branch: Option<String>,
    /// HEAD's short commit; `None` before the first commit.
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesRootSource {
    /// The folder saved for the subject with `files_set_root`.
    Saved,
    /// The subject's own checkout, from its working directory.
    Own,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesRoot {
    pub path: PathBuf,
    /// The checkout's folder name, or the folder's for a plain root.
    pub name: String,
    /// `None` for a plain folder outside git.
    pub git: Option<GitCheckout>,
    /// The subject's working directory below the root, relative to it; `None`
    /// when it is the root itself or outside it.
    pub cwd: Option<String>,
    pub source: FilesRootSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FilesRootOutcome {
    Root(FilesRoot),
    /// A chat or terminal outside a project, with no saved root.
    ChooseFolder,
    /// The subject's own checkout no longer exists.
    Missing {
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Folder,
    File,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub kind: EntryKind,
    /// Gitignored, from the root's last git status.
    pub ignored: bool,
    /// Bytes, for files only.
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileStatus {
    Modified,
    /// Added to the index, or copied from another path.
    Added,
    /// Kept for #1141; the tree leaves deleted paths out.
    Deleted,
    Renamed {
        from: String,
    },
    Untracked,
    Unmerged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusEntry {
    /// A nested repository or a submodule ends in `/`: a folder to its parent
    /// checkout, which marks it as changed but holds no file of that name.
    pub path: String,
    pub status: FileStatus,
}

impl StatusEntry {
    /// Whether the entry is a file on disk: not deleted, and not a folder.
    pub fn is_changed_file(&self) -> bool {
        self.status != FileStatus::Deleted && !self.path.ends_with('/')
    }
}

/// Every file under a root that git does not ignore, for finding one by name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIndex {
    /// Relative to the root, sorted.
    pub paths: Vec<String>,
    /// The root holds more than the limits above, so some are missing.
    pub truncated: bool,
}

/// What a `files_index` may still read. Everything read counts, kept or not, so
/// the work is bounded as well as the reply.
#[derive(Debug, Clone)]
pub struct IndexBudget {
    files: usize,
    bytes: usize,
}

impl Default for IndexBudget {
    fn default() -> Self {
        Self::new(MAX_INDEX_FILES, MAX_INDEX_BYTES)
    }
}

impl IndexBudget {
    pub fn new(files: usize, bytes: usize) -> Self {
        Self { files, bytes }
    }

    /// The bytes left, an upper bound on the raw bytes still worth reading.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Spends `path`'s size as a JSON string in an array, or returns false
    /// when it does not fit.
    #[must_use]
    pub fn take(&mut self, path: &[u8]) -> bool {
        let encoded = path.iter().fold(3, |size, byte| {
            size + match byte {
                0..0x20 => 6,
                b'"' | b'\\' => 2,
                _ => 1,
            }
        });
        if self.files == 0 || encoded > self.bytes {
            return false;
        }
        self.files -= 1;
        self.bytes -= encoded;
        true
    }
}

/// A root's `git status`, with paths relative to the root.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatus {
    pub entries: Vec<StatusEntry>,
    /// The changed files the tree shows, listed or not: every entry but the
    /// deleted and the folders. A lower bound if git's output itself ran past
    /// what `runnerd` reads, 256 MB.
    pub changed: usize,
    /// Ignored paths as git reports them with `--ignored=matching`: a wholly
    /// ignored folder once, ending in `/`, and ignored files one by one.
    pub ignored: Vec<String>,
    /// The root itself is ignored, or lies inside an ignored folder, so
    /// everything under it is.
    pub root_ignored: bool,
    /// The checkout reported more than the limits above, so some paths are
    /// missing.
    #[serde(default)]
    pub truncated: bool,
    /// `ignored`, indexed by the first lookup; never on the wire.
    #[serde(skip)]
    pub ignored_index: IgnoredIndex,
}

/// Lets a lookup cost the path's depth rather than the length of
/// [`GitStatus::ignored`], which can hold 200,000 paths. A copy of a status
/// indexes itself again, and two statuses compare by their fields alone.
#[derive(Debug, Default)]
pub struct IgnoredIndex(OnceLock<IgnoredSets>);

#[derive(Debug)]
struct IgnoredSets {
    folders: HashSet<String>,
    files: HashSet<String>,
}

impl Clone for IgnoredIndex {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl PartialEq for IgnoredIndex {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for IgnoredIndex {}

impl IgnoredSets {
    fn new(ignored: &[String]) -> Self {
        let mut sets = Self {
            folders: HashSet::new(),
            files: HashSet::with_capacity(ignored.len()),
        };
        for path in ignored {
            match path.strip_suffix('/') {
                Some(folder) => sets.folders.insert(folder.to_owned()),
                None => sets.files.insert(path.clone()),
            };
        }
        sets
    }

    fn holds(&self, path: &str) -> bool {
        self.files.contains(path)
            || self.folders.contains(path)
            || path
                .match_indices('/')
                .any(|(slash, _)| self.folders.contains(&path[..slash]))
    }
}

impl GitStatus {
    pub fn new(entries: Vec<StatusEntry>, ignored: Vec<String>) -> Self {
        let changed = entries
            .iter()
            .filter(|entry| entry.is_changed_file())
            .count();
        Self {
            entries,
            changed,
            ignored,
            root_ignored: false,
            truncated: false,
            ignored_index: IgnoredIndex::default(),
        }
    }

    /// Whether `path` (a file, or a folder with or without its trailing `/`)
    /// is ignored itself or lies inside an ignored folder.
    pub fn is_ignored(&self, path: &str) -> bool {
        if self.root_ignored {
            return true;
        }
        self.ignored_index
            .0
            .get_or_init(|| IgnoredSets::new(&self.ignored))
            .holds(path.trim_end_matches('/'))
    }

    /// The folders carrying a changed-dot; see [`rollup`].
    pub fn rollup(&self) -> BTreeSet<String> {
        rollup(&self.entries, |folder| self.is_ignored(folder))
    }
}

/// The folders that show a dot because something beneath them changed, as
/// paths relative to the root without a trailing `/`. Deleted paths, which
/// the tree leaves out, do not count, and a folder for which `ignored`
/// answers true never carries one.
pub fn rollup(entries: &[StatusEntry], ignored: impl Fn(&str) -> bool) -> BTreeSet<String> {
    let mut folders = BTreeSet::new();
    for entry in entries {
        if entry.status == FileStatus::Deleted {
            continue;
        }
        let mut path = entry.path.as_str();
        while let Some((parent, _)) = path.rsplit_once('/') {
            if !folders.insert(parent.to_owned()) {
                break;
            }
            path = parent;
        }
    }
    folders.retain(|folder| !ignored(folder));
    folders
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    /// The branch's short name; `None` when detached.
    pub branch: Option<String>,
    /// HEAD's short commit; `None` before the first commit.
    pub commit: Option<String>,
    pub kind: WorktreeKind,
    pub locked: bool,
    pub prunable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileContent {
    /// The file's bytes, base64 on the wire; the app decodes them.
    Text {
        bytes: TerminalBytes,
        size: u64,
        modified: Option<chrono::DateTime<chrono::Utc>>,
    },
    Binary {
        size: u64,
        /// The file's bytes, only when `files_read` asked for them, as the
        /// viewer does for a Markdown file's local images.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bytes: Option<TerminalBytes>,
    },
    /// Over [`MAX_READ_BYTES`], so not read.
    TooLarge {
        size: u64,
    },
    Missing,
}

/// `files/changed`: paths under a watched root changed on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesChanged {
    pub root: PathBuf,
    /// Folders whose listing may have changed; `""` is the root.
    pub folders: Vec<String>,
    /// The changed paths themselves, files or folders. Empty when the watcher
    /// cannot say what changed, so anything under the root may have.
    pub paths: Vec<String>,
}

/// `files/status`: a watched root's git status changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesStatusChanged {
    pub root: PathBuf,
    pub status: GitStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, status: FileStatus) -> StatusEntry {
        StatusEntry {
            path: path.into(),
            status,
        }
    }

    /// What the index replaced, the definition of an ignored path.
    fn scanned(ignored: &[String], path: &str) -> bool {
        let path = path.trim_end_matches('/');
        ignored
            .iter()
            .any(|ignored| match ignored.strip_suffix('/') {
                Some(folder) => path == folder || path.starts_with(ignored.as_str()),
                None => path == ignored,
            })
    }

    #[test]
    fn the_ignored_index_answers_as_a_scan_of_the_list_does() {
        let ignored = [
            "target/",
            "a/build/",
            "a/z.log",
            "a/z.log.d/",
            "é/ü/",
            "top.log",
        ]
        .map(String::from)
        .to_vec();
        let status = GitStatus::new(vec![], ignored.clone());
        for path in [
            "",
            "target",
            "target/",
            "target/debug",
            "target/debug/x/",
            "targets",
            "targets/x",
            "a",
            "a/",
            "a/build",
            "a/build/x",
            "a/builds",
            "a/z.log",
            "a/z.log/x",
            "a/z.log.d",
            "a/z.log.d/x",
            "b/target",
            "é",
            "é/ü",
            "é/ü/x",
            "top.log",
            "top.log/x",
            "/target",
            "target//x",
        ] {
            assert_eq!(status.is_ignored(path), scanned(&ignored, path), "{path:?}");
        }
    }

    #[test]
    fn a_copy_of_a_status_indexes_its_own_list_and_equals_it() {
        let status = GitStatus::new(vec![], vec!["target/".into()]);
        assert!(status.is_ignored("target/x"));
        let mut copy = status.clone();
        assert_eq!(copy, status, "the index plays no part in equality");
        copy.ignored = vec!["other/".into()];
        assert!(!copy.is_ignored("target/x"));
        assert!(copy.is_ignored("other/x"));
    }

    #[test]
    fn the_index_never_reaches_the_wire() {
        let status = GitStatus::new(vec![], vec!["target/".into()]);
        assert!(status.is_ignored("target"));
        let wire = serde_json::to_value(&status).unwrap();
        assert!(wire.get("ignored_index").is_none(), "{wire}");
        let back: GitStatus = serde_json::from_value(wire).unwrap();
        assert!(back.is_ignored("target"));
    }

    /// Reviewers measured 618 ms for this shape in an optimized build, one
    /// folder per change, when each folder scanned the ignored list; an
    /// unoptimized test build took many times that.
    #[test]
    fn rollup_stays_cheap_over_many_folders_and_ignored_paths() {
        let count = 30_000;
        let entries = (0..count)
            .map(|index| entry(&format!("src/d{index:06}/f.rs"), FileStatus::Modified))
            .collect();
        let ignored = (0..count)
            .map(|index| format!("build/d{index:06}/"))
            .collect();
        let status = GitStatus::new(entries, ignored);
        let start = std::time::Instant::now();
        assert_eq!(status.rollup().len(), count + 1);
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "rollup took {elapsed:?}"
        );
    }

    #[test]
    fn rollup_marks_every_ancestor_of_a_change() {
        let status = GitStatus::new(
            vec![
                entry("src/deep/x.rs", FileStatus::Modified),
                entry("src/lib.rs", FileStatus::Untracked),
                entry("top.txt", FileStatus::Added),
                entry(
                    "docs/new.md",
                    FileStatus::Renamed {
                        from: "old.md".into(),
                    },
                ),
            ],
            vec![],
        );
        assert_eq!(
            status.rollup().into_iter().collect::<Vec<_>>(),
            ["docs", "src", "src/deep"]
        );
        assert_eq!(status.changed, 4);
    }

    #[test]
    fn deleted_paths_neither_count_nor_dot() {
        let status = GitStatus::new(vec![entry("gone/a.rs", FileStatus::Deleted)], vec![]);
        assert!(status.rollup().is_empty());
        assert_eq!(status.changed, 0);
    }

    #[test]
    fn an_ignored_folder_never_carries_a_dot() {
        let status = GitStatus::new(
            vec![
                entry("target/tracked/x.rs", FileStatus::Modified),
                entry("src/a.rs", FileStatus::Modified),
            ],
            vec!["target/".into(), "z.log".into()],
        );
        assert_eq!(status.rollup().into_iter().collect::<Vec<_>>(), ["src"]);
        assert!(status.is_ignored("target"));
        assert!(status.is_ignored("target/"));
        assert!(status.is_ignored("target/debug/build"));
        assert!(status.is_ignored("z.log"));
        assert!(!status.is_ignored("targets"));
        assert!(!status.is_ignored("z.log.txt"));
        assert!(!status.is_ignored("src"));
    }

    #[test]
    fn an_ignored_root_ignores_everything_beneath_it() {
        let mut status = GitStatus::new(vec![entry("tracked/x.rs", FileStatus::Modified)], vec![]);
        status.root_ignored = true;
        assert!(status.is_ignored(""));
        assert!(status.is_ignored("debug/build"));
        assert!(status.rollup().is_empty());
    }

    #[test]
    fn a_folder_entry_dots_its_folder_but_is_no_changed_file() {
        let status = GitStatus::new(
            vec![
                entry("nested/", FileStatus::Untracked),
                entry("vendor/sub/", FileStatus::Modified),
                entry("a.rs", FileStatus::Modified),
            ],
            vec![],
        );
        assert_eq!(status.changed, 1);
        assert_eq!(
            status.rollup().into_iter().collect::<Vec<_>>(),
            ["nested", "vendor", "vendor/sub"]
        );
    }

    #[test]
    fn a_status_from_before_truncated_still_reads() {
        let status: GitStatus = serde_json::from_value(serde_json::json!({
            "entries": [], "changed": 0, "ignored": [], "root_ignored": false
        }))
        .unwrap();
        assert!(!status.truncated);
    }

    #[test]
    fn subjects_and_outcomes_keep_their_wire_shape() {
        assert_eq!(
            serde_json::to_value(FilesSubject::Mission("m1".into())).unwrap(),
            serde_json::json!({"kind": "mission", "id": "m1"})
        );
        assert_eq!(
            serde_json::to_value(FilesRootOutcome::Missing {
                path: "/gone".into()
            })
            .unwrap(),
            serde_json::json!({"kind": "missing", "path": "/gone"})
        );
        let text = FileContent::Text {
            bytes: TerminalBytes(b"hi\n".to_vec()),
            size: 3,
            modified: None,
        };
        assert_eq!(
            serde_json::to_value(&text).unwrap(),
            serde_json::json!({"kind": "text", "bytes": "aGkK", "size": 3, "modified": null})
        );
    }

    #[test]
    fn binary_bytes_travel_only_when_asked_for_and_old_requests_still_read() {
        let plain = FileContent::Binary {
            size: 3,
            bytes: None,
        };
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            serde_json::json!({"kind": "binary", "size": 3})
        );
        let with_bytes = FileContent::Binary {
            size: 3,
            bytes: Some(TerminalBytes(b"\0hi".to_vec())),
        };
        let wire = serde_json::to_value(&with_bytes).unwrap();
        assert_eq!(wire["bytes"], "AGhp");
        assert_eq!(
            serde_json::from_value::<FileContent>(wire).unwrap(),
            with_bytes
        );
        assert_eq!(
            serde_json::from_value::<FileContent>(serde_json::json!({"kind": "binary", "size": 3}))
                .unwrap(),
            plain
        );

        // A client from before `want_bytes` leaves it out; it reads as false.
        let old: crate::protocol::api::Request = serde_json::from_value(
            serde_json::json!({"files_read": {"root": "/repo", "path": "a.png"}}),
        )
        .unwrap();
        assert!(matches!(
            old,
            crate::protocol::api::Request::files_read {
                want_bytes: None,
                ..
            }
        ));
    }
}
