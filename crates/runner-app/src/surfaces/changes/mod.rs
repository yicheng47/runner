//! The side panel's Changes view (#1141): changed files, branch commits since
//! merge base, Unified diff with two-version syntax highlighting, and viewed marks.

pub(crate) mod diff;
pub(crate) mod model;
mod render;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    App, Context, Entity, EntityId, FocusHandle, KeyDownEvent, PathPromptOptions, ScrollHandle,
    ScrollStrategy, SharedString, Subscription, Task, UniformListScrollHandle, Window,
};
use runner_app::ui::{Scrollbar, TextField};
use runner_core::protocol::changes::{
    ChangedFile, ChangesCommits, ChangesDiff, ChangesFiles, ChangesTotals, ChangesViewed, DiffBody,
    DiffCounts, DiffHunk, ModeChange,
};
use runner_core::protocol::files::{FileStatus, FilesRoot, FilesRootOutcome, FilesSubject};
use runner_core::protocol::{ClientError, DaemonClient};

use crate::app_settings::{
    DiffViewMode, FileLinkEditor, TERMINAL_FONT_SIZE_MAX, TERMINAL_FONT_SIZE_MIN,
};
#[cfg(not(test))]
use crate::file_links::{self, FileLinkTarget};
use crate::surfaces::files::document::Document;
use crate::surfaces::files::highlight;
use crate::surfaces::files::{self, absolute, picker, FilesEvent, FilesHub, Look};
use crate::surfaces::side_panel::{self, PanelContext, PanelSurface, Widened};
use crate::theme;

pub(crate) use diff::UnifiedDiffView;
use model::{
    build_commit_rows, build_flat_rows, build_tree_rows, choose_initial_target, has_all_changes,
    is_crlf_only_change, reconcile_file_selection, reconcile_target, CommitRow, FocusPlace,
    ListRow, SelectedTarget,
};
pub(crate) use render::{
    render_shared_diff_header, render_shared_diff_placeholder, DiffHeaderProps, DiffModeCallback,
};

fn subject(context: &PanelContext) -> FilesSubject {
    match &context.subject {
        crate::surfaces::side_panel::PanelSubject::Session(id) => FilesSubject::Session(id.clone()),
        crate::surfaces::side_panel::PanelSubject::Mission(id) => FilesSubject::Mission(id.clone()),
    }
}

pub(super) enum Body {
    Loading,
    Remote,
    Choose,
    Missing { path: PathBuf },
    NotInGit,
    Root(Box<RootChanges>),
    Failed(SharedString),
}

pub(super) struct RootChanges {
    pub(super) root: FilesRoot,
    pub(super) commits: Option<ChangesCommits>,
    pub(super) selected_target: SelectedTarget,
    pub(super) initial_target_chosen: bool,
    pub(super) uncommitted_count: usize,
    pub(super) branch_count: usize,
    pub(super) files: Vec<ChangedFile>,
    pub(super) totals: ChangesTotals,
    pub(super) collapsed_folders: BTreeSet<String>,
    pub(super) list_rows: Vec<ListRow>,
    pub(super) selected_path: Option<String>,
    pub(super) highlighted_list_row: usize,
    pub(super) diff_state: ActiveDiff,
    pub(super) in_flight_diff: Option<(u64, SelectedTarget, String)>,
    pub(super) snapshot_seq: u64,
    pub(super) diff_seq: u64,
}

pub(super) enum ActiveDiff {
    Empty,
    Loading,
    Loaded(Box<LoadedDiff>),
    Failed(SharedString),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DiffVersion {
    pub(super) target: SelectedTarget,
    pub(super) path: String,
    pub(super) old_blob_id: Option<String>,
    pub(super) new_blob_id: Option<String>,
}

pub(super) struct LoadedDiff {
    pub(super) version: DiffVersion,
    pub(super) path: String,
    pub(super) status: FileStatus,
    pub(super) counts: DiffCounts,
    pub(super) mode_change: Option<ModeChange>,
    pub(super) conflicted: bool,
    pub(super) viewed: bool,
    pub(super) changed_since_viewed: bool,
    pub(super) kind: LoadedDiffKind,
}

impl LoadedDiff {
    pub(super) fn matches_file(&self, target: &SelectedTarget, file: &ChangedFile) -> bool {
        &self.version.target == target
            && self.version.path == file.path
            && self.version.old_blob_id == file.old_blob_id
            && self.version.new_blob_id == file.new_blob_id
    }

