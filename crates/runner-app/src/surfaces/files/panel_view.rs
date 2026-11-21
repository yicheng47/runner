//! Files panel state, initialization, focus, sizing and surface composition.

mod roots;
mod tree;
mod watch;

pub(crate) use roots::compact_missing_path;

use std::cell::Cell;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    canvas, div, img, px, rems, svg, AnyElement, App, Bounds, ClipboardItem, Context, Entity,
    FocusHandle, FontWeight, ImageSource, KeyDownEvent, MouseButton, MouseDownEvent,
    PathPromptOptions, Pixels, Resource, ScrollHandle, ScrollStrategy, SharedString, Subscription,
    Task, UniformListScrollHandle, Window,
};
use runner_app::ui::{
    menu::{popup_layer_sized, ContextMenu, MenuItem, PopupWidth},
    resize::{resize_strip, resize_strip_inset, ResizeAxis},
    Button, ButtonVariant, IconButton, Scrollbar, TextField,
};
use runner_core::protocol::files::{
    DirEntry, FilesRoot, FilesRootOutcome, FilesSubject, WorktreeEntry, WorktreeKind,
};
use runner_core::protocol::{ClientError, DaemonClient};

use super::links::{LinkRoot, Links};
use super::open_file::{reveal, reveal_label, Viewer};
use super::picker::{self, PickerItem, PickerKind, PickerSources};
use super::tree::{FileTree, Letter, Moved, PreparedStatus, Row};
use super::{FilesEvent, FilesHub};
use crate::file_icons::{self, IconSet};
use crate::surfaces::side_panel::{PanelContext, PanelSubject, PanelSurface, Widened};
use crate::text_util::{abbreviate_home, HomePath};
use crate::theme;

/// A drag of the divider between an open file and the tree.
#[derive(Clone)]
pub(crate) struct FileTreeResizeDrag;

impl Render for FileTreeResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1.)).h(px(1.))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum TreeFilter {
    #[default]
    All,
    Changed,
}

enum Body {
    Loading,
    Remote,
    Choose,
    Missing { path: PathBuf },
    Root(Box<RootState>),
    Failed(SharedString),
}

struct RootState {
    root: FilesRoot,
    tree: FileTree,
    /// The newest listing request applied per folder, so an older reply
    /// arriving late never replaces a newer one.
    listed: std::collections::HashMap<String, u64>,
    next_listing: u64,
    /// Paths with uncommitted changes against `HEAD` (`ChangesTarget::Uncommitted`),
    /// so `Files`'s `Diff` toggle reflects actual `Working tree vs HEAD` diffs.
    uncommitted_paths: Option<std::collections::HashSet<String>>,
    uncommitted_seq: u64,
}

pub(super) struct Picker {
    field: Entity<TextField>,
    query: String,
    worktrees: Vec<WorktreeEntry>,
    pub(super) items: Vec<PickerItem>,
    pub(super) highlighted: usize,
    /// The arrows moved the highlight, so a list that grows leaves it there.
    moved: bool,
    scroll: ScrollHandle,
    /// Scroll the highlighted item into view on the next frame.
    reveal: Cell<bool>,
    _observe: Subscription,
}

#[derive(Clone, Debug)]
pub(super) struct SavedFilesState {
    pub(super) root: PathBuf,
    pub(super) open_paths: Vec<String>,
    pub(super) active_path: Option<String>,
    pub(super) tree_hidden: bool,
    pub(super) filter: TreeFilter,
}

