//! The Files tree's model: folder listings as they load, which folders are
//! open, git status, the flattened rows the list draws, and the selection the
//! keyboard moves.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use runner_core::protocol::files::{DirEntry, EntryKind, FileStatus, GitStatus};

/// Typing within this long of the last key extends the type-ahead prefix.
const TYPE_AHEAD_WINDOW: Duration = Duration::from_millis(900);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Letter {
    Modified,
    Added,
    Renamed,
    Untracked,
    Unmerged,
}

impl Letter {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Renamed => "R",
            Self::Untracked => "U",
            Self::Unmerged => "!",
        }
    }

    fn from_status(status: &FileStatus) -> Option<Self> {
        match status {
            FileStatus::Modified => Some(Self::Modified),
            FileStatus::Added => Some(Self::Added),
            FileStatus::Renamed { .. } => Some(Self::Renamed),
            FileStatus::Untracked => Some(Self::Untracked),
            FileStatus::Unmerged => Some(Self::Unmerged),
            FileStatus::Deleted => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Row {
    /// Relative to the root, `/`-separated.
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) depth: usize,
    pub(crate) folder: bool,
    pub(crate) expanded: bool,
    pub(crate) ignored: bool,
    pub(crate) letter: Option<Letter>,
    pub(crate) dot: bool,
    pub(crate) cwd: bool,
}

/// What a key did to the tree.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Moved {
    /// Folders that were opened without a listing and need one.
    pub(crate) load: Vec<String>,
    pub(crate) changed: bool,
}

#[derive(Default)]
pub(crate) struct FileTree {
    listings: HashMap<String, Vec<DirEntry>>,
    expanded: BTreeSet<String>,
    status: Option<GitStatus>,
    letters: HashMap<String, Letter>,
    dots: BTreeSet<String>,
    cwd: Option<String>,
    rows: Vec<Row>,
    selected: Option<String>,
    type_ahead: String,
    typed_at: Option<Instant>,
}

pub(crate) fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

pub(crate) fn parent(path: &str) -> Option<&str> {
    if path.is_empty() {
        return None;
    }
    Some(path.rsplit_once('/').map_or("", |(parent, _)| parent))
}

impl FileTree {
    /// A tree whose working directory below the root, when there is one,
    /// opens expanded with its ancestors.
    pub(crate) fn new(cwd: Option<String>) -> Self {
        let mut tree = Self {
            cwd: cwd.filter(|cwd| !cwd.is_empty()),
            ..Self::default()
        };
        if let Some(cwd) = tree.cwd.clone() {
            let mut folder = Some(cwd.as_str());
            while let Some(path) = folder.filter(|path| !path.is_empty()) {
                tree.expanded.insert(path.to_owned());
                folder = parent(path);
            }
        }
        tree
    }

    /// The folders to list first: the root and every folder open from the start.
    pub(crate) fn initial_loads(&self) -> Vec<String> {
        std::iter::once(String::new())
            .chain(self.expanded.iter().cloned())
            .collect()
    }

    pub(crate) fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub(crate) fn status(&self) -> Option<&GitStatus> {
        self.status.as_ref()
    }

