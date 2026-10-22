use std::path::PathBuf;

pub(crate) type TabId = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanelBuiltin {
    Info,
    Roles,
    Meta,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TabKind {
    Builtin(PanelBuiltin),
    Files,
    New,
    File(PathBuf),
}

#[derive(Clone, Debug)]
pub(crate) struct PanelTab {
    pub id: TabId,
    pub kind: TabKind,
}

pub(crate) struct PanelTabs {
    pub tabs: Vec<PanelTab>,
    pub active: TabId,
    next_id: TabId,
}

impl PanelTabs {
    pub fn new(builtins: &[PanelBuiltin]) -> Self {
        let mut state = Self {
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
        };
        for builtin in builtins {
            state.open(TabKind::Builtin(*builtin));
        }
        state.open(TabKind::Files);
        state.active = state.tabs[0].id;
        state
    }

    pub fn open(&mut self, kind: TabKind) -> TabId {
        if kind != TabKind::New {
            if let Some(tab) = self.tabs.iter().find(|tab| tab.kind == kind) {
                self.active = tab.id;
                return tab.id;
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(PanelTab { id, kind });
        self.active = id;
        id
    }

    pub fn activate(&mut self, id: TabId) {
        if self.tabs.iter().any(|tab| tab.id == id) {
            self.active = id;
        }
    }

    pub fn close(&mut self, id: TabId) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.open(TabKind::New);
        } else if self.active == id {
            self.active = self.tabs[index.min(self.tabs.len() - 1)].id;
        }
    }

    pub fn selected(&self) -> &PanelTab {
        self.tabs
            .iter()
            .find(|tab| tab.id == self.active)
            .expect("panel always has a selected tab")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_views_remain_available_and_files_are_one_tab() {
        let mut state = PanelTabs::new(&[PanelBuiltin::Roles, PanelBuiltin::Meta]);
        assert_eq!(state.selected().kind, TabKind::Builtin(PanelBuiltin::Roles));
        let files = state.open(TabKind::Files);
        assert_eq!(state.open(TabKind::Files), files);
        assert_eq!(state.tabs.len(), 3);
    }

    #[test]
    fn file_identity_deduplicates_but_same_basename_does_not() {
        let mut state = PanelTabs::new(&[PanelBuiltin::Info]);
        let first = state.open(TabKind::File(PathBuf::from("/project/a/README.md")));
        let second = state.open(TabKind::File(PathBuf::from("/project/b/README.md")));
        assert_ne!(first, second);
        assert_eq!(
            state.open(TabKind::File(PathBuf::from("/project/a/README.md"))),
            first
        );
        assert_eq!(state.active, first);
    }

    #[test]
    fn closing_active_selects_right_neighbour_then_left() {
        let mut state = PanelTabs::new(&[]);
        let first = state.open(TabKind::New);
        let middle = state.open(TabKind::New);
        let last = state.open(TabKind::New);
        state.activate(middle);
        state.close(middle);
        assert_eq!(state.active, last);
        state.close(last);
        assert_eq!(state.active, first);
    }

    #[test]
    fn closing_background_keeps_active_and_last_close_shows_home() {
        let mut state = PanelTabs::new(&[PanelBuiltin::Info]);
        let info = state.active;
        let files = state.open(TabKind::Files);
        state.close(info);
        assert_eq!(state.active, files);
        state.close(files);
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.selected().kind, TabKind::New);
    }

    #[test]
    fn each_context_has_independent_tabs_and_stale_ids_are_noops() {
        let mut chat = PanelTabs::new(&[PanelBuiltin::Info]);
        let team = PanelTabs::new(&[PanelBuiltin::Roles, PanelBuiltin::Meta]);
        chat.open(TabKind::File(PathBuf::from("/chat/notes.md")));
        chat.activate(u64::MAX);
        chat.close(u64::MAX);
        assert!(matches!(chat.selected().kind, TabKind::File(_)));
        assert_eq!(team.selected().kind, TabKind::Builtin(PanelBuiltin::Roles));
    }
}