    pub(super) fn set_viewed_state(
        &mut self,
        viewed: bool,
        changed_since_viewed: bool,
        cx: &mut App,
    ) {
        self.viewed = viewed;
        self.changed_since_viewed = changed_since_viewed;
        if let LoadedDiffKind::Unified(entity) = &self.kind {
            entity.update(cx, |v, cx| {
                v.set_viewed_state(viewed, changed_since_viewed, cx);
            });
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RowIdentity {
    Folder(String),
    File(String),
}

fn row_identity(row: &ListRow) -> RowIdentity {
    match row {
        ListRow::Folder { path, .. } => RowIdentity::Folder(path.clone()),
        ListRow::File { path, .. } => RowIdentity::File(path.clone()),
    }
}

fn find_nearest_visible_ancestor_folder(path: &str, rows: &[ListRow]) -> Option<usize> {
    let mut current = path;
    while let Some((parent, _)) = current.rsplit_once('/') {
        if let Some(idx) = rows
            .iter()
            .position(|r| matches!(r, ListRow::Folder { path: p, .. } if p == parent))
        {
            return Some(idx);
        }
        current = parent;
    }
    None
}

fn resolve_highlighted_row(
    wanted: Option<&RowIdentity>,
    selected_path: Option<&str>,
    fallback_idx: usize,
    rows: &[ListRow],
) -> usize {
    if rows.is_empty() {
        return 0;
    }
    if let Some(identity) = wanted {
        match identity {
            RowIdentity::Folder(folder) => {
                if let Some(idx) = rows
                    .iter()
                    .position(|r| matches!(r, ListRow::Folder { path, .. } if path == folder))
                {
                    return idx;
                }
                if let Some(idx) = find_nearest_visible_ancestor_folder(folder, rows) {
                    return idx;
                }
                let prefix = format!("{folder}/");
                if let Some(idx) = rows.iter().position(
                    |r| matches!(r, ListRow::File { path, .. } if path.starts_with(&prefix)),
                ) {
                    return idx;
                }
            }
            RowIdentity::File(file) => {
                if let Some(idx) = rows
                    .iter()
                    .position(|r| matches!(r, ListRow::File { path, .. } if path == file))
                {
                    return idx;
                }
                if let Some(idx) = find_nearest_visible_ancestor_folder(file, rows) {
                    return idx;
                }
            }
        }
    }
    if let Some(selected) = selected_path {
        if let Some(idx) = rows
            .iter()
            .position(|r| matches!(r, ListRow::File { path, .. } if path == selected))
        {
            return idx;
        }
        if let Some(idx) = find_nearest_visible_ancestor_folder(selected, rows) {
            return idx;
        }
    }
    fallback_idx.min(rows.len().saturating_sub(1))
}

pub(crate) enum LoadedDiffKind {
    Unified(Entity<UnifiedDiffView>),
    CrlfOnly,
    Binary {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    TooLarge {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    ModeOnly {
        old_mode: String,
        new_mode: String,
    },
    Submodule {
        old_commit: Option<String>,
        new_commit: Option<String>,
    },
}

pub(crate) enum PreparedDiffBody {
    Unified {
        old_doc: Arc<Document>,
        new_doc: Arc<Document>,
        hunks: Vec<DiffHunk>,
    },
    CrlfOnly,
    Binary {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    TooLarge {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    ModeOnly {
        old_mode: String,
        new_mode: String,
    },
    Submodule {
        old_commit: Option<String>,
        new_commit: Option<String>,
    },
}

pub(crate) fn prepare_diff_body(diff: &ChangesDiff) -> PreparedDiffBody {
    match &diff.body {
        DiffBody::Text { old, new, hunks } => {
            if is_crlf_only_change(&old.0, &new.0, diff.mode_change.as_ref(), diff.conflicted) {
                PreparedDiffBody::CrlfOnly
            } else {
                let old_doc = Arc::new(Document::decode(&old.0));
                let new_doc = Arc::new(Document::decode(&new.0));
                PreparedDiffBody::Unified {
                    old_doc,
                    new_doc,
                    hunks: hunks.clone(),
                }
            }
        }
        DiffBody::Binary { old_size, new_size } => PreparedDiffBody::Binary {
            old_size: *old_size,
            new_size: *new_size,
        },
        DiffBody::TooLarge { old_size, new_size } => PreparedDiffBody::TooLarge {
            old_size: *old_size,
            new_size: *new_size,
        },
        DiffBody::ModeOnly { old_mode, new_mode } => PreparedDiffBody::ModeOnly {
            old_mode: old_mode.clone(),
            new_mode: new_mode.clone(),
        },
        DiffBody::Submodule {
            old_commit,
            new_commit,
        } => PreparedDiffBody::Submodule {
            old_commit: old_commit.clone(),
            new_commit: new_commit.clone(),
        },
    }
}

struct SnapshotPayload {
    commits: ChangesCommits,
    uncommitted: ChangesFiles,
    branch: Option<ChangesFiles>,
    commit_files: Option<(String, ChangesFiles)>,
}

pub(crate) struct ChangesView {
    pub(super) context: PanelContext,
    client: DaemonClient,
    pub(super) root_focus: FocusHandle,
    pub(super) list_focus: FocusHandle,
    pub(super) diff_focus: FocusHandle,
    pub(super) focus_place: FocusPlace,
    pub(super) _focus_lost: Option<Subscription>,
    pub(super) body: Body,
    generation: u64,
    root_change: u64,
    pub(super) shown: bool,
    watched: Option<PathBuf>,
    watch_chain: Option<Task<()>>,
    root_chain: Option<Task<()>>,
    pub(super) error: Option<SharedString>,
    pub(super) recent: Vec<PathBuf>,
    pub(super) choose_field: Entity<TextField>,
    pub(super) commits_scroll: ScrollHandle,
    pub(super) files_scroll: UniformListScrollHandle,
    pub(super) files_scrollbar: Entity<Scrollbar>,
    pub(super) widened: Option<Widened>,
    pub(super) maximized: bool,
    pub(super) on_request_maximize: Option<side_panel::PanelCallback>,
    pub(super) owner: Option<EntityId>,
    pub(super) reported_wide: Option<bool>,
    pub(super) tree_resizing: bool,
    look: Option<Look>,
    tree_view_pref: bool,
    diff_mode_pref: DiffViewMode,
    #[cfg(test)]
    pub(super) launched: Vec<(Option<FileLinkEditor>, PathBuf)>,
    #[cfg(test)]
    hold_diffs: bool,
    #[cfg(test)]
    held_diffs: Vec<futures::channel::oneshot::Sender<()>>,
    #[cfg(test)]
    hold_snapshots: bool,
    #[cfg(test)]
    held_snapshots: Vec<futures::channel::oneshot::Sender<()>>,
    _hub: Subscription,
    _settings: Subscription,
}

impl ChangesView {
    pub(crate) fn new(context: PanelContext, cx: &mut Context<Self>) -> Self {
        let client = crate::app_store::global_app_store(cx)
            .read(cx)
            .client
            .clone();
        let hub = files::hub(&client, cx);
        let hub_subscription = cx.subscribe(&hub, |this, _: Entity<FilesHub>, event, cx| {
            this.on_files_event(event, cx)
        });
        let store = crate::app_store::global_app_store(cx);
        let settings_subscription = cx.observe(&store, |this, _, cx| this.settings_changed(cx));
        let root_weak = cx.weak_entity();
        let choose_field = cx.new(move |field_cx| {
            TextField::new(
                field_cx.focus_handle(),
                "",
                crate::i18n::message("ui.type_a_path"),
                true,
            )
            .text_size(theme::text_ui())
            .right_padding(32.)
            .key_interceptor(Rc::new(move |event, window, cx| {
                if event.keystroke.key != "enter" {
                    return false;
                }
                let root_weak = root_weak.clone();
                window.defer(cx, move |window, cx| {
                    let _ = root_weak.update(cx, |this, cx| this.apply_typed_path(window, cx));
                });
                true
            }))
        });
        let files_scroll = UniformListScrollHandle::new();
        let entity_id = cx.entity_id();
        let base = files_scroll.0.borrow().base_handle.clone();
        let files_scrollbar = cx.new(|_| Scrollbar::app(base, entity_id));
        let tree_view_pref = files::settings(cx).is_some_and(|s| s.changes_tree_view);
        let diff_mode_pref =
            files::settings(cx).map_or(DiffViewMode::Unified, |s| s.changes_diff_mode);

        let mut view = Self {
            body: if context.remote {
                Body::Remote
            } else {
                Body::Loading
            },
            context,
            client,
            root_focus: cx.focus_handle(),
            list_focus: cx.focus_handle(),
            diff_focus: cx.focus_handle(),
            focus_place: FocusPlace::Files,
            _focus_lost: None,
            generation: 0,
            root_change: 0,
            shown: false,
            watched: None,
            watch_chain: None,
            root_chain: None,
            error: None,
            recent: Vec::new(),
            choose_field,
            commits_scroll: ScrollHandle::new(),
            files_scroll,
            files_scrollbar,
            widened: None,
            maximized: false,
            on_request_maximize: None,
            owner: None,
            reported_wide: None,
            tree_resizing: false,
            look: Look::current(cx),
            tree_view_pref,
            diff_mode_pref,
            #[cfg(test)]
            launched: Vec::new(),
            #[cfg(test)]
            hold_diffs: false,
            #[cfg(test)]
            held_diffs: Vec::new(),
            #[cfg(test)]
            hold_snapshots: false,
            #[cfg(test)]
            held_snapshots: Vec::new(),
            _hub: hub_subscription,
            _settings: settings_subscription,
        };
        view.resolve(cx);
        cx.on_release(|this: &mut Self, cx| {
            if let Some(chain) = this.root_chain.take() {
                chain.detach();
            }
            let previous = this.watch_chain.take();
            let root = this.watched.take();
            let client = this.client.clone();
            cx.background_spawn(async move {
                if let Some(previous) = previous {
                    previous.await;
                }
                if let Some(root) = root {
                    let _ = client.files_unwatch(&root);
                }
            })
            .detach();
        })
        .detach();
        view
    }

    pub(crate) fn set_context(&mut self, context: PanelContext, cx: &mut Context<Self>) {
        if self.context == context {
            return;
        }
        let moved =
            self.context.remote != context.remote || self.context.project != context.project;
        self.context = context;
        if moved {
            self.resolve(cx);
        }
        cx.notify();
    }

    pub(crate) fn set_widened(&mut self, widened: Option<Widened>, cx: &mut Context<Self>) {
        if self.widened != widened {
            self.widened = widened;
            self.sync_effective_diff_mode(cx);
            cx.notify();
        }
    }

    pub(crate) fn set_maximized(&mut self, maximized: bool, cx: &mut Context<Self>) {
        if self.maximized != maximized {
            self.maximized = maximized;
            self.sync_effective_diff_mode(cx);
            cx.notify();
        }
    }

    pub(crate) fn set_on_request_maximize(&mut self, handler: Option<side_panel::PanelCallback>) {
        self.on_request_maximize = handler;
    }

    pub(crate) fn focus_default(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.root_focus.contains_focused(window, cx) {
            return;
        }
        if matches!(self.body, Body::Root(_)) {
            self.focus_list(window, cx);
        } else {
            self.root_focus.focus(window, cx);
        }
    }

    pub(super) fn diff_width(&self) -> f32 {
        self.widened
            .map_or(crate::app_settings::FILE_VIEWER_DEFAULT, |w| w.file)
    }

    pub(super) fn effective_diff_mode(&self, cx: &App) -> DiffViewMode {
        let preferred = files::settings(cx).map_or(self.diff_mode_pref, |s| s.changes_diff_mode);
        side_panel::effective_diff_mode(preferred, self.diff_width(), self.maximized)
    }

    fn sync_effective_diff_mode(&self, cx: &mut Context<Self>) {
        let mode = self.effective_diff_mode(cx);
        if let Body::Root(state) = &self.body
            && let ActiveDiff::Loaded(loaded) = &state.diff_state
            && let LoadedDiffKind::Unified(entity) = &loaded.kind
        {
            entity.update(cx, |v, cx| v.set_mode(mode, cx));
        }
    }

    pub(super) fn select_diff_mode(
        &mut self,
        mode: DiffViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.diff_mode_pref = mode;
        crate::app_store::global_app_store(cx).update(cx, |store, cx| {
            store.update_settings(
                |settings| {
                    let changed = settings.changes_diff_mode != mode;
                    settings.changes_diff_mode = mode;
                    changed
                },
                true,
                cx,
            );
        });
        if !self.root_focus.contains_focused(window, cx) {
            self.focus_default(window, cx);
        }
        if mode == DiffViewMode::Split
            && !self.maximized
            && self.diff_width() < side_panel::SPLIT_MIN_DIFF_WIDTH
            && let Some(request) = self.on_request_maximize.clone()
        {
            self.maximized = true;
            request(window, cx);
        }
        self.sync_effective_diff_mode(cx);
        cx.notify();
    }

    pub(crate) fn set_owner(&mut self, owner: Option<EntityId>) {
        self.owner = owner;
    }

    pub(crate) fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown == shown {
            return;
        }
        self.shown = shown;
        self.sync_watch(cx);
    }

    /// `Some(true)` once a git root is active; `None` while loading so the
    /// panel can fall back to Files' known root width without flickering.
    pub(crate) fn widening(&self) -> Option<bool> {
        match &self.body {
            Body::Loading => None,
            Body::Root(_) => Some(true),
            _ => Some(false),
        }
    }

    fn watched_git_root(&self) -> Option<&Path> {
        match &self.body {
            Body::Root(state) => Some(&state.root.path),
            _ => None,
        }
    }

    pub(super) fn noun(&self) -> String {
        crate::i18n::text(match self.context.surface {
            PanelSurface::Chat => "noun.chat",
            PanelSurface::Terminal => "noun.terminal",
            PanelSurface::Mission => "noun.mission",
        })
        .to_owned()
    }

    pub(super) fn projects(&self, cx: &App) -> Vec<(String, PathBuf)> {
        crate::app_store::global_app_store(cx)
            .read(cx)
            .projects
            .iter()
            .filter(|project| !project.cwd.trim().is_empty())
            .map(|project| (project.name.clone(), PathBuf::from(&project.cwd)))
            .collect()
    }

    pub(super) fn own_project(&self, cx: &App) -> Option<(String, PathBuf)> {
        let id = self.context.project.as_deref()?;
        crate::app_store::global_app_store(cx)
            .read(cx)
            .projects
            .iter()
            .find(|project| project.id == id && !project.cwd.trim().is_empty())
            .map(|project| (project.name.clone(), PathBuf::from(&project.cwd)))
    }

    pub(super) fn home(cx: &App) -> Option<PathBuf> {
        crate::app_store::global_app_store(cx)
            .read(cx)
            .home_dir
            .clone()
    }

    fn request<T: Send + 'static>(
        &self,
        cx: &mut Context<Self>,
        call: impl FnOnce(&DaemonClient) -> Result<T, ClientError> + Send + 'static,
        then: impl FnOnce(&mut Self, Result<T, ClientError>, &mut Context<Self>) + 'static,
    ) {
        let client = self.client.clone();
        let generation = self.generation;
        let task = cx.background_spawn(async move { call(&client) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    then(this, result, cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn resolve(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        if self.context.remote {
            self.body = Body::Remote;
            self.sync_watch(cx);
            cx.notify();
            return;
        }
        let subject = subject(&self.context);
        self.request(
            cx,
            move |client| client.files_root(&subject),
            |this, result, cx| this.apply_outcome(result, cx),
        );
    }

    pub(super) fn set_root(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.context.remote {
            return;
        }
        self.root_change += 1;
        let change = self.root_change;
        let subject = subject(&self.context);
        let client = self.client.clone();
        let previous = self.root_chain.take();
        let (reply, result) = futures::channel::oneshot::channel();
        self.root_chain = Some(cx.background_spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            let _ = reply.send(client.files_set_root(&subject, path.as_deref()));
        }));
        cx.spawn(async move |this, cx| {
            let Ok(result) = result.await else { return };
            let _ = this.update(cx, |this, cx| {
                if this.root_change != change {
                    return;
                }
                match result {
                    Ok(outcome) => {
                        let panel_subject = this.context.subject.clone();
                        let origin = cx.entity_id();
                        let client = this.client.clone();
                        this.apply_outcome(Ok(outcome), cx);
                        files::hub(&client, cx).update(cx, |_, cx| {
                            cx.emit(FilesEvent::RootChanged {
                                subject: panel_subject,
                                origin,
                            });
                        });
                    }
                    Err(error) => {
                        this.error = Some(error.to_string().into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_outcome(
        &mut self,
        outcome: Result<FilesRootOutcome, ClientError>,
        cx: &mut Context<Self>,
    ) {
        let same = matches!(
            (&outcome, &self.body),
            (Ok(FilesRootOutcome::Root(root)), Body::Root(state))
                if root.path == state.root.path && root.git.is_some()
        );
        if !same {
            self.generation += 1;
        }
        self.error = None;
        match outcome {
            Ok(FilesRootOutcome::Root(root)) => {
                if root.git.is_none() {
                    self.body = Body::NotInGit;
                } else if let Body::Root(state) = &mut self.body
                    && same
                {
                    state.root = root;
                    self.load_snapshot(cx);
                } else {
                    self.files_scroll = UniformListScrollHandle::new();
                    let entity_id = cx.entity_id();
                    let base = self.files_scroll.0.borrow().base_handle.clone();
                    self.files_scrollbar = cx.new(|_| Scrollbar::app(base, entity_id));
                    self.commits_scroll = ScrollHandle::new();
                    self.body = Body::Root(Box::new(RootChanges {
                        root,
                        commits: None,
                        selected_target: SelectedTarget::Uncommitted,
                        initial_target_chosen: false,
                        uncommitted_count: 0,
                        branch_count: 0,
                        files: Vec::new(),
                        totals: ChangesTotals::default(),
                        collapsed_folders: BTreeSet::new(),
                        list_rows: Vec::new(),
                        selected_path: None,
                        highlighted_list_row: 0,
                        diff_state: ActiveDiff::Empty,
                        in_flight_diff: None,
                        snapshot_seq: 0,
                        diff_seq: 0,
                    }));
                    self.load_snapshot(cx);
                }
            }
            Ok(FilesRootOutcome::ChooseFolder) => {
                self.body = Body::Choose;
                self.load_recent(cx);
            }
            Ok(FilesRootOutcome::Missing { path }) => {
                self.body = Body::Missing { path };
            }
            Err(error) => {
                self.body = Body::Failed(error.to_string().into());
            }
        }
        self.watched_root_changed(cx);
        cx.notify();
    }

    pub(super) fn show_choose_screen(&mut self, cx: &mut Context<Self>) {
        self.body = Body::Choose;
        self.load_recent(cx);
        self.watched_root_changed(cx);
        cx.notify();
    }

    fn load_recent(&mut self, cx: &mut Context<Self>) {
        self.request(
            cx,
            |client| client.files_recent_folders(),
            |this, result, cx| {
                if let Ok(recent) = result {
                    this.recent = recent;
                    cx.notify();
                }
            },
        );
    }

    pub(super) fn apply_typed_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.choose_field.read(cx).text().to_owned();
        match picker::typed_path(&text, Self::home(cx).as_deref()) {
            Some(path) => {
                self.set_root(Some(path), cx);
                self.list_focus.focus(window, cx);
            }
            None => {
                self.error = Some(crate::i18n::text("ui.type_a_full_path").into());
                cx.notify();
            }
        }
    }

    pub(super) fn browse(&mut self, cx: &mut Context<Self>) {
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(crate::i18n::text("ui.choose_folder").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = selected.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.set_root(Some(path), cx));
        })
        .detach();
    }

    // ── Watch & live events ────────────────────────────────────────────

    fn watched_root_changed(&mut self, cx: &mut Context<Self>) {
        if self.watched.as_deref() != self.watched_git_root() {
            self.unwatch(cx);
        }
        self.sync_watch(cx);
    }

    fn chain_watch(
        &mut self,
        cx: &mut Context<Self>,
        call: impl FnOnce(&DaemonClient) + Send + 'static,
    ) {
        let previous = self.watch_chain.take();
        let client = self.client.clone();
        self.watch_chain = Some(cx.background_spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            call(&client);
        }));
    }

    fn unwatch(&mut self, cx: &mut Context<Self>) {
        if let Some(root) = self.watched.take() {
            self.chain_watch(cx, move |client| {
                let _ = client.files_unwatch(&root);
            });
        }
    }

    fn sync_watch(&mut self, cx: &mut Context<Self>) {
        let wanted = self
            .shown
            .then(|| self.watched_git_root().map(Path::to_path_buf))
            .flatten();
        if let Some(root) = &wanted
            && !root.is_dir()
        {
            self.unwatch(cx);
            self.resolve(cx);
            return;
        }
        if self.watched == wanted {
            return;
        }
        self.unwatch(cx);
        let Some(root) = wanted else {
            return;
        };
        let already_loaded = matches!(&self.body, Body::Root(state) if state.commits.is_some());
        self.watched = Some(root.clone());
        self.chain_watch(cx, move |client| {
            let _ = client.files_watch(&root);
        });
        if already_loaded {
            self.load_snapshot(cx);
        }
    }

    fn on_files_event(&mut self, event: &FilesEvent, cx: &mut Context<Self>) {
        match event {
            FilesEvent::Changed(changed) => {
                if self.shown && self.watched_git_root() == Some(changed.root.as_path()) {
                    self.load_snapshot(cx);
                }
            }
            FilesEvent::Status { root, .. } => {
                if self.shown && self.watched_git_root() == Some(root.as_path()) {
                    self.load_snapshot(cx);
                }
            }
            FilesEvent::Viewed(viewed) => {
                self.apply_viewed_event(viewed, cx);
            }
            FilesEvent::RootChanged { subject, origin } => {
                if &self.context.subject == subject && cx.entity_id() != *origin {
                    self.resolve(cx);
                }
            }
            FilesEvent::Resync { rewatch } => {
                if self.context.remote {
                    return;
                }
                if !matches!(self.body, Body::Root(_)) {
                    self.resolve(cx);
                    return;
                }
                if !self.shown {
                    return;
                }
                if *rewatch {
                    self.watched = None;
                    self.sync_watch(cx);
                }
                self.load_snapshot(cx);
            }
        }
    }

    // ── Snapshot & diff loading ────────────────────────────────────────

    fn recover_from_root_error(&mut self, msg: &str, cx: &mut Context<Self>) -> bool {
        let root_missing = matches!(&self.body, Body::Root(state) if !state.root.path.is_dir())
            || msg.contains("is not an existing folder");
        if root_missing {
            self.resolve(cx);
            return true;
        }
        if msg.contains("not inside a git checkout") || msg.contains("git is not on PATH") {
            self.body = Body::NotInGit;
            self.watched_root_changed(cx);
            cx.notify();
            return true;
        }
        false
    }

    fn load_snapshot(&mut self, cx: &mut Context<Self>) {
        self.load_snapshot_inner(true, cx);
    }

    fn load_snapshot_inner(&mut self, reload_same_diff: bool, cx: &mut Context<Self>) {
        let Body::Root(state) = &mut self.body else {
            return;
        };
        state.snapshot_seq += 1;
        let seq = state.snapshot_seq;
        let root_path = state.root.path.clone();
        let wanted_commit = match &state.selected_target {
            SelectedTarget::Commit(id) => Some(id.clone()),
            _ => None,
        };
        #[cfg(test)]
        let hold_rx = self.hold_snapshots.then(|| {
            let (tx, rx) = futures::channel::oneshot::channel();
            self.held_snapshots.push(tx);
            rx
        });
        let client = self.client.clone();
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            let commits = client.changes_commits(&root_path)?;
            let uncommitted = client.changes_files(
                &root_path,
                &runner_core::protocol::changes::ChangesTarget::Uncommitted,
            )?;
            let branch = if has_all_changes(&commits) {
                Some(client.changes_files(
                    &root_path,
                    &runner_core::protocol::changes::ChangesTarget::Branch,
                )?)
            } else {
                None
            };
            let commit_files = if let Some(id) = wanted_commit {
                if commits.commits.iter().any(|c| c.id == id) {
                    client
                        .changes_files(
                            &root_path,
                            &runner_core::protocol::changes::ChangesTarget::Commit(id.clone()),
                        )
                        .ok()
                        .map(|files| (id, files))
                } else {
                    None
                }
            } else {
                None
            };
            Ok(SnapshotPayload {
                commits,
                uncommitted,
                branch,
                commit_files,
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            #[cfg(test)]
            if let Some(rx) = hold_rx {
                let _ = rx.await;
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.apply_snapshot(seq, reload_same_diff, result, cx);
                }
            });
        })
        .detach();
    }

    fn apply_snapshot(
        &mut self,
        seq: u64,
        reload_same_diff: bool,
        result: Result<SnapshotPayload, ClientError>,
        cx: &mut Context<Self>,
    ) {
        let is_current = match &self.body {
            Body::Root(state) => seq >= state.snapshot_seq,
            _ => false,
        };
        if !is_current {
            return;
        }
        let payload = match result {
            Ok(payload) => payload,
            Err(error) => {
                let msg = error.to_string();
                if self.recover_from_root_error(&msg, cx) {
                    return;
                }
                self.error = Some(msg.into());
                cx.notify();
                return;
            }
        };
        let Body::Root(state) = &mut self.body else {
            return;
        };
        self.error = None;
        let uncommitted_count = payload.uncommitted.files.len();
        let branch_count = payload.branch.as_ref().map_or(0, |b| b.files.len());
        let next_target = if !state.initial_target_chosen {
            state.initial_target_chosen = true;
            choose_initial_target(&payload.commits, uncommitted_count)
        } else {
            reconcile_target(&state.selected_target, &payload.commits, uncommitted_count)
        };

        let active_files = match &next_target {
            SelectedTarget::Uncommitted => payload.uncommitted,
            SelectedTarget::Branch => payload.branch.unwrap_or(ChangesFiles {
                checkout_top: payload.commits.checkout_top.clone(),
                files: Vec::new(),
                totals: ChangesTotals::default(),
            }),
            SelectedTarget::Commit(id) => payload
                .commit_files
                .filter(|(loaded_id, _)| loaded_id == id)
                .map(|(_, files)| files)
                .unwrap_or(ChangesFiles {
                    checkout_top: payload.commits.checkout_top.clone(),
                    files: Vec::new(),
                    totals: ChangesTotals::default(),
                }),
        };

        state.commits = Some(payload.commits);
        state.uncommitted_count = uncommitted_count;
        state.branch_count = branch_count;
        state.selected_target = next_target;
        self.apply_files_list(active_files, reload_same_diff, cx);
    }

    pub(super) fn select_target(&mut self, target: SelectedTarget, cx: &mut Context<Self>) {
        let Body::Root(state) = &mut self.body else {
            return;
        };
        if state.selected_target == target {
            return;
        }
        state.selected_target = target.clone();
        state.selected_path = None;
        state.highlighted_list_row = 0;
        state.diff_state = ActiveDiff::Empty;
        state.in_flight_diff = None;
        state.snapshot_seq += 1;
        let seq = state.snapshot_seq;
        let root_path = state.root.path.clone();
        let proto = target.to_protocol();
        self.request(
            cx,
            move |client| client.changes_files(&root_path, &proto),
            move |this, result, cx| {
                let is_current = match &this.body {
                    Body::Root(state) => seq >= state.snapshot_seq,
                    _ => false,
                };
                if !is_current {
                    return;
                }
                match result {
                    Ok(files) => {
                        if let Body::Root(state) = &mut this.body {
                            match state.selected_target {
                                SelectedTarget::Uncommitted => {
                                    state.uncommitted_count = files.files.len();
                                }
                                SelectedTarget::Branch => {
                                    state.branch_count = files.files.len();
                                }
                                SelectedTarget::Commit(_) => {}
                            }
                        }
                        this.apply_files_list(files, false, cx);
                    }
                    Err(error) => {
                        let msg = error.to_string();
                        if this.recover_from_root_error(&msg, cx) {
                            return;
                        }
                        this.error = Some(msg.into());
                        cx.notify();
                    }
                }
            },
        );
        cx.notify();
    }

    fn apply_files_list(
        &mut self,
        mut changes_files: ChangesFiles,
        reload_same_diff: bool,
        cx: &mut Context<Self>,
    ) {
        changes_files.files.sort_by(|a, b| a.path.cmp(&b.path));
        let tree_mode = files::settings(cx).is_some_and(|s| s.changes_tree_view);
        self.tree_view_pref = tree_mode;

        let (selected_path, should_load_diff) = {
            let Body::Root(state) = &mut self.body else {
                return;
            };
            let prev_path = state.selected_path.clone();
            let prev_highlight = state
                .list_rows
                .get(state.highlighted_list_row)
                .map(row_identity);
            let prev_highlight_idx = state.highlighted_list_row;
            state.files = changes_files.files;
            state.totals = changes_files.totals;
            state.list_rows = if tree_mode {
                build_tree_rows(&state.files, &state.collapsed_folders)
            } else {
                build_flat_rows(&state.files)
            };

            let next_path = reconcile_file_selection(
                prev_path.as_deref(),
                &state.files,
                &state.list_rows,
                tree_mode,
            );
            let same_path = next_path.is_some() && next_path == prev_path;
            state.selected_path = next_path.clone();
            if state.selected_path.is_some() {
                let wanted = if same_path {
                    prev_highlight
                } else {
                    state
                        .selected_path
                        .as_ref()
                        .map(|p| RowIdentity::File(p.clone()))
                };
                state.highlighted_list_row = resolve_highlighted_row(
                    wanted.as_ref(),
                    state.selected_path.as_deref(),
                    prev_highlight_idx,
                    &state.list_rows,
                );
                if !same_path {
                    self.files_scroll
                        .scroll_to_item(state.highlighted_list_row, ScrollStrategy::Nearest);
                }
            } else {
                state.highlighted_list_row = 0;
                state.diff_state = ActiveDiff::Empty;
                state.in_flight_diff = None;
            }
            if let ActiveDiff::Loaded(loaded) = &mut state.diff_state
                && let Some(file) = state
                    .files
                    .iter()
                    .find(|f| loaded.matches_file(&state.selected_target, f))
            {
                loaded.set_viewed_state(file.viewed, file.changed_since_viewed, cx);
            }
            let loaded_matches_selected = match (&state.selected_path, &state.diff_state) {
                (Some(path), ActiveDiff::Loaded(loaded)) => state
                    .files
                    .iter()
                    .find(|f| &f.path == path)
                    .is_some_and(|f| loaded.matches_file(&state.selected_target, f)),
                _ => false,
            };
            let already_in_flight = match (&state.selected_path, &state.in_flight_diff) {
                (Some(path), Some((_, in_flight_target, in_flight_path))) => {
                    in_flight_target == &state.selected_target && in_flight_path == path
                }
                _ => false,
            };
            let need_diff = if reload_same_diff {
                true
            } else {
                !loaded_matches_selected && !already_in_flight
            };
            (next_path, need_diff)
        };

        if let Some(path) = selected_path
            && should_load_diff
        {
            self.load_diff(path, cx);
        }
        cx.notify();
    }

    pub(super) fn select_file_by_path(&mut self, path: String, cx: &mut Context<Self>) {
        let changed = {
            let Body::Root(state) = &mut self.body else {
                return;
            };
            if let Some(row_idx) = state
                .list_rows
                .iter()
                .position(|r| matches!(r, ListRow::File { path: p, .. } if p == &path))
            {
                state.highlighted_list_row = row_idx;
                self.files_scroll
                    .scroll_to_item(row_idx, ScrollStrategy::Nearest);
            }
            if state.selected_path.as_deref() == Some(path.as_str()) {
                false
            } else {
                state.selected_path = Some(path.clone());
                true
            }
        };
        if changed {
            self.load_diff(path, cx);
        }
        cx.notify();
    }

    fn load_diff(&mut self, path: String, cx: &mut Context<Self>) {
        let (seq, root_path, selected_target, target) = {
            let Body::Root(state) = &mut self.body else {
                return;
            };
            state.diff_seq += 1;
            let seq = state.diff_seq;
            let selected_target = state.selected_target.clone();
            state.in_flight_diff = Some((seq, selected_target.clone(), path.clone()));
            let keep_existing = matches!(
                &state.diff_state,
                ActiveDiff::Loaded(loaded)
                    if loaded.version.target == selected_target && loaded.path == path
            );
            if !keep_existing {
                state.diff_state = ActiveDiff::Loading;
            }
            let proto = selected_target.to_protocol();
            (seq, state.root.path.clone(), selected_target, proto)
        };
        #[cfg(test)]
        let hold_rx = self.hold_diffs.then(|| {
            let (tx, rx) = futures::channel::oneshot::channel();
            self.held_diffs.push(tx);
            rx
        });
        let req_path = path.clone();
        let client = self.client.clone();
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            let diff = client.changes_diff(&root_path, &target, &req_path)?;
            let prepared = prepare_diff_body(&diff);
            Ok((diff, prepared))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            #[cfg(test)]
            if let Some(rx) = hold_rx {
                let _ = rx.await;
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.apply_diff(seq, selected_target, path, result, cx);
                }
            });
        })
        .detach();
    }

    fn apply_diff(
        &mut self,
        seq: u64,
        requested_target: SelectedTarget,
        requested_path: String,
        result: Result<(ChangesDiff, PreparedDiffBody), ClientError>,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.weak_entity();
        {
            let Body::Root(state) = &mut self.body else {
                return;
            };
            if state
                .in_flight_diff
                .as_ref()
                .is_some_and(|(s, ..)| *s == seq)
            {
                state.in_flight_diff = None;
            }
            if seq < state.diff_seq
                || state.selected_target != requested_target
                || state.selected_path.as_deref() != Some(requested_path.as_str())
            {
                return;
            }
        }
        let mut list_stale = false;
        match result {
            Ok((diff, prepared)) => {
                let effective_mode = self.effective_diff_mode(cx);
                let Body::Root(state) = &mut self.body else {
                    return;
                };
                let version = DiffVersion {
                    target: requested_target,
                    path: diff.path.clone(),
                    old_blob_id: diff.old_blob_id,
                    new_blob_id: diff.new_blob_id,
                };
                let matching_file = state.files.iter().find(|f| {
                    f.path == version.path
                        && f.old_blob_id == version.old_blob_id
                        && f.new_blob_id == version.new_blob_id
                });
                let (viewed, changed_since_viewed) = matching_file
                    .map(|f| (f.viewed, f.changed_since_viewed))
                    .unwrap_or((false, false));
                list_stale = matching_file.is_none();

                let file_name: SharedString = model::file_name(&diff.path).to_owned().into();
                let grammar = highlight::grammar_for_path(&diff.path);

                let kind = match prepared {
                    PreparedDiffBody::Unified {
                        old_doc,
                        new_doc,
                        hunks,
                    } => {
                        if let ActiveDiff::Loaded(existing) = &state.diff_state
                            && existing.path == diff.path
                            && let LoadedDiffKind::Unified(entity) = &existing.kind
                        {
                            let entity = entity.clone();
                            entity.update(cx, |v, cx| {
                                v.set_mode(effective_mode, cx);
                                v.update_content(
                                    old_doc,
                                    new_doc,
                                    &hunks,
                                    grammar,
                                    file_name,
                                    viewed,
                                    changed_since_viewed,
                                    cx,
                                );
                            });
                            LoadedDiffKind::Unified(entity)
                        } else {
                            let esc_weak = weak.clone();
                            let toggle_weak = weak.clone();
                            let focus_weak = weak;
                            let entity = cx.new(|cx| {
                                let mut v = UnifiedDiffView::new(
                                    old_doc,
                                    new_doc,
                                    &hunks,
                                    grammar,
                                    file_name,
                                    viewed,
                                    changed_since_viewed,
                                    cx,
                                );
                                v.set_mode(effective_mode, cx);
                                v.set_callbacks(
                                    Rc::new(move |window, cx| {
                                        let _ = esc_weak.update(cx, |this, cx| {
                                            this.focus_list(window, cx);
                                        });
                                    }),
                                    Some(Rc::new(move |_, cx| {
                                        let _ = toggle_weak.update(cx, |this, cx| {
                                            this.toggle_viewed_for_loaded_diff(cx);
                                        });
                                    })),
                                    Rc::new(move |_, cx| {
                                        let _ = focus_weak.update(cx, |this, cx| {
                                            if this.focus_place != FocusPlace::Diff {
                                                this.focus_place = FocusPlace::Diff;
                                                cx.notify();
                                            }
                                        });
                                    }),
                                );
                                v
                            });
                            LoadedDiffKind::Unified(entity)
                        }
                    }
                    PreparedDiffBody::CrlfOnly => LoadedDiffKind::CrlfOnly,
                    PreparedDiffBody::Binary { old_size, new_size } => {
                        LoadedDiffKind::Binary { old_size, new_size }
                    }
                    PreparedDiffBody::TooLarge { old_size, new_size } => {
                        LoadedDiffKind::TooLarge { old_size, new_size }
                    }
                    PreparedDiffBody::ModeOnly { old_mode, new_mode } => {
                        LoadedDiffKind::ModeOnly { old_mode, new_mode }
                    }
                    PreparedDiffBody::Submodule {
                        old_commit,
                        new_commit,
                    } => LoadedDiffKind::Submodule {
                        old_commit,
                        new_commit,
                    },
                };

                state.diff_state = ActiveDiff::Loaded(Box::new(LoadedDiff {
                    version,
                    path: diff.path,
                    status: diff.status,
                    counts: diff.counts,
                    mode_change: diff.mode_change,
                    conflicted: diff.conflicted,
                    viewed,
                    changed_since_viewed,
                    kind,
                }));
            }
            Err(error) => {
                let msg = error.to_string();
                if self.recover_from_root_error(&msg, cx) {
                    return;
                }
                if msg.contains("has no changes in target") {
                    self.load_snapshot(cx);
                    return;
                }
                if let Body::Root(state) = &mut self.body {
                    state.diff_state = ActiveDiff::Failed(msg.into());
                }
            }
        }
        if list_stale {
            self.load_snapshot_inner(false, cx);
        }
        cx.notify();
    }

    // ── Viewed marks & flat/tree toggle ────────────────────────────────

    pub(super) fn toggle_viewed_for_path(&mut self, path: &str, cx: &mut Context<Self>) {
        let (root_path, target, old_blob, new_blob, next_viewed) = {
            let Body::Root(state) = &mut self.body else {
                return;
            };
            let (next_viewed, changed, old_blob, new_blob, matches_loaded) = {
                let Some(file) = state.files.iter_mut().find(|f| f.path == path) else {
                    return;
                };
                let next_viewed = !file.viewed;
                file.viewed = next_viewed;
                if next_viewed {
                    file.changed_since_viewed = false;
                }
                let matches_loaded = match &state.diff_state {
                    ActiveDiff::Loaded(loaded) => loaded.matches_file(&state.selected_target, file),
                    _ => false,
                };
                (
                    next_viewed,
                    file.changed_since_viewed,
                    file.old_blob_id.clone(),
                    file.new_blob_id.clone(),
                    matches_loaded,
                )
            };
            state.totals.viewed = state.files.iter().filter(|f| f.viewed).count();
            if matches_loaded && let ActiveDiff::Loaded(loaded) = &mut state.diff_state {
                loaded.set_viewed_state(next_viewed, changed, cx);
            }
            (
                state.root.path.clone(),
                state.selected_target.to_protocol(),
                old_blob,
                new_blob,
                next_viewed,
            )
        };
        let req_path = path.to_owned();
        self.send_viewed_toggle(
            root_path,
            target,
            req_path,
            old_blob,
            new_blob,
            next_viewed,
            cx,
        );
    }

    pub(super) fn toggle_viewed_for_loaded_diff(&mut self, cx: &mut Context<Self>) {
        let (root_path, version, next_viewed) = {
            let Body::Root(state) = &mut self.body else {
                return;
            };
            let ActiveDiff::Loaded(loaded) = &mut state.diff_state else {
                return;
            };
            let next_viewed = !loaded.viewed;
            let next_changed = if next_viewed {
                false
            } else {
                loaded.changed_since_viewed
            };
            loaded.set_viewed_state(next_viewed, next_changed, cx);
            let version = loaded.version.clone();
            if state.selected_target == version.target
                && let Some(file) = state.files.iter_mut().find(|f| {
                    f.path == version.path
                        && f.old_blob_id == version.old_blob_id
                        && f.new_blob_id == version.new_blob_id
                })
            {
                file.viewed = next_viewed;
                if next_viewed {
                    file.changed_since_viewed = false;
                }
                state.totals.viewed = state.files.iter().filter(|f| f.viewed).count();
            }
            (state.root.path.clone(), version, next_viewed)
        };
        let proto = version.target.to_protocol();
        self.send_viewed_toggle(
            root_path,
            proto,
            version.path,
            version.old_blob_id,
            version.new_blob_id,
            next_viewed,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn send_viewed_toggle(
        &mut self,
        root_path: PathBuf,
        target: runner_core::protocol::changes::ChangesTarget,
        req_path: String,
        old_blob: Option<String>,
        new_blob: Option<String>,
        next_viewed: bool,
        cx: &mut Context<Self>,
    ) {
        self.request(
            cx,
            move |client| {
                client.changes_set_viewed(
                    &root_path,
                    &target,
                    &req_path,
                    old_blob.as_deref(),
                    new_blob.as_deref(),
                    next_viewed,
                )
            },
            |this, result, cx| {
                if let Err(error) = result {
                    let msg = error.to_string();
                    if this.recover_from_root_error(&msg, cx) {
                        return;
                    }
                    this.error = Some(msg.into());
                    this.load_snapshot_inner(false, cx);
                }
            },
        );
        cx.notify();
    }

    fn apply_viewed_event(&mut self, event: &ChangesViewed, cx: &mut Context<Self>) {
        let mut matched_checkout = false;
        if let Body::Root(state) = &mut self.body
            && state
                .commits
                .as_ref()
                .is_some_and(|c| c.checkout_top == event.checkout_top)
        {
            matched_checkout = true;
            let event_abs = absolute(&event.checkout_top, &event.path);
            for file in &mut state.files {
                if absolute(&state.root.path, &file.path) == event_abs
                    && file.old_blob_id == event.old_blob_id
                    && file.new_blob_id == event.new_blob_id
                {
                    file.viewed = event.viewed;
                    if event.viewed {
                        file.changed_since_viewed = false;
                    }
                }
            }
            if let ActiveDiff::Loaded(loaded) = &mut state.diff_state
                && absolute(&state.root.path, &loaded.version.path) == event_abs
                && loaded.version.old_blob_id == event.old_blob_id
                && loaded.version.new_blob_id == event.new_blob_id
            {
                let changed = if event.viewed {
                    false
                } else {
                    loaded.changed_since_viewed
                };
                loaded.set_viewed_state(event.viewed, changed, cx);
            }
            state.totals.viewed = state.files.iter().filter(|f| f.viewed).count();
        }
        if matched_checkout {
            if self.shown {
                self.load_snapshot_inner(false, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn set_tree_view(&mut self, tree_view: bool, cx: &mut Context<Self>) {
        crate::app_store::global_app_store(cx).update(cx, |store, cx| {
            store.update_settings(
                |settings| {
                    let changed = settings.changes_tree_view != tree_view;
                    settings.changes_tree_view = tree_view;
                    changed
                },
                true,
                cx,
            );
        });
        self.rebuild_list_rows(tree_view, cx);
    }

    fn rebuild_list_rows(&mut self, tree_view: bool, cx: &mut Context<Self>) {
        self.tree_view_pref = tree_view;
        if let Body::Root(state) = &mut self.body {
            let prev_highlight = state
                .list_rows
                .get(state.highlighted_list_row)
                .map(row_identity);
            let prev_idx = state.highlighted_list_row;
            state.list_rows = if tree_view {
                build_tree_rows(&state.files, &state.collapsed_folders)
            } else {
                build_flat_rows(&state.files)
            };
            state.highlighted_list_row = resolve_highlighted_row(
                prev_highlight.as_ref(),
                state.selected_path.as_deref(),
                prev_idx,
                &state.list_rows,
            );
            self.files_scroll
                .scroll_to_item(state.highlighted_list_row, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    pub(super) fn toggle_folder(&mut self, folder_path: &str, cx: &mut Context<Self>) {
        let Body::Root(state) = &mut self.body else {
            return;
        };
        if !state.collapsed_folders.remove(folder_path) {
            state.collapsed_folders.insert(folder_path.to_owned());
        }
        let tree_mode = self.tree_view_pref;
        self.rebuild_list_rows(tree_mode, cx);
    }

    // ── Settings, zoom, divider resize & external launch ───────────────

    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let look = Look::current(cx);
        let tree_pref = files::settings(cx).is_some_and(|s| s.changes_tree_view);
        if tree_pref != self.tree_view_pref {
            self.rebuild_list_rows(tree_pref, cx);
        }
        let diff_mode = files::settings(cx).map_or(DiffViewMode::Unified, |s| s.changes_diff_mode);
        if diff_mode != self.diff_mode_pref {
            self.diff_mode_pref = diff_mode;
            self.sync_effective_diff_mode(cx);
            cx.notify();
        }
        if look == self.look {
            return;
        }
        self.look = look;
        if let Body::Root(state) = &self.body
            && let ActiveDiff::Loaded(loaded) = &state.diff_state
            && let LoadedDiffKind::Unified(entity) = &loaded.kind
        {
            entity.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }

    fn diff_has_focus(&self, window: &Window, cx: &App) -> bool {
        if self.diff_focus.is_focused(window) {
            return true;
        }
        if let Body::Root(state) = &self.body
            && let ActiveDiff::Loaded(loaded) = &state.diff_state
            && let LoadedDiffKind::Unified(entity) = &loaded.kind
        {
            return entity.read(cx).focus_handle().is_focused(window);
        }
        false
    }

    pub(super) fn zoom(
        &mut self,
        step: i16,
        fallback: Box<dyn gpui::Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.diff_has_focus(window, cx) {
            window.dispatch_action(fallback, cx);
            return;
        }
        crate::app_store::global_app_store(cx).update(cx, |store, cx| {
            store.update_settings(
                |settings| {
                    let current = settings.files_font_size() as i16;
                    let next = (current + step)
                        .clamp(TERMINAL_FONT_SIZE_MIN as i16, TERMINAL_FONT_SIZE_MAX as i16)
                        as u16;
                    let changed = settings.files_font_size != Some(next);
                    settings.files_font_size = Some(next);
                    changed
                },
                true,
                cx,
            );
        });
    }

    pub(super) fn resize_tree(&mut self, tree: f32, cx: &mut Context<Self>) {
        let Some(widened) = self.widened.filter(|w| w.tree) else {
            return;
        };
        self.tree_resizing = true;
        if self.maximized {
            let total = widened.file + widened.tree_width;
            let tree = crate::app_settings::clamp_file_tree_width(tree)
                .min((total - widened.standard).max(crate::app_settings::FILE_TREE_MIN));
            crate::app_store::global_app_store(cx).update(cx, |store, cx| {
                store.update_settings(
                    |settings| {
                        let changed = settings.file_tree_width != tree;
                        settings.file_tree_width = tree;
                        changed
                    },
                    false,
                    cx,
                );
            });
        } else {
            let (file, tree) = widened.resize_tree(tree);
            crate::app_store::global_app_store(cx).update(cx, |store, cx| {
                store.update_settings(
                    |settings| {
                        let changed =
                            settings.file_tree_width != tree || settings.file_viewer_width != file;
                        settings.file_tree_width = tree;
                        settings.file_viewer_width = file;
                        changed
                    },
                    false,
                    cx,
                );
            });
        }
        if let Some(owner) = self.owner {
            cx.defer(move |cx| cx.notify(owner));
        }
        cx.notify();
    }

    pub(super) fn finish_tree_resize(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.tree_resizing) {
            crate::app_store::global_app_store(cx)
                .read(cx)
                .save_settings();
            cx.notify();
        }
    }

    pub(super) fn launch_external(
        &mut self,
        editor: Option<FileLinkEditor>,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        #[cfg(test)]
        {
            let _ = cx;
            self.launched.push((editor, path));
        }
        #[cfg(not(test))]
        {
            let target = FileLinkTarget {
                path,
                line: None,
                column: None,
            };
            match editor {
                Some(editor) => file_links::open_file_with(editor, target, cx),
                None => file_links::open_file_link(target, cx),
            }
        }
    }

    // ── Focus & Keyboard ───────────────────────────────────────────────

    pub(super) fn on_focus_lost(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = window.focus_lost_restore_target(cx) else {
            return;
        };
        if target == self.diff_focus
            && self.focus_place == FocusPlace::Diff
            && matches!(
                &self.body,
                Body::Root(state) if !matches!(state.diff_state, ActiveDiff::Empty)
            )
        {
            self.diff_focus.focus(window, cx);
            cx.notify();
        } else if target == self.root_focus || target == self.diff_focus {
            self.focus_list(window, cx);
        }
    }

    pub(super) fn focus_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus_place == FocusPlace::Diff {
            self.focus_place = FocusPlace::Files;
        }
        self.list_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn enter_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_place = FocusPlace::Diff;
        if let Body::Root(state) = &self.body
            && let ActiveDiff::Loaded(loaded) = &state.diff_state
            && let LoadedDiffKind::Unified(entity) = &loaded.kind
        {
            entity.read(cx).focus_handle().clone().focus(window, cx);
        } else {
            self.diff_focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn jump_hunk(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        if let Body::Root(state) = &self.body
            && let ActiveDiff::Loaded(loaded) = &state.diff_state
            && let LoadedDiffKind::Unified(entity) = &loaded.kind
        {
            return entity.update(cx, |v, cx| v.jump_hunk(forward, cx));
        }
        false
    }

    pub(super) fn on_list_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return;
        }
        match event.keystroke.key.as_str() {
            "tab" => {
                cx.stop_propagation();
                self.focus_place = self.focus_place.toggle_list_section();
                cx.notify();
            }
            "down" | "j" => {
                cx.stop_propagation();
                self.step_list(1, cx);
            }
            "up" | "k" => {
                cx.stop_propagation();
                self.step_list(-1, cx);
            }
            "left" if self.focus_place == FocusPlace::Files => {
                if self.move_list_left(cx) {
                    cx.stop_propagation();
                }
            }
            "right" if self.focus_place == FocusPlace::Files => {
                if self.move_list_right(cx) {
                    cx.stop_propagation();
                }
            }
            "enter" => {
                cx.stop_propagation();
                if self.focus_place == FocusPlace::Files
                    && let Some((folder_path, _)) = self.highlighted_folder()
                {
                    self.toggle_folder(&folder_path, cx);
                } else {
                    self.enter_diff(window, cx);
                }
            }
            "space" if self.focus_place == FocusPlace::Files => {
                cx.stop_propagation();
                if let Some((folder_path, _)) = self.highlighted_folder() {
                    self.toggle_folder(&folder_path, cx);
                } else if let Body::Root(state) = &self.body {
                    let path = match state.list_rows.get(state.highlighted_list_row) {
                        Some(ListRow::File { path, .. }) => Some(path.clone()),
                        _ => state.selected_path.clone(),
                    };
                    if let Some(path) = path {
                        self.toggle_viewed_for_path(&path, cx);
                    }
                }
            }
            "]" => {
                cx.stop_propagation();
                self.jump_hunk(true, cx);
            }
            "[" => {
                cx.stop_propagation();
                self.jump_hunk(false, cx);
            }
            _ => {}
        }
    }

    fn move_list_left(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some((folder_path, true)) = self.highlighted_folder() {
            self.toggle_folder(&folder_path, cx);
            return true;
        }
        let parent_idx = {
            let Body::Root(state) = &self.body else {
                return false;
            };
            let Some(row) = state.list_rows.get(state.highlighted_list_row) else {
                return false;
            };
            find_nearest_visible_ancestor_folder(row.path(), &state.list_rows)
        };
        if let Some(idx) = parent_idx
            && let Body::Root(state) = &mut self.body
        {
            state.highlighted_list_row = idx;
            self.files_scroll
                .scroll_to_item(idx, ScrollStrategy::Nearest);
            cx.notify();
            return true;
        }
        false
    }

    fn move_list_right(&mut self, cx: &mut Context<Self>) -> bool {
        match self.highlighted_folder() {
            Some((folder_path, false)) => {
                self.toggle_folder(&folder_path, cx);
                true
            }
            Some((_, true)) => {
                let has_visible_child = match &self.body {
                    Body::Root(state) => match (
                        state.list_rows.get(state.highlighted_list_row),
                        state.list_rows.get(state.highlighted_list_row + 1),
                    ) {
                        (
                            Some(ListRow::Folder { depth, .. }),
                            Some(
                                ListRow::Folder {
                                    depth: child_depth, ..
                                }
                                | ListRow::File {
                                    depth: child_depth, ..
                                },
                            ),
                        ) => child_depth > depth,
                        _ => false,
                    },
                    _ => false,
                };
                if has_visible_child {
                    self.step_list(1, cx);
                    true
                } else {
                    false
                }
            }
            None => false,
        }
    }

    pub(super) fn on_placeholder_diff_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return;
        }
        if event.keystroke.key == "escape" {
            cx.stop_propagation();
            self.focus_list(window, cx);
        }
    }

    fn highlighted_folder(&self) -> Option<(String, bool)> {
        let Body::Root(state) = &self.body else {
            return None;
        };
        match state.list_rows.get(state.highlighted_list_row)? {
            ListRow::Folder { path, expanded, .. } => Some((path.clone(), *expanded)),
            ListRow::File { .. } => None,
        }
    }

    fn step_list(&mut self, delta: isize, cx: &mut Context<Self>) {
        match self.focus_place {
            FocusPlace::Commits => {
                let next_target = {
                    let Body::Root(state) = &self.body else {
                        return;
                    };
                    let Some(commits) = &state.commits else {
                        return;
                    };
                    let rows: Vec<CommitRow> =
                        build_commit_rows(commits, state.branch_count, state.uncommitted_count);
                    if rows.is_empty() {
                        return;
                    }
                    let current_idx = rows
                        .iter()
                        .position(|r| r.target() == state.selected_target)
                        .unwrap_or(0);
                    let next_idx = (current_idx as isize + delta)
                        .clamp(0, rows.len().saturating_sub(1) as isize)
                        as usize;
                    self.commits_scroll.scroll_to_item(next_idx);
                    rows[next_idx].target()
                };
                self.select_target(next_target, cx);
            }
            FocusPlace::Files | FocusPlace::Diff => {
                self.focus_place = FocusPlace::Files;
                let file_to_select = {
                    let Body::Root(state) = &mut self.body else {
                        return;
                    };
                    if state.list_rows.is_empty() {
                        return;
                    }
                    let next_idx = (state.highlighted_list_row as isize + delta)
                        .clamp(0, state.list_rows.len().saturating_sub(1) as isize)
                        as usize;
                    state.highlighted_list_row = next_idx;
                    self.files_scroll
                        .scroll_to_item(next_idx, ScrollStrategy::Nearest);
                    match &state.list_rows[next_idx] {
                        ListRow::File { path, .. } => Some(path.clone()),
                        ListRow::Folder { .. } => None,
                    }
                };
                if let Some(path) = file_to_select {
                    self.select_file_by_path(path, cx);
                } else {
                    cx.notify();
                }
            }
        }
    }
}

#[cfg(test)]
impl ChangesView {
    pub(crate) fn receive(&mut self, event: FilesEvent, cx: &mut Context<Self>) {
        self.on_files_event(&event, cx);
    }

    pub(crate) fn selected_target(&self) -> Option<&SelectedTarget> {
        match &self.body {
            Body::Root(state) => Some(&state.selected_target),
            _ => None,
        }
    }

    pub(crate) fn selected_path(&self) -> Option<&str> {
        match &self.body {
            Body::Root(state) => state.selected_path.as_deref(),
            _ => None,
        }
    }

    pub(crate) fn focus_place(&self) -> FocusPlace {
        self.focus_place
    }

    pub(crate) fn unified_diff_view(&self) -> Option<Entity<UnifiedDiffView>> {
        match &self.body {
            Body::Root(state) => match &state.diff_state {
                ActiveDiff::Loaded(loaded) => match &loaded.kind {
                    LoadedDiffKind::Unified(entity) => Some(entity.clone()),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }

    pub(crate) fn set_hold_diffs(&mut self, hold: bool) {
        self.hold_diffs = hold;
    }

    pub(crate) fn release_held_diffs(&mut self) -> usize {
        let mut released = 0;
        for tx in self.held_diffs.drain(..) {
            let _ = tx.send(());
            released += 1;
        }
        released
    }

    pub(crate) fn set_hold_snapshots(&mut self, hold: bool) {
        self.hold_snapshots = hold;
    }

    pub(crate) fn release_held_snapshots(&mut self) -> usize {
        let mut released = 0;
        for tx in self.held_snapshots.drain(..) {
            let _ = tx.send(());
            released += 1;
        }
        released
    }

    pub(crate) fn reload_diff_for_test(&mut self, path: &str, cx: &mut Context<Self>) {
        self.load_diff(path.to_owned(), cx);
    }

    pub(crate) fn highlighted_row(&self) -> Option<&ListRow> {
        match &self.body {
            Body::Root(state) => state.list_rows.get(state.highlighted_list_row),
            _ => None,
        }
    }

    pub(crate) fn is_diff_loading(&self) -> bool {
        matches!(
            &self.body,
            Body::Root(state) if matches!(state.diff_state, ActiveDiff::Loading)
        )
    }

    pub(super) fn loaded_diff_version(&self) -> Option<&DiffVersion> {
        match &self.body {
            Body::Root(state) => match &state.diff_state {
                ActiveDiff::Loaded(loaded) => Some(&loaded.version),
                _ => None,
            },
            _ => None,
        }
    }
}
