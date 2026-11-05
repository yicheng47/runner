//! The file viewer's tabs: which files are open and which one is active.
//! Every file opened stays open in a tab of its own (Jason, 2026-11-05).

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileTab {
    /// Relative to the root, `/`-separated.
    pub(crate) path: String,
}

impl FileTab {
    pub(crate) fn name(&self) -> &str {
        self.path
            .rsplit_once('/')
            .map_or(&self.path, |(_, name)| name)
    }

    pub(crate) fn folder(&self) -> &str {
        self.path.rsplit_once('/').map_or("", |(folder, _)| folder)
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

    pub(crate) fn position(&self, path: &str) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.path == path)
    }

    /// Opens `path`: an open file switches to its tab, and a new one gets a
    /// tab beside the active one. Returns whether the file is new and needs
    /// reading.
    pub(crate) fn open(&mut self, path: &str) -> bool {
        if let Some(index) = self.position(path) {
            self.active = index;
            return false;
        }
        let at = if self.tabs.is_empty() {
            0
        } else {
            self.active + 1
        };
        self.tabs.insert(
            at,
            FileTab {
                path: path.to_owned(),
            },
        );
        self.active = at;
        true
    }

    pub(crate) fn activate(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    /// Closes the tab, giving the active place to its right neighbour, or its
    /// left one at the end. Returns the closed file.
    pub(crate) fn close(&mut self, index: usize) -> Option<String> {
        if index >= self.tabs.len() {
            return None;
        }
        let closed = self.tabs.remove(index);
        if index < self.active || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        Some(closed.path)
    }

    #[cfg(test)]
    pub(crate) fn close_active(&mut self) -> Option<String> {
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
        tabs.tabs().iter().map(|tab| tab.path.as_str()).collect()
    }

    #[test]
    fn every_open_keeps_a_tab_and_an_open_file_switches_to_its_own() {
        let mut tabs = FileTabs::default();
        assert!(tabs.open("src/a.rs"));
        assert!(tabs.open("src/b.rs"));
        assert!(tabs.open("c.md"));
        assert_eq!(paths(&tabs), ["src/a.rs", "src/b.rs", "c.md"]);
        assert_eq!(tabs.active().unwrap().path, "c.md");
        assert!(!tabs.open("src/a.rs"), "already open: no read");
        assert_eq!(tabs.active_index(), Some(0));
        assert_eq!(paths(&tabs), ["src/a.rs", "src/b.rs", "c.md"]);
    }

    #[test]
    fn a_new_file_opens_beside_the_active_tab() {
        let mut tabs = FileTabs::default();
        tabs.open("README.md");
        tabs.open("docs/guide.md");
        tabs.activate(0);
        tabs.open("a.md");
        assert_eq!(paths(&tabs), ["README.md", "a.md", "docs/guide.md"]);
        assert_eq!(tabs.active().unwrap().path, "a.md");
    }

    #[test]
    fn closing_moves_to_the_right_neighbour_and_the_last_close_empties() {
        let mut tabs = FileTabs::default();
        for path in ["a", "b", "c"] {
            tabs.open(path);
        }
        tabs.activate(1);
        assert_eq!(tabs.close_active().as_deref(), Some("b"));
        assert_eq!(tabs.active().unwrap().path, "c");
        assert_eq!(tabs.close_active().as_deref(), Some("c"));
        assert_eq!(tabs.active().unwrap().path, "a");
        // Closing a tab left of the active one keeps the active file.
        tabs.open("d");
        tabs.close(0);
        assert_eq!(tabs.active().unwrap().path, "d");
        assert_eq!(tabs.close_active().as_deref(), Some("d"));
        assert!(tabs.is_empty());
        assert_eq!(tabs.active_index(), None);
        assert_eq!(tabs.close_active(), None);
    }

    #[test]
    fn cycling_wraps_both_ways() {
        let mut tabs = FileTabs::default();
        tabs.cycle(false);
        for path in ["a", "b", "c"] {
            tabs.open(path);
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
            path: "crates/runner-core/src/lib.rs".into(),
        };
        assert_eq!(tab.name(), "lib.rs");
        assert_eq!(tab.folder(), "crates/runner-core/src");
        let top = FileTab {
            path: "README.md".into(),
        };
        assert_eq!((top.name(), top.folder()), ("README.md", ""));
    }
}