pub(crate) struct FilesView {
    context: PanelContext,
    client: DaemonClient,
    pub(super) focus: FocusHandle,
    /// The file viewer's own focus, for a placeholder or a loading file
    /// while the tree is hidden.
    pub(super) viewer_focus: FocusHandle,
    /// The whole view, so focus lost inside it can be told from elsewhere.
    root_focus: FocusHandle,
    _focus_lost: Option<Subscription>,
    body: Body,
    /// Bumped whenever the root changes, so a reply for an earlier root is dropped.
    generation: u64,
    /// Bumped per root change sent, so only the latest one's reply applies.
    root_change: u64,
    pub(super) shown: bool,
    watched: Option<PathBuf>,
    /// The last watch or unwatch sent; each waits for the one before, so an
    /// unwatch never overtakes the watch it undoes.
    watch_chain: Option<Task<()>>,
    /// The last root sent; each waits for the one before, so the daemon saves
    /// the roots in the order they were chosen.
    root_chain: Option<Task<()>>,
    /// Holds the next root write until the test lets it go.
    #[cfg(test)]
    pub(super) root_gate: Option<futures::channel::oneshot::Receiver<()>>,
    error: Option<SharedString>,
    /// Why the last asked-for status failed, until a status arrives.
    pub(super) status_error: Option<SharedString>,
    recent: Vec<PathBuf>,
    pub(super) choose_field: Entity<TextField>,
    pub(super) picker: Option<Picker>,
    picker_anchor: Rc<Cell<Bounds<Pixels>>>,
    pub(super) finder: Option<super::finder::Finder>,
    /// The whole view, which the finder centres over.
    pub(super) view_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub(super) menu: Option<Entity<ContextMenu>>,
    pub(super) scroll: UniformListScrollHandle,
    scrollbar: Entity<Scrollbar>,
    pub(super) viewer: Viewer,
    pub(super) links: Links,
    pub(super) filter: TreeFilter,
    pub(super) maximized: bool,
    pub(super) on_request_maximize: Option<crate::surfaces::side_panel::PanelMaximizeCallback>,
    pub(super) diff_mode_pref: crate::app_settings::DiffViewMode,
    /// Redrawn when the panel's width should change.
    owner: Option<gpui::EntityId>,
    /// The widening, open-file, and dirty-changes state last told to the owner.
    reported: (Option<bool>, bool, bool),
    /// Set when the view hides (watch stopped) and cleared when fresh status lands.
    pub(super) status_stale: bool,
    /// Open file tabs / filter preserved across LRU view eviction (#1403).
    pub(super) pending_restore: Option<SavedFilesState>,
    /// Whether the divider between the file and the tree is being dragged.
    tree_resizing: bool,
    /// Settings → Side panel → Files as last drawn.
    pub(super) look: Option<super::Look>,
    /// Reads an open ignored file again while the view shows.
    ignored_refresh: Option<Task<()>>,
    _hub: Subscription,
    _settings: Subscription,
}

/// `relative` (`/`-separated) inside `root`, as a path on this computer,
/// with its own separators even where git gave the root as `C:/…`.
pub(crate) fn absolute(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.components().collect(), |path, part| path.join(part))
}

pub(super) fn letter_color(letter: Letter) -> gpui::Hsla {
    match letter {
        Letter::Modified => theme::warning(),
        Letter::Added | Letter::Untracked => theme::accent(),
        Letter::Renamed => theme::info(),
        Letter::Unmerged => theme::danger(),
    }
}

