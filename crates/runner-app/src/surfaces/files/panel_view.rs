//! Files panel state, initialization, focus, and right-rail tree surface (#1421).

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
    Button, ButtonVariant, IconButton, Scrollbar, TextField,
};
use runner_core::protocol::files::{
    DirEntry, FilesRoot, FilesRootOutcome, FilesSubject, WorktreeEntry, WorktreeKind,
};
use runner_core::protocol::{ClientError, DaemonClient};

use super::links::{LinkRoot, Links};
use super::open_file::{reveal, reveal_label, UnifiedViewer};
use super::picker::{self, PickerItem, PickerKind, PickerSources};
use super::tree::{FileTree, Letter, Moved, PreparedStatus, Row};
use super::{FilesEvent, FilesHub};
use crate::file_icons::{self, IconSet};
use crate::surfaces::side_panel::{PanelContext, PanelSubject, PanelSurface, Widened};
use crate::text_util::{abbreviate_home, HomePath};
use crate::theme;

/// A drag of the inner divider between the unified viewer and the right rail.
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
    pub(super) filter: TreeFilter,
}

pub(crate) struct FilesView {
    context: PanelContext,
    client: DaemonClient,
    pub(crate) viewer: Entity<UnifiedViewer>,
    pub(super) focus: FocusHandle,
    /// The whole view, so focus lost inside it can be told from elsewhere.
    pub(super) root_focus: FocusHandle,
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
    /// This rail's bounds; used as fallback when centering the finder.
    pub(super) view_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Full composed side-panel bounds, so the finder centers over viewer + rail.
    pub(crate) panel_bounds: Option<Rc<Cell<Bounds<Pixels>>>>,
    pub(super) menu: Option<Entity<ContextMenu>>,
    pub(super) scroll: UniformListScrollHandle,
    scrollbar: Entity<Scrollbar>,
    pub(super) links: Links,
    pub(super) filter: TreeFilter,
    /// Path waiting to scroll into view in the tree once its folder lists.
    pub(super) reveal: Option<String>,
    widened: Option<Widened>,
    /// Redrawn when dirty-changes state changes.
    owner: Option<gpui::EntityId>,
    reported_dirty: bool,
    /// Set when the view hides (watch stopped) and cleared when fresh status lands.
    pub(super) status_stale: bool,
    /// Filter preserved across LRU view eviction.
    pub(super) pending_restore: Option<SavedFilesState>,
    /// Settings → Side panel → Files as last drawn.
    pub(super) look: Option<super::Look>,
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
    pub(crate) fn new(
        context: PanelContext,
        viewer: Entity<UnifiedViewer>,
        cx: &mut Context<Self>,
    ) -> Self {
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
            viewer,
            focus: cx.focus_handle(),
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
            panel_bounds: None,
            menu: None,
            scroll,
            scrollbar,
            links: Links::default(),
            filter: TreeFilter::All,
            reveal: None,
            widened: None,
            owner: None,
            reported_dirty: false,
            status_stale: false,
            pending_restore: None,
            look: super::Look::current(cx),
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

    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let look = super::Look::current(cx);
        if look != self.look {
            self.look = look;
            cx.notify();
        }
    }

    fn on_focus_lost(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ours = window
            .focus_lost_restore_target(cx)
            .is_some_and(|target| target == self.root_focus);
        if ours {
            self.focus_tree(window, cx);
        }
    }

    pub(crate) fn set_widened(&mut self, widened: Option<Widened>, cx: &mut Context<Self>) {
        if self.widened != widened {
            self.widened = widened;
            cx.notify();
        }
    }

    pub(crate) fn set_panel_bounds(&mut self, bounds: Option<Rc<Cell<Bounds<Pixels>>>>) {
        self.panel_bounds = bounds;
    }