    pub(crate) fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        let selected = self.selected.as_deref()?;
        self.rows.iter().position(|row| row.path == selected)
    }

    /// The folders whose listing is loaded, to refresh after the daemon
    /// reconnects.
    pub(crate) fn loaded_folders(&self) -> Vec<String> {
        let mut folders = self.listings.keys().cloned().collect::<Vec<_>>();
        folders.sort();
        folders
    }

    /// Of the folders `files/changed` names, those this tree has listed.
    pub(crate) fn listed(&self, folders: &[String]) -> Vec<String> {
        folders
            .iter()
            .filter(|folder| self.listings.contains_key(folder.as_str()))
            .cloned()
            .collect()
    }

    pub(crate) fn set_listing(&mut self, folder: &str, entries: Vec<DirEntry>) {
        self.listings.insert(folder.to_owned(), entries);
        self.rebuild();
    }

    /// A folder that can no longer be listed, as when it was deleted.
    pub(crate) fn drop_listing(&mut self, folder: &str) {
        let prefix = format!("{folder}/");
        self.listings
            .retain(|path, _| path != folder && !path.starts_with(&prefix));
        self.rebuild();
    }

    pub(crate) fn set_status(&mut self, status: GitStatus) {
        self.letters = status
            .entries
            .iter()
            .filter_map(|entry| Some((entry.path.clone(), Letter::from_status(&entry.status)?)))
            .collect();
        self.dots = status.rollup();
        self.status = Some(status);
        self.rebuild();
    }

    pub(crate) fn select(&mut self, path: &str) -> bool {
        if self.selected.as_deref() == Some(path) {
            return false;
        }
        self.selected = Some(path.to_owned());
        true
    }

    /// The status letter git gives `path`.
    pub(crate) fn letter(&self, path: &str) -> Option<Letter> {
        self.letters.get(path).copied()
    }

    /// Selects `path` with every folder above it open, as switching file
    /// tabs does; the folders still unlisted are returned to load.
    pub(crate) fn reveal(&mut self, path: &str) -> Moved {
        let mut load = Vec::new();
        let mut opened = false;
        let mut folder = parent(path);
        let mut folders = Vec::new();
        while let Some(path) = folder.filter(|path| !path.is_empty()) {
            folders.push(path.to_owned());
            folder = parent(path);
        }
        for folder in folders.into_iter().rev() {
            if self.expanded.insert(folder.clone()) {
                opened = true;
                if !self.listings.contains_key(&folder) {
                    load.push(folder);
                }
            }
        }
        if opened {
            self.rebuild();
        }
        let selected = self.select(path);
        Moved {
            load,
            changed: opened || selected,
        }
    }

    /// Opens a collapsed folder or closes an open one.
    pub(crate) fn toggle(&mut self, path: &str) -> Moved {
        if self.expanded.contains(path) {
            self.expanded.remove(path);
            self.rebuild();
            Moved {
                load: Vec::new(),
                changed: true,
            }
        } else {
            self.expand(path)
        }
    }

    pub(crate) fn expand(&mut self, path: &str) -> Moved {
        if !self.expanded.insert(path.to_owned()) {
            return Moved::default();
        }
        self.rebuild();
        // The watcher drops events under ignored folders, so an ignored folder
        // shown again, or one open inside it, is listed again rather than
        // trusted from before.
        let inside = format!("{path}/");
        let mut load = self
            .rows
            .iter()
            .filter(|row| row.folder && row.expanded && row.ignored)
            .filter(|row| row.path == path || row.path.starts_with(&inside))
            .filter(|row| self.listings.contains_key(&row.path))
            .map(|row| row.path.clone())
            .collect::<Vec<_>>();
        if !self.listings.contains_key(path) {
            load.insert(0, path.to_owned());
        }
        Moved {
            load,
            changed: true,
        }
    }

    pub(crate) fn move_down(&mut self) -> Moved {
        let next = match self.selected_index() {
            Some(index) => (index + 1).min(self.rows.len().saturating_sub(1)),
            None => 0,
        };
        self.select_index(next)
    }

    pub(crate) fn move_up(&mut self) -> Moved {
        let previous = match self.selected_index() {
            Some(index) => index.saturating_sub(1),
            None => self.rows.len().saturating_sub(1),
        };
        self.select_index(previous)
    }

    /// → opens a closed folder, or moves into an open one.
    pub(crate) fn move_right(&mut self) -> Moved {
        let Some(index) = self.selected_index() else {
            return self.select_index(0);
        };
        let row = &self.rows[index];
        if !row.folder {
            return Moved::default();
        }
        if !row.expanded {
            let path = row.path.clone();
            return self.expand(&path);
        }
        match self.rows.get(index + 1) {
            Some(child) if child.depth > row.depth => self.select_index(index + 1),
            _ => Moved::default(),
        }
    }

    /// ← closes an open folder, or moves to the parent.
    pub(crate) fn move_left(&mut self) -> Moved {
        let Some(index) = self.selected_index() else {
            return Moved::default();
        };
        let row = &self.rows[index];
        if row.folder && row.expanded {
            let path = row.path.clone();
            return self.toggle(&path);
        }
        match parent(&row.path).filter(|parent| !parent.is_empty()) {
            Some(parent) => {
                let parent = parent.to_owned();
                Moved {
                    load: Vec::new(),
                    changed: self.select(&parent),
                }
            }
            None => Moved::default(),
        }
    }

    /// Typing jumps to the next row starting with the letters typed so far.
    pub(crate) fn type_ahead(&mut self, text: &str, now: Instant) -> Moved {
        let continuing = self
            .typed_at
            .is_some_and(|typed_at| now.duration_since(typed_at) < TYPE_AHEAD_WINDOW);
        if !continuing {
            self.type_ahead.clear();
        }
        self.typed_at = Some(now);
        self.type_ahead.push_str(&text.to_lowercase());
        if self.rows.is_empty() {
            return Moved::default();
        }
        let current = self.selected_index();
        // A new prefix starts looking after the selection; a longer one keeps it
        // when it still matches.
        let start = match (current, continuing) {
            (Some(index), true) => index,
            (Some(index), false) => index + 1,
            (None, _) => 0,
        };
        let count = self.rows.len();
        let prefix = self.type_ahead.clone();
        let found = (0..count)
            .map(|offset| (start + offset) % count)
            .find(|&index| self.rows[index].name.to_lowercase().starts_with(&prefix));
        match found {
            Some(index) => self.select_index(index),
            None => Moved::default(),
        }
    }

    fn select_index(&mut self, index: usize) -> Moved {
        let Some(path) = self.rows.get(index).map(|row| row.path.clone()) else {
            return Moved::default();
        };
        Moved {
            load: Vec::new(),
            changed: self.select(&path),
        }
    }

    fn ignored(&self, path: &str, entry: &DirEntry) -> bool {
        match &self.status {
            Some(status) => status.is_ignored(path),
            None => entry.ignored,
        }
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        self.push_rows("", 0, false, &mut rows);
        self.rows = rows;
    }

    fn push_rows(&self, folder: &str, depth: usize, ignored_above: bool, rows: &mut Vec<Row>) {
        let Some(entries) = self.listings.get(folder) else {
            return;
        };
        for entry in entries {
            let path = join(folder, &entry.name);
            let folder_row = entry.kind == EntryKind::Folder;
            let letter = self.letters.get(&path).copied();
            // Deleted entries still on disk are rare; a path git only knows as
            // deleted never shows, since the listing comes from the disk.
            let ignored = ignored_above || self.ignored(&path, entry);
            let expanded = folder_row && self.expanded.contains(&path);
            rows.push(Row {
                name: entry.name.clone(),
                depth,
                folder: folder_row,
                expanded,
                ignored,
                letter: if folder_row || ignored { None } else { letter },
                dot: folder_row && !ignored && self.dots.contains(&path),
                cwd: self.cwd.as_deref() == Some(path.as_str()),
                path: path.clone(),
            });
            if expanded {
                self.push_rows(&path, depth + 1, ignored, rows);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_core::protocol::files::StatusEntry;

    fn folder(name: &str) -> DirEntry {
        DirEntry {
            name: name.into(),
            kind: EntryKind::Folder,
            ignored: false,
            size: None,
        }
    }

    fn file(name: &str) -> DirEntry {
        DirEntry {
            name: name.into(),
            kind: EntryKind::File,
            ignored: false,
            size: Some(1),
        }
    }

    fn change(path: &str, status: FileStatus) -> StatusEntry {
        StatusEntry {
            path: path.into(),
            status,
        }
    }

    fn paths(tree: &FileTree) -> Vec<&str> {
        tree.rows().iter().map(|row| row.path.as_str()).collect()
    }

    fn sample() -> FileTree {
        let mut tree = FileTree::new(None);
        tree.set_listing(
            "",
            vec![
                folder("crates"),
                folder("docs"),
                folder("target"),
                file("Cargo.lock"),
                file("README.md"),
            ],
        );
        tree
    }

    #[test]
    fn folders_open_lazily_and_close_keeping_their_listing() {
        let mut tree = sample();
        assert_eq!(
            paths(&tree),
            ["crates", "docs", "target", "Cargo.lock", "README.md"]
        );
        let moved = tree.toggle("crates");
        assert_eq!(moved.load, ["crates"]);
        assert_eq!(paths(&tree).len(), 5, "no rows until the listing arrives");
        tree.set_listing("crates", vec![folder("app"), file("lib.rs")]);
        assert_eq!(
            paths(&tree),
            [
                "crates",
                "crates/app",
                "crates/lib.rs",
                "docs",
                "target",
                "Cargo.lock",
                "README.md"
            ]
        );
        assert_eq!(tree.rows()[1].depth, 1);
        assert!(tree.rows()[0].expanded);

        assert!(tree.toggle("crates").load.is_empty());
        assert_eq!(paths(&tree).len(), 5);
        assert!(
            tree.toggle("crates").load.is_empty(),
            "reopening reuses the listing"
        );
        assert_eq!(paths(&tree).len(), 7);
    }

    #[test]
    fn status_letters_dots_and_ignored_entries() {
        let mut tree = sample();
        tree.toggle("crates");
        tree.set_listing(
            "crates",
            vec![
                folder("app"),
                file("lib.rs"),
                file("new.rs"),
                file("gone.rs"),
            ],
        );
        tree.toggle("target");
        tree.set_listing("target", vec![file("build.rs")]);
        assert!(tree.status().is_none());
        tree.set_status(GitStatus::new(
            vec![
                change("crates/lib.rs", FileStatus::Modified),
                change("crates/new.rs", FileStatus::Untracked),
                change("crates/app/x.rs", FileStatus::Added),
                change("crates/gone.rs", FileStatus::Deleted),
                change("Cargo.lock", FileStatus::Modified),
                change(
                    "README.md",
                    FileStatus::Renamed {
                        from: "OLD.md".into(),
                    },
                ),
            ],
            vec!["target/".into()],
        ));
        let row = |path: &str| {
            tree.rows()
                .iter()
                .find(|row| row.path == path)
                .unwrap()
                .clone()
        };
        assert_eq!(row("crates/lib.rs").letter, Some(Letter::Modified));
        assert_eq!(row("crates/new.rs").letter, Some(Letter::Untracked));
        assert_eq!(row("crates/gone.rs").letter, None);
        assert_eq!(row("README.md").letter, Some(Letter::Renamed));
        assert!(row("crates").dot);
        assert!(row("crates/app").dot);
        assert!(!row("docs").dot);
        assert!(row("target").ignored);
        assert!(row("target/build.rs").ignored, "inside an ignored folder");
        assert!(!row("target").dot);
        assert!(!row("crates").ignored);
        assert_eq!(tree.status().unwrap().changed, 5);
    }

    #[test]
    fn before_status_ignored_marks_come_from_the_listing() {
        let mut tree = FileTree::new(None);
        let mut target = folder("target");
        target.ignored = true;
        tree.set_listing("", vec![target, folder("src")]);
        assert!(tree.rows()[0].ignored);
        assert!(!tree.rows()[1].ignored);
    }

    #[test]
    fn revealing_a_file_opens_its_folders_and_selects_it() {
        let mut tree = sample();
        tree.toggle("crates");
        tree.set_listing("crates", vec![folder("app"), file("lib.rs")]);
        tree.toggle("crates");
        let moved = tree.reveal("crates/app/src/main.rs");
        assert_eq!(moved.load, ["crates/app", "crates/app/src"]);
        assert!(moved.changed);
        assert_eq!(tree.selected(), Some("crates/app/src/main.rs"));
        assert!(tree.rows().iter().any(|row| row.path == "crates/app"));
        tree.set_listing("crates/app", vec![folder("src")]);
        tree.set_listing("crates/app/src", vec![file("main.rs")]);
        assert!(tree.selected_index().is_some());
        assert_eq!(tree.reveal("crates/app/src/main.rs"), Moved::default());
        assert_eq!(tree.letter("crates/app/src/main.rs"), None);
    }

    #[test]
    fn the_working_directory_opens_expanded_and_tagged() {
        let mut tree = FileTree::new(Some("crates/app".into()));
        assert_eq!(tree.initial_loads(), ["", "crates", "crates/app"]);
        tree.set_listing("", vec![folder("crates"), file("a")]);
        tree.set_listing("crates", vec![folder("app"), folder("cli")]);
        tree.set_listing("crates/app", vec![file("main.rs")]);
        assert_eq!(
            paths(&tree),
            [
                "crates",
                "crates/app",
                "crates/app/main.rs",
                "crates/cli",
                "a"
            ]
        );
        let tagged = tree
            .rows()
            .iter()
            .filter(|row| row.cwd)
            .map(|row| row.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(tagged, ["crates/app"]);
    }

    #[test]
    fn arrows_move_open_and_close() {
        let mut tree = sample();
        assert!(tree.move_down().changed);
        assert_eq!(tree.selected(), Some("crates"));
        let moved = tree.move_right();
        assert_eq!(moved.load, ["crates"]);
        tree.set_listing("crates", vec![file("lib.rs")]);
        assert_eq!(
            tree.selected(),
            Some("crates"),
            "→ on a closed folder opens it"
        );
        tree.move_right();
        assert_eq!(
            tree.selected(),
            Some("crates/lib.rs"),
            "→ on an open one steps in"
        );
        assert!(!tree.move_right().changed, "→ on a file does nothing");
        tree.move_left();
        assert_eq!(
            tree.selected(),
            Some("crates"),
            "← on a file goes to its parent"
        );
        tree.move_left();
        assert!(!tree.rows()[0].expanded, "← on an open folder closes it");
        assert_eq!(tree.selected(), Some("crates"));
        assert!(!tree.move_left().changed, "← at the top does nothing");
        tree.move_down();
        tree.move_down();
        assert_eq!(tree.selected(), Some("target"));
        tree.move_up();
        assert_eq!(tree.selected(), Some("docs"));
        for _ in 0..10 {
            tree.move_down();
        }
        assert_eq!(tree.selected(), Some("README.md"), "stops at the last row");
        tree.move_up();
        tree.move_up();
        tree.move_up();
        tree.move_up();
        assert!(!tree.move_up().changed, "stops at the first row");
    }

    #[test]
    fn type_ahead_jumps_by_name_and_extends_while_typing() {
        let mut tree = FileTree::new(None);
        tree.set_listing(
            "",
            vec![
                folder("crates"),
                file("Cargo.lock"),
                file("Cargo.toml"),
                file("README.md"),
            ],
        );
        let start = Instant::now();
        tree.type_ahead("c", start);
        assert_eq!(tree.selected(), Some("crates"));
        tree.type_ahead("a", start + Duration::from_millis(100));
        assert_eq!(tree.selected(), Some("Cargo.lock"), "ca extends the prefix");
        tree.type_ahead("rgo.t", start + Duration::from_millis(200));
        assert_eq!(tree.selected(), Some("Cargo.toml"));
        tree.type_ahead("c", start + Duration::from_secs(5));
        assert_eq!(
            tree.selected(),
            Some("crates"),
            "a pause starts over and wraps to the next match"
        );
        tree.type_ahead("c", start + Duration::from_secs(10));
        assert_eq!(tree.selected(), Some("Cargo.lock"));
        assert!(
            !tree
                .type_ahead("zz", start + Duration::from_secs(20))
                .changed
        );
    }

    #[test]
    fn reopening_an_ignored_folder_lists_it_again_with_its_open_folders() {
        let mut tree = sample();
        tree.set_status(GitStatus::new(vec![], vec!["target/".into()]));
        assert_eq!(tree.toggle("target").load, ["target"]);
        tree.set_listing("target", vec![folder("debug")]);
        assert_eq!(tree.toggle("target/debug").load, ["target/debug"]);
        tree.set_listing("target/debug", vec![file("out")]);
        tree.toggle("target");
        assert_eq!(
            tree.toggle("target").load,
            ["target", "target/debug"],
            "reopened, with the open folder inside"
        );
        tree.toggle("crates");
        tree.set_listing("crates", vec![file("lib.rs")]);
        tree.toggle("crates");
        assert!(
            tree.toggle("crates").load.is_empty(),
            "a folder the watcher covers keeps its listing"
        );
    }

    #[test]
    fn refreshing_a_listing_keeps_expansion_and_selection() {
        let mut tree = sample();
        tree.toggle("crates");
        tree.set_listing("crates", vec![folder("app"), file("lib.rs")]);
        tree.toggle("crates/app");
        tree.set_listing("crates/app", vec![file("main.rs")]);
        tree.select("crates/app/main.rs");
        assert_eq!(
            tree.listed(&["crates".into(), "elsewhere".into()]),
            ["crates"]
        );

        tree.set_listing(
            "crates",
            vec![folder("app"), file("lib.rs"), file("new.rs")],
        );
        assert_eq!(
            paths(&tree)[..5],
            [
                "crates",
                "crates/app",
                "crates/app/main.rs",
                "crates/lib.rs",
                "crates/new.rs"
            ]
        );
        assert_eq!(tree.selected(), Some("crates/app/main.rs"));
        assert_eq!(tree.selected_index(), Some(2));

        tree.drop_listing("crates/app");
        assert_eq!(tree.loaded_folders(), ["", "crates"]);
        assert_eq!(tree.selected_index(), None, "its rows left with it");
    }
}