impl FilesView {
    pub(crate) fn new(context: PanelContext, cx: &mut Context<Self>) -> Self {
        let client = crate::app_store::global_app_store(cx)
            .read(cx)
            .client
            .clone();
        let hub = super::hub(&client, cx);
        let hub_subscription = cx.subscribe(&hub, |this, _: Entity<FilesHub>, event, cx| {
            this.on_files_event(event, cx)
        });
        let store = crate::app_store::global_app_store(cx);
        let settings_subscription = cx.observe(&store, |this, _, cx| this.settings_changed(cx));
        // Weak: the view owns this field, so a strong handle would keep the
        // view, and its watch, alive forever.
        let root = cx.weak_entity();
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
                // The field is mid-update here, so read it once it is done.
                let root = root.clone();
                window.defer(cx, move |window, cx| {
                    let _ = root.update(cx, |this, cx| this.apply_typed_path(window, cx));
                });
                true
            }))
        });
        let scroll = UniformListScrollHandle::new();
        let owner = cx.entity_id();
        let base = scroll.0.borrow().base_handle.clone();
        let scrollbar = cx.new(|_| Scrollbar::app(base, owner));
        let mut view = Self {
            body: if context.remote {
                Body::Remote
            } else {
                Body::Loading
            },
            context,
            client,
            focus: cx.focus_handle(),
            viewer_focus: cx.focus_handle(),
            root_focus: cx.focus_handle(),
            _focus_lost: None,
            generation: 0,
            root_change: 0,
            shown: false,
            watched: None,
            watch_chain: None,
            root_chain: None,
            #[cfg(test)]
            root_gate: None,
            error: None,
            status_error: None,
            recent: Vec::new(),
            choose_field,
            picker: None,
            picker_anchor: Rc::default(),
            finder: None,
            view_bounds: Rc::default(),
            menu: None,
            scroll,
            scrollbar,
            viewer: Viewer::default(),
            links: Links::default(),
            filter: TreeFilter::All,
            maximized: false,
            on_request_maximize: None,
            diff_mode_pref: super::settings(cx)
                .map_or(crate::app_settings::DiffViewMode::Unified, |settings| {
                    settings.changes_diff_mode
                }),
            owner: None,
            reported: (None, false, false),
            status_stale: false,
            pending_restore: None,
            tree_resizing: false,
            look: super::Look::current(cx),
            ignored_refresh: None,
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

    /// The focused element went undrawn: the tree hid, a tab closed, its
    /// code became a placeholder, or find's field went with it. When that
    /// was inside this view, focus moves to what the view still draws, so
    /// its keys keep working; focus lost elsewhere is left alone.
    fn on_focus_lost(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ours = window
            .focus_lost_restore_target(cx)
            .is_some_and(|target| target == self.root_focus || target == self.viewer_focus);
        if ours {
            self.keep_focus(window, cx);
        }
    }

    /// How the owner widened the panel for an open file, and so whether the
    /// tree fits beside it and how wide it is.
    pub(crate) fn set_widened(&mut self, widened: Option<Widened>, cx: &mut Context<Self>) {
        if self.viewer.widened != widened {
            self.viewer.widened = widened;
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

    pub(crate) fn set_on_request_maximize(
        &mut self,
        handler: Option<crate::surfaces::side_panel::PanelMaximizeCallback>,
    ) {
        self.on_request_maximize = handler;
    }

    pub(crate) fn focus_default(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.root_focus.contains_focused(window, cx) {
            return;
        }
        if matches!(self.body, Body::Root(_)) {
            self.keep_focus(window, cx);
        } else {
            self.root_focus.focus(window, cx);
        }
    }

    pub(super) fn diff_width(&self) -> f32 {
        self.viewer
            .widened
            .map_or(crate::app_settings::FILE_VIEWER_DEFAULT, |widened| {
                widened.file
            })
    }

    pub(super) fn effective_diff_mode(&self, cx: &App) -> crate::app_settings::DiffViewMode {
        let preferred = super::settings(cx).map_or(self.diff_mode_pref, |s| s.changes_diff_mode);
        crate::surfaces::side_panel::effective_diff_mode(
            preferred,
            self.diff_width(),
            self.maximized,
        )
    }

    pub(super) fn select_diff_mode(
        &mut self,
        mode: crate::app_settings::DiffViewMode,
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
        if mode == crate::app_settings::DiffViewMode::Split
            && !self.maximized
            && self.diff_width() < crate::surfaces::side_panel::SPLIT_MIN_DIFF_WIDTH
            && let Some(request) = self.on_request_maximize.clone()
        {
            self.maximized = true;
            request(true, window, cx);
        }
        self.sync_effective_diff_mode(cx);
        cx.notify();
    }

    pub(crate) fn set_owner(&mut self, owner: Option<gpui::EntityId>) {
        self.owner = owner;
    }

    pub(super) fn notify_owner(&self, cx: &mut Context<Self>) {
        if let Some(owner) = self.owner {
            cx.defer(move |cx| cx.notify(owner));
        }
    }

    /// Sizes the tree `tree` wide from the divider's drag, trading width
    /// with the file so the panel keeps its width.
    fn resize_tree(&mut self, tree: f32, cx: &mut Context<Self>) {
        let Some(widened) = self.viewer.widened.filter(|widened| widened.tree) else {
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

    fn finish_tree_resize(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.tree_resizing) {
            crate::app_store::global_app_store(cx)
                .read(cx)
                .save_settings();
            cx.notify();
        }
    }

    /// Whether the view draws the file viewer area beside the tree.
    pub(super) fn shows_file_area(&self) -> bool {
        self.has_tabs()
    }

    /// Whether a root is shown or loading, for the finder to search.
    pub(super) fn searchable(&self) -> bool {
        matches!(self.body, Body::Root(_) | Body::Loading)
    }

    /// The panel widens only when a file tab is open: whether the tree toggle is on.
    pub(crate) fn widening(&self) -> Option<bool> {
        if self.has_tabs() {
            return Some(!self.viewer.tree_hidden);
        }
        if matches!(self.body, Body::Loading) {
            return self
                .pending_restore
                .as_ref()
                .filter(|saved| !saved.open_paths.is_empty())
                .map(|saved| !saved.tree_hidden);
        }
        None
    }

    pub(crate) fn has_dirty_changes(&self) -> bool {
        self.tree_ref().is_some_and(FileTree::is_dirty)
    }

    /// Authoritative live git-status dirty flag while this view is watching.
    pub(crate) fn fresh_dirty_changes(&self) -> Option<bool> {
        if !self.shown || self.status_stale {
            return None;
        }
        match &self.body {
            Body::Root(state) if state.root.git.is_some() => {
                state.tree.status().map(|status| status.changed > 0)
            }
            Body::Root(_) | Body::Choose | Body::Missing { .. } | Body::Remote => Some(false),
            Body::Loading | Body::Failed(_) => None,
        }
    }

    pub(super) fn saved_state(&self) -> Option<SavedFilesState> {
        match &self.body {
            Body::Root(state) => {
                let open_paths: Vec<String> = self
                    .viewer
                    .tabs
                    .tabs()
                    .iter()
                    .map(|tab| tab.path.clone())
                    .collect();
                let active_path = self.viewer.tabs.active().map(|tab| tab.path.clone());
                if open_paths.is_empty()
                    && !self.viewer.tree_hidden
                    && self.filter == TreeFilter::All
                {
                    return None;
                }
                Some(SavedFilesState {
                    root: state.root.path.clone(),
                    open_paths,
                    active_path,
                    tree_hidden: self.viewer.tree_hidden,
                    filter: self.filter,
                })
            }
            Body::Loading => self.pending_restore.clone(),
            _ => None,
        }
    }

    pub(super) fn restore_saved_state(&mut self, saved: SavedFilesState, cx: &mut Context<Self>) {
        if matches!(self.body, Body::Root(_)) {
            self.apply_saved_files(saved, cx);
        } else {
            self.pending_restore = Some(saved);
        }
    }

    pub(super) fn apply_saved_files(&mut self, saved: SavedFilesState, cx: &mut Context<Self>) {
        let same_root = matches!(
            &self.body,
            Body::Root(state)
                if runner_core::app_paths::same_normalized_checkout_path(
                    &state.root.path,
                    &saved.root,
                )
        );
        if !same_root {
            self.notify_owner(cx);
            cx.notify();
            return;
        }
        self.filter = saved.filter;
        self.viewer.tree_hidden = saved.tree_hidden;
        for path in &saved.open_paths {
            self.open_file(path.clone(), cx);
        }
        if let Some(active) = saved.active_path
            && let Some(index) = self.viewer.tabs.position(&active)
        {
            self.viewer.tabs.activate(index);
        }
        self.notify_owner(cx);
        cx.notify();
    }

    pub(super) fn collapse_all_folders(&mut self, cx: &mut Context<Self>) {
        if let Some(tree) = self.tree_mut()
            && tree.collapse_all()
        {
            cx.notify();
        }
    }

    pub(super) fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root_path().map(Path::to_path_buf) else {
            return;
        };
        if let Some(tree) = self.tree_ref() {
            let folders = tree.loaded_folders();
            self.load(folders, cx);
            self.reload_changed(None, cx);
        }
        self.request(
            cx,
            move |client| client.files_status(&root).map(PreparedStatus::new),
            |this, result, cx| match result {
                Ok(status) => this.apply_status(status, cx),
                Err(error) => {
                    this.status_error = Some(error.to_string().into());
                    cx.notify();
                }
            },
        );
    }

    /// Runs `call` against the daemon off the UI thread and hands its result
    /// to `then`, unless the root changed meanwhile.
    pub(super) fn request<T: Send + 'static>(
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
}

#[cfg(test)]
impl FilesView {
    /// A `files/*` event arriving from the daemon.
    pub(crate) fn receive(&mut self, event: FilesEvent, cx: &mut Context<Self>) {
        self.on_files_event(&event, cx);
    }

    pub(crate) fn git_status(&self) -> Option<runner_core::protocol::files::GitStatus> {
        self.tree_ref()?.status().cloned()
    }

    pub(crate) fn watched(&self) -> Option<&Path> {
        self.watched.as_deref()
    }

    pub(crate) fn root(&self) -> Option<&FilesRoot> {
        match &self.body {
            Body::Root(state) => Some(&state.root),
            _ => None,
        }
    }

    pub(crate) fn state(&self) -> &'static str {
        match &self.body {
            Body::Loading => "loading",
            Body::Remote => "remote",
            Body::Choose => "choose",
            Body::Missing { .. } => "missing",
            Body::Root(_) => "root",
            Body::Failed(_) => "failed",
        }
    }

    pub(crate) fn selected(&self) -> Option<String> {
        self.tree_ref()
            .and_then(|tree| tree.selected())
            .map(str::to_owned)
    }
}

impl Render for FilesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let reported = (self.widening(), self.has_tabs(), self.has_dirty_changes());
        if reported != self.reported {
            self.reported = reported;
            if let Some(owner) = self.owner {
                cx.defer(move |cx| cx.notify(owner));
            }
        }
        if self.active_showing_diff()
            && self.viewer_focus.is_focused(window)
            && let Some(diff) = self.active_diff_view()
        {
            diff.read(cx).focus_handle().clone().focus(window, cx);
        }
        let context = self.context.clone();
        let body = match &self.body {
            Body::Loading => div().flex_1().into_any_element(),
            Body::Remote => self.render_message(
                "FILES_REMOTE",
                "laptop.svg",
                crate::i18n::text("ui.files_arent_available_here"),
                crate::i18n::tf(
                    "format.files_on_another_computer",
                    &[("subject", self.noun())],
                ),
                None,
            ),
            Body::Choose => self.render_choose(cx),
            Body::Missing { path } => {
                let path = path.clone();
                self.render_missing(&path, cx)
            }
            Body::Root(_) => self.render_tree(window, cx),
            Body::Failed(error) => {
                let retry = cx.weak_entity();
                div()
                    .when(cfg!(test), |element| {
                        element.debug_selector(|| "FILES_FAILED".into())
                    })
                    .p_4()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_3()
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::danger())
                            .child(error.clone()),
                    )
                    .child(
                        Button::new("files-retry", crate::i18n::text("ui.try_again"))
                            .variant(ButtonVariant::Secondary)
                            .debug_selector("FILES_RETRY")
                            .on_press(move |_, cx| {
                                let _ = retry.update(cx, |this, cx| this.resolve(cx));
                            }),
                    )
                    .into_any_element()
            }
        };
        let header = self.render_header(cx);
        let picker = self.render_picker(window, cx);
        let finder = self.render_finder(window, cx);
        let view_bounds = self.view_bounds.clone();
        let finder_open = self.finder.is_some();
        let tree_column = div()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .flex()
            .flex_col()
            .child(header)
            .children(
                self.error
                    .clone()
                    .or(self.status_error.clone())
                    .map(|error| {
                        div()
                            .when(cfg!(test), |element| {
                                element.debug_selector(|| "FILES_ERROR".into())
                            })
                            .px_4()
                            .py_2()
                            .text_size(theme::text_meta())
                            .text_color(theme::danger())
                            .child(error)
                    }),
            )
            .child(body);
        let viewing = self.shows_file_area();
        let tree_shown = self.tree_shown();
        let tree_width = self
            .viewer
            .widened
            .map_or(crate::app_settings::FILE_TREE_DEFAULT, |widened| {
                widened.tree_width
            });
        let zoom = crate::app_store::global_app_store(cx)
            .read(cx)
            .settings
            .app_zoom;
        let tree_resize = (viewing && tree_shown).then(|| {
            resize_strip(
                "file-tree-resize",
                ResizeAxis::Columns,
                self.tree_resizing,
                zoom,
                None,
                false,
            )
            .id("file-tree-resize")
            .when(cfg!(test), |handle| {
                handle.debug_selector(|| "FILE_TREE_GUTTER".into())
            })
            .map(|handle| {
                #[cfg(any(windows, target_os = "linux"))]
                let handle = handle.occlude();
                handle
            })
            .absolute()
            .top_0()
            .right(px(tree_width * zoom - resize_strip_inset(zoom)))
            .on_drag(
                FileTreeResizeDrag,
                |drag: &FileTreeResizeDrag, _, _, cx: &mut App| cx.new(|_| drag.clone()),
            )
        });
        if self._focus_lost.is_none() {
            self._focus_lost = Some(cx.on_focus_lost(window, Self::on_focus_lost));
        }
        let popups = if viewing {
            self.render_viewer_popups(window, cx)
        } else {
            Vec::new()
        };
        div()
            .when(cfg!(test), |view| {
                view.debug_selector(|| format!("FILES_VIEW {context:?}"))
            })
            .key_context("FileViewer")
            .track_focus(&self.root_focus)
            .on_action(cx.listener(|this, _: &crate::CloseFileTab, window, cx| {
                match this.viewer.tabs.active_index() {
                    Some(index) => this.close_tab(index, window, cx),
                    None => cx.propagate(),
                }
            }))
            .on_action(cx.listener(|this, _: &crate::NextFileTab, window, cx| {
                this.cycle_tab(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::PreviousFileTab, window, cx| {
                this.cycle_tab(true, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &crate::FindInFile, window, cx| this.open_find(window, cx)),
            )
            .on_action(cx.listener(|this, _: &crate::ZoomFileIn, window, cx| {
                this.zoom(1, Box::new(crate::ZoomIn), window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::ZoomFileOut, window, cx| {
                this.zoom(-1, Box::new(crate::ZoomOut), window, cx)
            }))
            .flex_1()
            .min_h(px(0.))
            .flex()
            .when(viewing, |view| {
                view.flex_row().child(self.render_viewer(window, cx))
            })
            .when(!viewing, |view| view.flex_col())
            .when(tree_shown, |view| {
                view.child(
                    tree_column
                        .when(viewing, |column| {
                            // One step lighter than the file, which sits on
                            // the terminal's background (Jason, 2026-11-05).
                            column
                                .flex_none()
                                .w(rems(tree_width / 16.))
                                .bg(theme::panel())
                                .border_l_1()
                                .border_color(theme::border())
                                .rounded_br(rems(11. / 16.))
                        })
                        .map(|element| {
                            #[cfg(test)]
                            let element =
                                crate::theme_snapshot::record_fill("FILES_TREE_COLUMN", element);
                            element
                        }),
                )
            })
            .relative()
            .child(
                canvas(
                    move |bounds, window, _| {
                        // The finder is placed from these, a frame behind.
                        if finder_open && view_bounds.get() != bounds {
                            window.request_animation_frame();
                        }
                        view_bounds.set(bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .children(tree_resize)
            .on_drag_move::<FileTreeResizeDrag>(cx.listener(
                move |this, event: &gpui::DragMoveEvent<FileTreeResizeDrag>, _, cx| {
                    let tree = f32::from(event.bounds.right() - event.event.position.x) / zoom;
                    this.resize_tree(tree, cx);
                },
            ))
            .on_drop(cx.listener(|this, _: &FileTreeResizeDrag, _, cx| this.finish_tree_resize(cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_tree_resize(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_tree_resize(cx)),
            )
            .children(picker)
            .children(finder)
            .children(self.menu.clone())
            .children(popups)
    }
}
