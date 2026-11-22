//! The unified viewer's tabs (#1421): pinned file/diff tabs plus at most one
//! reusable italic preview tab driven by single-click / arrow navigation in
//! Changes.

use crate::surfaces::changes::model::SelectedTarget;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct TabKey {
    /// Relative to the root, `/`-separated.
    pub(crate) path: String,
    /// Diff target associated with this tab. Files opened from the project tree
    /// use `SelectedTarget::Uncommitted` (`Working tree vs HEAD`).
    pub(crate) target: SelectedTarget,
}

impl TabKey {
    pub(crate) fn working_tree(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            target: SelectedTarget::Uncommitted,
        }
    }

    pub(crate) fn new(path: impl Into<String>, target: SelectedTarget) -> Self {
        Self {
            path: path.into(),
            target,
        }
    }

    pub(crate) fn name(&self) -> &str {
        self.path
            .rsplit_once('/')
            .map_or(&self.path, |(_, name)| name)
    }

    pub(crate) fn folder(&self) -> &str {
        self.path.rsplit_once('/').map_or("", |(folder, _)| folder)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileTab {
    pub(crate) key: TabKey,
    /// True for the single reusable italic preview tab opened from Changes.
    pub(crate) preview: bool,
}

impl FileTab {
    pub(crate) fn path(&self) -> &str {
        &self.key.path
    }

    pub(crate) fn name(&self) -> &str {
        self.key.name()
    }

    pub(crate) fn folder(&self) -> &str {
        self.key.folder()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OpenPinnedOutcome {
    Created(TabKey),
    Existing(TabKey),
    /// An existing preview tab for the same path was promoted to pinned.
    Promoted(TabKey),
}

impl OpenPinnedOutcome {
    pub(crate) fn key(&self) -> &TabKey {
        match self {
            Self::Created(k) | Self::Existing(k) | Self::Promoted(k) => k,
        }
    }

    pub(crate) fn is_new(&self) -> bool {
        matches!(self, Self::Created(_))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OpenPreviewOutcome {
    FocusedPinned(TabKey),
    AlreadyPreview(TabKey),
    CreatedPreview(TabKey),
    ReplacedPreview { replaced: TabKey, key: TabKey },
}

impl OpenPreviewOutcome {
    pub(crate) fn key(&self) -> &TabKey {
        match self {
            Self::FocusedPinned(k)
            | Self::AlreadyPreview(k)
            | Self::CreatedPreview(k)
            | Self::ReplacedPreview { key: k, .. } => k,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FileTabs {
    tabs: Vec<FileTab>,
    active: usize,
}

impl FileTabs {
    pub(crate) fn tabs(&self) -> &[FileTab] {
        &self.tabs
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub(crate) fn active_index(&self) -> Option<usize> {
        (!self.tabs.is_empty()).then_some(self.active)
    }

    pub(crate) fn active(&self) -> Option<&FileTab> {
        self.tabs.get(self.active)
    }

    pub(crate) fn position_key(&self, key: &TabKey) -> Option<usize> {
        self.tabs.iter().position(|tab| &tab.key == key)
    }

    pub(crate) fn preview_index(&self) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.preview)
    }

    pub(crate) fn has_duplicate_path(&self, path: &str) -> bool {
        self.tabs
            .iter()
            .filter(|tab| tab.key.path == path)
            .nth(1)
            .is_some()
    }

    /// Opens `path` as a pinned tab (from Files tree, Finder, Markdown link, or
    /// terminal ⌘-click). Focuses an existing pinned working-tree tab, promotes
    /// a matching preview tab of the same path, or inserts a new pinned tab
    /// beside the active one.
    pub(crate) fn open_pinned(&mut self, path: &str) -> OpenPinnedOutcome {
        let working_key = TabKey::working_tree(path);
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| !tab.preview && tab.key == working_key)
        {
            self.active = index;
            return OpenPinnedOutcome::Existing(working_key);
        }
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.preview && tab.key.path == path)
        {
            self.tabs[index].preview = false;
            let key = self.tabs[index].key.clone();
            self.active = index;
            return OpenPinnedOutcome::Promoted(key);
        }
        let at = if self.tabs.is_empty() {
            0
        } else {
            (self.active + 1).min(self.tabs.len())
        };
        self.tabs.insert(
            at,
            FileTab {
                key: working_key.clone(),
                preview: false,
            },
        );
        self.active = at;
        OpenPinnedOutcome::Created(working_key)
    }

    /// Opens `(path, target)` from Changes single-click or `↑↓` / `j` / `k`:
    /// focuses an already-pinned matching `(path, target)` without duplicating,
    /// otherwise creates or swaps the single italic preview tab.
    pub(crate) fn open_preview(
        &mut self,
        path: &str,
        target: SelectedTarget,
    ) -> OpenPreviewOutcome {
        let key = TabKey::new(path, target);
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| !tab.preview && tab.key == key)
        {
            self.active = index;
            return OpenPreviewOutcome::FocusedPinned(key);
        }
        if let Some(index) = self.preview_index() {
            if self.tabs[index].key == key {
                self.active = index;
                return OpenPreviewOutcome::AlreadyPreview(key);
            }
            let replaced = std::mem::replace(&mut self.tabs[index].key, key.clone());
            self.active = index;
            return OpenPreviewOutcome::ReplacedPreview { replaced, key };
        }
        let at = self.tabs.len();
        self.tabs.push(FileTab {
            key: key.clone(),
            preview: true,
        });
        self.active = at;
        OpenPreviewOutcome::CreatedPreview(key)
    }

    /// Promotes `index` from italic preview to a permanent pinned tab.
    /// Returns whether a preview tab was promoted.
    pub(crate) fn pin(&mut self, index: usize) -> bool {
        let Some(tab) = self.tabs.get(index) else {
            return false;
        };
        if !tab.preview {
            return false;
        }
        let key = tab.key.clone();
        if let Some(existing) = self
            .tabs
            .iter()
            .enumerate()
            .position(|(i, t)| i != index && !t.preview && t.key == key)
        {
            self.tabs.remove(index);
            self.active = if index < existing {
                existing - 1
            } else {
                existing
            };
            return true;
        }
        self.tabs[index].preview = false;
        true
    }

    /// Restores a saved tab entry (for LRU `KEPT_VIEWS` eviction).
    pub(crate) fn push_restored(&mut self, key: TabKey, preview: bool) {
        if self.position_key(&key).is_some() {
            return;
        }
        if preview && self.preview_index().is_some() {
            return;
        }
        self.tabs.push(FileTab { key, preview });
    }

    pub(crate) fn activate(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    /// Closes the tab, giving the active place to its right neighbour, or its
    /// left one at the end. Returns the closed tab key.
    pub(crate) fn close(&mut self, index: usize) -> Option<TabKey> {
        if index >= self.tabs.len() {
            return None;
        }
        let closed = self.tabs.remove(index);
        if index < self.active || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        Some(closed.key)
    }

    #[cfg(test)]
    pub(crate) fn close_active(&mut self) -> Option<TabKey> {
        self.active_index().and_then(|index| self.close(index))
    }

    /// Moves to the next tab, or the previous with `back`, wrapping around.
    pub(crate) fn cycle(&mut self, back: bool) {
        let count = self.tabs.len();
        if count == 0 {
            return;
        }
        self.active = if back {
            (self.active + count - 1) % count
        } else {
            (self.active + 1) % count
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(tabs: &FileTabs) -> Vec<&str> {
        tabs.tabs().iter().map(FileTab::path).collect()
    }

    #[test]
    fn every_open_keeps_a_tab_and_an_open_file_switches_to_its_own() {
        let mut tabs = FileTabs::default();
        assert!(tabs.open_pinned("src/a.rs").is_new());
        assert!(tabs.open_pinned("src/b.rs").is_new());
        assert!(tabs.open_pinned("c.md").is_new());
        assert_eq!(paths(&tabs), ["src/a.rs", "src/b.rs", "c.md"]);
        assert_eq!(tabs.active().unwrap().path(), "c.md");
        assert!(
            !tabs.open_pinned("src/a.rs").is_new(),
            "already open: no read"
        );
        assert_eq!(tabs.active_index(), Some(0));
        assert_eq!(paths(&tabs), ["src/a.rs", "src/b.rs", "c.md"]);
    }

    #[test]
    fn a_new_file_opens_beside_the_active_tab() {
        let mut tabs = FileTabs::default();
        tabs.open_pinned("README.md");
        tabs.open_pinned("docs/guide.md");
        tabs.activate(0);
        tabs.open_pinned("a.md");
        assert_eq!(paths(&tabs), ["README.md", "a.md", "docs/guide.md"]);
        assert_eq!(tabs.active().unwrap().path(), "a.md");
    }

    #[test]
    fn preview_tab_reuses_one_slot_focuses_pinned_and_promotes_cleanly() {
        let mut tabs = FileTabs::default();
        // Single-click / step in Changes reuses one italic preview slot.
        assert_eq!(
            tabs.open_preview("src/a.rs", SelectedTarget::Uncommitted),
            OpenPreviewOutcome::CreatedPreview(TabKey::working_tree("src/a.rs"))
        );
        assert!(tabs.active().unwrap().preview);
        assert_eq!(
            tabs.open_preview("src/b.rs", SelectedTarget::Uncommitted),
            OpenPreviewOutcome::ReplacedPreview {
                replaced: TabKey::working_tree("src/a.rs"),
                key: TabKey::working_tree("src/b.rs"),
            }
        );
        assert_eq!(paths(&tabs), ["src/b.rs"]);
        assert!(tabs.active().unwrap().preview);

        // Pinning converts preview into a permanent tab; next preview gets its own single slot.
        assert!(tabs.pin(0));
        assert!(!tabs.active().unwrap().preview);
        assert_eq!(
            tabs.open_preview("src/c.rs", SelectedTarget::Uncommitted),
            OpenPreviewOutcome::CreatedPreview(TabKey::working_tree("src/c.rs"))
        );
        assert_eq!(paths(&tabs), ["src/b.rs", "src/c.rs"]);
        assert!(tabs.tabs()[1].preview);

        // Selecting already-pinned (path, target) focuses it without duplicating.
        assert_eq!(
            tabs.open_preview("src/b.rs", SelectedTarget::Uncommitted),
            OpenPreviewOutcome::FocusedPinned(TabKey::working_tree("src/b.rs"))
        );
        assert_eq!(tabs.active_index(), Some(0));
        assert_eq!(paths(&tabs), ["src/b.rs", "src/c.rs"]);

        // Opening the previewed path from Files promotes it to pinned.
        assert_eq!(
            tabs.open_pinned("src/c.rs"),
            OpenPinnedOutcome::Promoted(TabKey::working_tree("src/c.rs"))
        );
        assert!(!tabs.tabs()[1].preview);
        assert_eq!(tabs.preview_index(), None);

        // Different diff targets on the same path do not collide.
        let commit_target = SelectedTarget::Commit("abc1234".into());
        assert_eq!(
            tabs.open_preview("src/b.rs", commit_target.clone()),
            OpenPreviewOutcome::CreatedPreview(TabKey::new("src/b.rs", commit_target))
        );
        assert!(tabs.has_duplicate_path("src/b.rs"));
        assert_eq!(tabs.tabs().len(), 3);
    }

    #[test]
    fn closing_moves_to_the_right_neighbour_and_the_last_close_empties() {
        let mut tabs = FileTabs::default();
        for path in ["a", "b", "c"] {
            tabs.open_pinned(path);
        }
        tabs.activate(1);
        assert_eq!(tabs.close_active().unwrap().path, "b");
        assert_eq!(tabs.active().unwrap().path(), "c");
        assert_eq!(tabs.close_active().unwrap().path, "c");
        assert_eq!(tabs.active().unwrap().path(), "a");
        tabs.open_pinned("d");
        tabs.close(0);
        assert_eq!(tabs.active().unwrap().path(), "d");
        assert_eq!(tabs.close_active().unwrap().path, "d");
        assert!(tabs.is_empty());
        assert_eq!(tabs.active_index(), None);
        assert_eq!(tabs.close_active(), None);
    }

    #[test]
    fn cycling_wraps_both_ways() {
        let mut tabs = FileTabs::default();
        tabs.cycle(false);
        for path in ["a", "b", "c"] {
            tabs.open_pinned(path);
        }
        tabs.cycle(false);
        assert_eq!(tabs.active_index(), Some(0));
        tabs.cycle(true);
        assert_eq!(tabs.active_index(), Some(2));
        tabs.cycle(true);
        assert_eq!(tabs.active_index(), Some(1));
    }

    #[test]
    fn a_tab_names_its_file_and_folder() {
        let tab = FileTab {
            key: TabKey::working_tree("crates/runner-core/src/lib.rs"),
            preview: false,
        };
        assert_eq!(tab.name(), "lib.rs");
        assert_eq!(tab.folder(), "crates/runner-core/src");
        let top = FileTab {
            key: TabKey::working_tree("README.md"),
            preview: false,
        };
        assert_eq!((top.name(), top.folder()), ("README.md", ""));
    }
}
