mod state;
#[cfg(test)]
mod tests;
mod view;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
#[cfg(not(test))]
use std::time::Duration;

use crate::file_preview::FilePreviewView;
use gpui::prelude::*;
use gpui::{
    App, Context, Entity, EventEmitter, ScrollHandle, Subscription, UniformListScrollHandle,
};
use runner_app::workspace_files::{self, DirectoryEntry, FilePreview};
pub(crate) use state::PanelBuiltin;
use state::{PanelTabs, TabId, TabKind};

#[derive(Clone, Debug)]
pub(crate) enum PanelEvent {
    Collapse,
    Width(f32),
    Terminal,
}

pub(crate) struct HostedPanel {
    pub panel: Entity<WorkspacePanel>,
    pub _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
struct DirectoryState {
    entries: Vec<DirectoryEntry>,
    error: Option<String>,
    loading: bool,
    loaded: bool,
    request: u64,
}

#[derive(Default)]
struct PreviewState {
    view: Option<Entity<FilePreviewView>>,
    snapshot: Option<FilePreview>,
    modified: Option<std::time::SystemTime>,
    error: Option<String>,
    loading: bool,
    request: u64,
}

pub(crate) struct WorkspacePanel {
    tabs: PanelTabs,
    builtins: Vec<PanelBuiltin>,
    root: Option<PathBuf>,
    generation: u64,
    navigation_revision: u64,
    directories: HashMap<PathBuf, DirectoryState>,
    expanded: HashSet<PathBuf>,
    previews: HashMap<TabId, PreviewState>,
    tree_scroll: UniformListScrollHandle,
    tabs_scroll: ScrollHandle,
    revealed_tab: Option<(TabId, usize, u32)>,
    visible: bool,
    normal_width: Option<f32>,
    terminal_available: bool,
    opening: HashSet<PathBuf>,
    notice: Option<String>,
    selected_file: Option<PathBuf>,
}

impl EventEmitter<PanelEvent> for WorkspacePanel {}

impl WorkspacePanel {
    pub fn new(builtins: &[PanelBuiltin], cx: &mut Context<Self>) -> Self {
        #[cfg(test)]
        let _ = cx;
        #[cfg(not(test))]
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            if this
                .update(cx, |this, cx| {
                    if this.visible {
                        this.refresh(false, cx);
                    }
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
        Self {
            tabs: PanelTabs::new(builtins),
            builtins: builtins.to_vec(),
            root: None,
            generation: 0,
            navigation_revision: 0,
            directories: HashMap::new(),
            expanded: HashSet::new(),
            previews: HashMap::new(),
            tree_scroll: UniformListScrollHandle::new(),
            tabs_scroll: ScrollHandle::new(),
            revealed_tab: None,
            visible: false,
            normal_width: None,
            terminal_available: false,
            opening: HashSet::new(),
            notice: None,
            selected_file: None,
        }
    }

    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.root == root {
            return;
        }
        self.root = root;
        self.generation += 1;
        self.directories.clear();
        self.expanded.clear();
        self.previews.clear();
        self.opening.clear();
        self.tabs = PanelTabs::new(&self.builtins);
        self.revealed_tab = None;
        self.tree_scroll = UniformListScrollHandle::new();
        self.notice = None;
        self.selected_file = None;
        cx.notify();
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub fn set_terminal_available(&mut self, available: bool, cx: &mut Context<Self>) {
        if self.terminal_available != available {
            self.terminal_available = available;
            cx.notify();
        }
    }

    pub fn active_builtin(&self) -> Option<PanelBuiltin> {
        match self.tabs.selected().kind {
            TabKind::Builtin(kind) => Some(kind),
            _ => None,
        }
    }

    pub fn activate_builtin(&mut self, kind: PanelBuiltin, cx: &mut Context<Self>) {
        self.tabs.open(TabKind::Builtin(kind));
        cx.notify();
    }

    fn select(&mut self, id: TabId, cx: &mut Context<Self>) {
        self.navigation_revision += 1;
        self.tabs.activate(id);
        if let TabKind::File(path) = &self.tabs.selected().kind {
            self.selected_file = Some(path.clone());
        }
        self.notice = None;
        self.refresh(false, cx);
        cx.notify();
    }

    fn close(&mut self, id: TabId, cx: &mut Context<Self>) {
        self.navigation_revision += 1;
        self.tabs.close(id);
        self.previews.remove(&id);
        cx.notify();
    }

    fn show_files(&mut self, cx: &mut Context<Self>) {
        self.navigation_revision += 1;
        self.tabs.open(TabKind::Files);
        self.refresh(false, cx);
        cx.notify();
    }

    fn toggle_directory(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.expanded.remove(&path) {
            self.expanded.insert(path.clone());
            self.load_directory(path, true, cx);
        }
        cx.notify();
    }

    fn load_directory(&mut self, path: PathBuf, announce: bool, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let state = self.directories.entry(path.clone()).or_default();
        if state.loading {
            return;
        }
        state.loading = true;
        state.request += 1;
        let request = state.request;
        let generation = self.generation;
        let target = path.clone();
        let task =
            cx.background_spawn(async move { workspace_files::read_directory(&root, &target) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let Some(state) = this.directories.get_mut(&path) else {
                    return;
                };
                if state.request != request {
                    return;
                }
                let previous_error = state.error.clone();
                state.loading = false;
                let changed = match result {
                    Ok(entries) => {
                        let changed =
                            !state.loaded || state.entries != entries || state.error.is_some();
                        state.entries = entries;
                        state.error = None;
                        state.loaded = true;
                        changed
                    }
                    Err(error) => {
                        state.entries.clear();
                        state.error = Some(error);
                        state.loaded = true;
                        state.error != previous_error
                    }
                };
                if announce || changed {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn open_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if !self.opening.insert(path.clone()) {
            return;
        }
        self.notice = None;
        self.navigation_revision += 1;
        let navigation_revision = self.navigation_revision;
        let generation = self.generation;
        let target = path.clone();
        let task =
            cx.background_spawn(async move { workspace_files::resolve_path(&root, &target) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.opening.remove(&path);
                if this.navigation_revision != navigation_revision {
                    return;
                }
                match result {
                    Ok(path) => {
                        this.selected_file = Some(path.clone());
                        let id = this.tabs.open(TabKind::File(path.clone()));
                        this.load_preview(id, path, true, cx);
                    }
                    Err(error) => this.notice = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn load_preview(&mut self, id: TabId, path: PathBuf, announce: bool, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let state = self.previews.entry(id).or_default();
        if state.loading {
            return;
        }
        state.loading = true;
        state.request += 1;
        let request = state.request;
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            let result = workspace_files::read_preview(&root, &path);
            let modified = std::fs::metadata(&path)
                .ok()
                .and_then(|meta| meta.modified().ok());
            (result, modified)
        });
        cx.spawn(async move |this, cx| {
            let (result, modified) = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let Some(state) = this.previews.get_mut(&id) else {
                    return;
                };
                if state.request != request {
                    return;
                }
                state.loading = false;
                match result {
                    Ok(preview) => {
                        let changed = state.error.is_some()
                            || state.modified != modified
                            || state
                                .snapshot
                                .as_ref()
                                .is_none_or(|previous| !same_preview(previous, &preview));
                        state.error = None;
                        state.modified = modified;
                        if changed || announce {
                            if let Some(view) = &state.view {
                                view.update(cx, |view, cx| view.replace(preview.clone(), cx));
                            } else {
                                let root = this.root.clone().expect("preview has a workspace");
                                state.view = Some(
                                    cx.new(|cx| FilePreviewView::new(root, preview.clone(), cx)),
                                );
                            }
                            state.snapshot = Some(preview);
                            cx.notify();
                        }
                    }
                    Err(error) => {
                        let changed = state.error.as_ref() != Some(&error);
                        state.error = Some(error);
                        if announce || changed {
                            cx.notify();
                        }
                    }
                }
            });
        })
        .detach();
    }

    fn refresh(&mut self, announce: bool, cx: &mut Context<Self>) {
        match self.tabs.selected().kind.clone() {
            TabKind::Files => {
                let paths = self.visible_directories();
                for path in paths {
                    self.load_directory(path, announce, cx);
                }
            }
            TabKind::File(path) => self.load_preview(self.tabs.active, path, announce, cx),
            _ => {}
        }
    }

    fn visible_directories(&self) -> Vec<PathBuf> {
        let mut paths = self.root.iter().cloned().collect::<Vec<_>>();
        paths.extend(
            self.rows()
                .into_iter()
                .filter(|row| row.entry.is_dir && self.expanded.contains(&row.entry.path))
                .map(|row| row.entry.path),
        );
        paths.sort();
        paths.dedup();
        paths
    }

    fn rows(&self) -> Vec<TreeRow> {
        let Some(root) = &self.root else {
            return Vec::new();
        };
        let Some(state) = self.directories.get(root) else {
            return Vec::new();
        };
        let mut output = Vec::new();
        let mut pending = state
            .entries
            .iter()
            .rev()
            .cloned()
            .map(|entry| TreeRow { entry, depth: 0 })
            .collect::<Vec<_>>();
        let mut expanded_once = HashSet::new();
        while let Some(row) = pending.pop() {
            let path = row.entry.path.clone();
            let depth = row.depth;
            let expand = row.entry.is_dir
                && self.expanded.contains(&path)
                && depth < 64
                && expanded_once.insert(path.clone());
            output.push(row);
            if expand {
                if let Some(state) = self.directories.get(&path) {
                    pending.extend(state.entries.iter().rev().cloned().map(|entry| TreeRow {
                        entry,
                        depth: depth + 1,
                    }));
                }
            }
        }
        output
    }
}

#[derive(Clone)]
struct TreeRow {
    entry: DirectoryEntry,
    depth: usize,
}

fn same_preview(left: &FilePreview, right: &FilePreview) -> bool {
    use workspace_files::PreviewContent;
    left.path == right.path
        && left.bytes == right.bytes
        && match (&left.content, &right.content) {
            (PreviewContent::Markdown(a), PreviewContent::Markdown(b))
            | (PreviewContent::Text(a), PreviewContent::Text(b))
            | (PreviewContent::Unsupported(a), PreviewContent::Unsupported(b)) => a == b,
            (PreviewContent::Image, PreviewContent::Image) => true,
            _ => false,
        }
}

pub(crate) fn workspace_root(
    cwd: Option<&str>,
    project: Option<&str>,
    default: &str,
) -> Option<PathBuf> {
    let path = [cwd, project, Some(default)]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())?;
    if path == "~" {
        return runner_core::app_paths::home_dir();
    }
    if let Some(suffix) = path.strip_prefix("~/") {
        return runner_core::app_paths::home_dir().map(|home| home.join(suffix));
    }
    Some(Path::new(path).to_path_buf())
}

pub(crate) fn bounded_panel_width(requested: f32, host_width: f32) -> f32 {
    requested.max(0.).min((host_width - 240.).max(0.))
}