    pub(crate) fn focus_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.body, Body::Root(_)) {
            self.focus.focus(window, cx);
        } else {
            self.root_focus.focus(window, cx);
        }
    }

    pub(crate) fn focus_default(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.root_focus.contains_focused(window, cx) {
            return;
        }
        self.focus_tree(window, cx);
    }

    pub(crate) fn set_owner(&mut self, owner: Option<gpui::EntityId>) {
        self.owner = owner;
    }

    pub(super) fn notify_owner(&self, cx: &mut Context<Self>) {
        if let Some(owner) = self.owner {
            cx.defer(move |cx| cx.notify(owner));
        }
    }

    /// Whether a root is shown or loading, for the finder to search.
    pub(super) fn searchable(&self) -> bool {
        matches!(self.body, Body::Root(_) | Body::Loading)
    }

    pub(crate) fn open_file(&mut self, path: String, cx: &mut Context<Self>) {
        self.viewer
            .update(cx, |viewer, cx| viewer.open_pinned_file(path, None, cx));
    }

    pub(crate) fn open_cited(&mut self, path: String, line: Option<u32>, cx: &mut Context<Self>) {
        self.viewer
            .update(cx, |viewer, cx| viewer.open_pinned_file(path, line, cx));
    }

    pub(crate) fn reveal_in_tree(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(tree) = self.tree_mut() else {
            return;
        };
        let moved = tree.reveal(path);
        self.reveal = Some(path.to_owned());
        self.after_move(moved, cx);
        self.after_listing(cx);
    }

    pub(super) fn after_listing(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.reveal.clone() else {
            return;
        };
        let Some(index) = self
            .tree_ref()
            .and_then(|tree| tree.rows().iter().position(|row| row.path == path))
        else {
            return;
        };
        self.reveal = None;
        self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        cx.notify();
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
                if self.filter == TreeFilter::All {
                    return None;
                }
                Some(SavedFilesState {
                    root: state.root.path.clone(),
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
            cx.notify();
            return;
        }
        self.filter = saved.filter;
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
            self.viewer
                .update(cx, |viewer, cx| viewer.reload_changed(None, cx));
        }
        self.request(
            cx,
            move |client| client.files_status(&root).map(PreparedStatus::new),
            |this, result, cx| match result {
                Ok(status) => this.apply_status(status, true, cx),
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
    /// A `files/*` event arriving from the daemon; forwarded to both the rail
    /// and the shared viewer for unit tests that inject events directly.
    pub(crate) fn receive(&mut self, event: FilesEvent, cx: &mut Context<Self>) {
        self.on_files_event(&event, cx);
        self.viewer
            .update(cx, |viewer, cx| viewer.on_files_event(&event, cx));
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

    pub(crate) fn has_tabs(&self, cx: &App) -> bool {
        self.viewer.read(cx).has_tabs()
    }

    pub(crate) fn widening(&self, cx: &App) -> Option<bool> {
        self.viewer.read(cx).widening()
    }

    pub(crate) fn has_uncommitted_diff(&self, path: &str, cx: &App) -> Option<bool> {
        self.viewer.read(cx).has_uncommitted_diff(path)
    }

    pub(crate) fn open_tabs(&self, cx: &App) -> Vec<String> {
        self.viewer.read(cx).open_tabs()
    }

    pub(crate) fn active_tab(&self, cx: &App) -> Option<String> {
        self.viewer.read(cx).active_tab()
    }

    pub(crate) fn code_view(&self, cx: &App) -> Option<Entity<super::code::CodeView>> {
        self.viewer.read(cx).code_view()
    }

    pub(crate) fn showing_diff(&self, cx: &App) -> bool {
        self.viewer.read(cx).showing_diff()
    }

    pub(crate) fn tab_diff_view(
        &self,
        cx: &App,
    ) -> Option<Entity<crate::surfaces::changes::UnifiedDiffView>> {
        self.viewer.read(cx).tab_diff_view()
    }

    pub(crate) fn read_seq(&self, path: &str, cx: &App) -> Option<u64> {
        self.viewer.read(cx).read_seq(path)
    }

    pub(crate) fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.viewer
            .update(cx, |v, cx| v.close_tab(index, window, cx));
    }

    pub(crate) fn reply_text(&mut self, path: &str, read: u64, text: &str, cx: &mut Context<Self>) {
        self.viewer
            .update(cx, |v, cx| v.reply_text(path, read, text, cx));
    }

    pub(crate) fn reading(&self, path: &str, cx: &App) -> bool {
        self.viewer.read(cx).reading(path)
    }

    pub(crate) fn last_search(&self, cx: &App) -> Option<u64> {
        self.viewer.read(cx).last_search()
    }

    pub(crate) fn reply_search(&mut self, search: u64, count: usize, cx: &mut Context<Self>) {
        self.viewer
            .update(cx, |v, cx| v.reply_search(search, count, cx));
    }

    pub(crate) fn reply_binary(
        &mut self,
        path: &str,
        read: u64,
        size: u64,
        cx: &mut Context<Self>,
    ) {
        self.viewer
            .update(cx, |v, cx| v.reply_binary(path, read, size, cx));
    }

    pub(crate) fn hold_search(&mut self, hold: bool, cx: &mut Context<Self>) {
        self.viewer.update(cx, |v, cx| v.hold_search(hold, cx));
    }

    pub(crate) fn step(&mut self, next: bool, cx: &mut Context<Self>) {
        self.viewer.update(cx, |v, cx| v.step(next, cx));
    }

    pub(crate) fn launched(
        &self,
        cx: &App,
    ) -> Vec<(
        Option<crate::app_settings::FileLinkEditor>,
        PathBuf,
        Option<u32>,
    )> {
        self.viewer.read(cx).launched()
    }

    pub(crate) fn rendered_view(&self, cx: &App) -> Option<Entity<super::markdown::MarkdownView>> {
        self.viewer.read(cx).rendered_view()
    }

    pub(crate) fn opened_urls(&self, cx: &App) -> Vec<String> {
        self.viewer.read(cx).opened_urls()
    }

    pub(crate) fn follow_link(&mut self, href: &str, cx: &mut Context<Self>) {
        self.viewer.update(cx, |v, cx| v.follow_link(href, cx));
    }

    pub(crate) fn find_state(&self, cx: &App) -> Option<(Option<String>, Option<usize>)> {
        self.viewer.read(cx).find_state()
    }
}

impl Render for FilesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dirty = self.has_dirty_changes();
        if dirty != self.reported_dirty {
            self.reported_dirty = dirty;
            self.notify_owner(cx);
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
        let viewing = self.viewer.read(cx).has_tabs();
        if self._focus_lost.is_none() {
            self._focus_lost = Some(cx.on_focus_lost(window, Self::on_focus_lost));
        }
        div()
            .when(cfg!(test), |view| {
                view.debug_selector(|| format!("FILES_VIEW {context:?}"))
            })
            .key_context("FileViewer")
            .track_focus(&self.root_focus)
            .on_action(cx.listener(|this, _: &crate::CloseFileTab, window, cx| {
                let active = this.viewer.read(cx).tabs.active_index();
                match active {
                    Some(index) => {
                        this.viewer
                            .update(cx, |viewer, cx| viewer.close_tab(index, window, cx));
                    }
                    None => cx.propagate(),
                }
            }))
            .on_action(cx.listener(|this, _: &crate::NextFileTab, window, cx| {
                if this.viewer.read(cx).has_tabs() {
                    this.viewer
                        .update(cx, |viewer, cx| viewer.cycle_tab(false, window, cx));
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &crate::PreviousFileTab, window, cx| {
                if this.viewer.read(cx).has_tabs() {
                    this.viewer
                        .update(cx, |viewer, cx| viewer.cycle_tab(true, window, cx));
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &crate::FindInFile, window, cx| {
                if this.viewer.read(cx).has_tabs() {
                    this.viewer
                        .update(cx, |viewer, cx| viewer.open_find(window, cx));
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|_, _: &crate::ZoomFileIn, window, cx| {
                window.dispatch_action(Box::new(crate::ZoomIn), cx);
            }))
            .on_action(cx.listener(|_, _: &crate::ZoomFileOut, window, cx| {
                window.dispatch_action(Box::new(crate::ZoomOut), cx);
            }))
            .relative()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .when(viewing, |column| column.bg(theme::panel()))
                    .map(|element| {
                        #[cfg(test)]
                        let element =
                            crate::theme_snapshot::record_fill("FILES_TREE_COLUMN", element);
                        element
                    })
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
                    .child(body),
            )
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
            .children(picker)
            .children(finder)
            .children(self.menu.clone())
    }
}
