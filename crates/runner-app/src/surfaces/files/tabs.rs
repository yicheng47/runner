//! The file viewer's tabs: which files are open, which one is active, and
//! which one is the preview tab the next single click replaces.

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileTab {
    /// Relative to the root, `/`-separated.
    pub(crate) path: String,
    pub(crate) preview: bool,
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

/// What opening a file did, so the owner knows which file state to load or
/// drop.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Opened {
    /// The file had no tab and needs reading.
    pub(crate) new: bool,
    /// The preview tab's file it replaced.
    pub(crate) replaced: Option<String>,
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

    /// Opens `path`: an open file switches to its tab, kept when `keep`; a
    /// new one takes the preview tab's place, or a tab of its own.
    pub(crate) fn open(&mut self, path: &str, keep: bool) -> Opened {
        if let Some(index) = self.position(path) {
            self.active = index;
            if keep {
                self.tabs[index].preview = false;
            }
            return Opened::default();
        }
        let tab = FileTab {
            path: path.to_owned(),
            preview: !keep,
        };
        if let Some(index) = self.tabs.iter().position(|tab| tab.preview) {
            let replaced = std::mem::replace(&mut self.tabs[index], tab);
            self.active = index;
            return Opened {
                new: true,
                replaced: Some(replaced.path),
            };
        }
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        Opened {
            new: true,
            replaced: None,
        }
    }

    /// Opens `path` in a kept tab of its own beside the active one, as a
    /// link does, leaving every other tab, the preview too, as it is; an
    /// open file switches to its tab.
    pub(crate) fn open_beside(&mut self, path: &str) -> Opened {
        if let Some(index) = self.position(path) {
            self.active = index;
            return Opened::default();
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
                preview: false,
            },
        );
        self.active = at;
        Opened {
            new: true,
            replaced: None,
        }
    }

    /// Keeps every tab, the preview too.
    pub(crate) fn keep_all(&mut self) {
        for tab in &mut self.tabs {
            tab.preview = false;
        }
    }

    /// Keeps the tab, as a double-click on it or on its file does.
    pub(crate) fn keep(&mut self, index: usize) {
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.preview = false;
        }
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

    fn paths(tabs: &FileTabs) -> Vec<(&str, bool)> {
        tabs.tabs()
            .iter()
            .map(|tab| (tab.path.as_str(), tab.preview))
            .collect()
    }

    #[test]
    fn a_single_click_replaces_the_preview_and_a_double_click_keeps_it() {
        let mut tabs = FileTabs::default();
        assert_eq!(
            tabs.open("src/a.rs", false),
            Opened {
                new: true,
                replaced: None
            }
        );
        assert_eq!(
            tabs.open("src/b.rs", false),
            Opened {
                new: true,
                replaced: Some("src/a.rs".into())
            }
        );
        assert_eq!(paths(&tabs), [("src/b.rs", true)]);
        // A double-click on the file keeps the preview it opened.
        assert_eq!(tabs.open("src/b.rs", true), Opened::default());
        assert_eq!(paths(&tabs), [("src/b.rs", false)]);
        tabs.open("c.md", false);
        assert_eq!(paths(&tabs), [("src/b.rs", false), ("c.md", true)]);
        // A double-click on the tab keeps it too.
        tabs.keep(1);
        tabs.open("d.toml", false);
        assert_eq!(
            paths(&tabs),
            [("src/b.rs", false), ("c.md", false), ("d.toml", true)]
        );
        assert_eq!(tabs.active().unwrap().path, "d.toml");
    }

    #[test]
    fn an_open_file_switches_to_its_tab_and_a_new_one_takes_the_preview_place() {
        let mut tabs = FileTabs::default();
        tabs.open("a", true);
        tabs.open("b", false);
        assert_eq!(tabs.open("a", false), Opened::default());
        assert_eq!(tabs.active_index(), Some(0));
        assert_eq!(paths(&tabs), [("a", false), ("b", true)]);
        // Enter in the tree opens a kept tab in the preview's place, as a
        // double-click's first click would have.
        assert_eq!(
            tabs.open("c", true),
            Opened {
                new: true,
                replaced: Some("b".into())
            }
        );
        assert_eq!(paths(&tabs), [("a", false), ("c", false)]);
        assert_eq!(tabs.active_index(), Some(1));
        tabs.open("d", true);
        assert_eq!(paths(&tabs), [("a", false), ("c", false), ("d", false)]);
    }

    #[test]
    fn closing_moves_to_the_right_neighbour_and_the_last_close_empties() {
        let mut tabs = FileTabs::default();
        for path in ["a", "b", "c"] {
            tabs.open(path, true);
        }
        tabs.activate(1);
        assert_eq!(tabs.close_active().as_deref(), Some("b"));
        assert_eq!(tabs.active().unwrap().path, "c");
        assert_eq!(tabs.close_active().as_deref(), Some("c"));
        assert_eq!(tabs.active().unwrap().path, "a");
        // Closing a tab left of the active one keeps the active file.
        tabs.open("d", true);
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
            tabs.open(path, true);
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
            preview: false,
        };
        assert_eq!(tab.name(), "lib.rs");
        assert_eq!(tab.folder(), "crates/runner-core/src");
        let top = FileTab {
            path: "README.md".into(),
            preview: true,
        };
        assert_eq!((top.name(), top.folder()), ("README.md", ""));
    }

    #[test]
    fn a_link_opens_beside_the_active_tab_and_never_replaces_the_preview() {
        let mut tabs = FileTabs::default();
        tabs.open("README.md", false);
        let opened = tabs.open_beside("docs/guide.md");
        assert_eq!(
            opened,
            Opened {
                new: true,
                replaced: None
            }
        );
        assert_eq!(
            paths(&tabs),
            [("README.md", true), ("docs/guide.md", false)]
        );
        assert_eq!(tabs.active().unwrap().path, "docs/guide.md");
        tabs.activate(0);
        tabs.open_beside("a.md");
        assert_eq!(
            paths(&tabs),
            [
                ("README.md", true),
                ("a.md", false),
                ("docs/guide.md", false)
            ]
        );
        assert_eq!(tabs.open_beside("README.md"), Opened::default());
        assert_eq!(tabs.active_index(), Some(0));
        assert_eq!(
            paths(&tabs)[0],
            ("README.md", true),
            "following a link keeps nothing"
        );
    }
}
