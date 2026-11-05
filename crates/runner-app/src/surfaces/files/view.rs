use std::cell::Cell;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    canvas, div, img, px, rems, svg, AnyElement, App, Bounds, ClipboardItem, Context, Entity,
    EventEmitter, FocusHandle, FontWeight, ImageSource, KeyDownEvent, MouseButton, MouseDownEvent,
    PathPromptOptions, Pixels, Resource, ScrollHandle, ScrollStrategy, SharedString, Subscription,
    Task, UniformListScrollHandle, Window,
};
use runner_app::ui::{
    menu::{popup_layer_sized, ContextMenu, MenuItem, PopupWidth},
    resize::{resize_strip, resize_strip_inset, ResizeAxis},
    Button, ButtonVariant, IconButton, Scrollbar, TextField,
};
use runner_core::protocol::files::{
    DirEntry, FilesRoot, FilesRootOutcome, FilesSubject, GitStatus, WorktreeEntry, WorktreeKind,
};
use runner_core::protocol::{ClientError, DaemonClient};

use super::picker::{self, PickerItem, PickerKind, PickerSources};
use super::tree::{FileTree, Letter, Moved, Row};
use super::viewer::Viewer;
use super::{FilesEvent, FilesHub};
use crate::file_icons::{self, IconSet};
use crate::surfaces::side_panel::{PanelContext, PanelSubject, PanelSurface, Widened};
use crate::theme;

const ROW_HEIGHT: f32 = 26.;
const INDENT: f32 = 14.;
const PICKER_WIDTH: f32 = 300.;

/// Space or Enter on a file in the tree: the hook the file viewer (m4)
/// subscribes to, opening a preview tab for Space and a kept tab for Enter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileOpen {
    pub(crate) root: PathBuf,
    pub(crate) path: String,
    pub(crate) keep: bool,
}

type Listings = Vec<(String, Result<Vec<DirEntry>, ClientError>)>;

/// A drag of the divider between an open file and the tree.
#[derive(Clone)]
struct FileTreeResizeDrag;

impl Render for FileTreeResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1.)).h(px(1.))
    }
}
type MenuAction = Box<dyn Fn(&mut FilesView, &mut Window, &mut Context<FilesView>)>;

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
    scroll: ScrollHandle,
    /// Scroll the highlighted item into view on the next frame.
    reveal: Cell<bool>,
    _observe: Subscription,
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
    shown: bool,
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
    recent: Vec<PathBuf>,
    pub(super) choose_field: Entity<TextField>,
    pub(super) picker: Option<Picker>,
    picker_anchor: Rc<Cell<Bounds<Pixels>>>,
    menu: Option<Entity<ContextMenu>>,
    pub(super) scroll: UniformListScrollHandle,
    scrollbar: Entity<Scrollbar>,
    pub(super) viewer: Viewer,
    /// Redrawn when the panel's width should change.
    owner: Option<gpui::EntityId>,
    reported: Option<bool>,
    /// Whether the divider between the file and the tree is being dragged.
    tree_resizing: bool,
    /// Settings → Side panel → Files as last drawn.
    pub(super) look: Option<super::Look>,
    _hub: Subscription,
    _settings: Subscription,
}

impl EventEmitter<FileOpen> for FilesView {}

fn subject(context: &PanelContext) -> FilesSubject {
    match &context.subject {
        PanelSubject::Session(id) => FilesSubject::Session(id.clone()),
        PanelSubject::Mission(id) => FilesSubject::Mission(id.clone()),
    }
}

/// `relative` (`/`-separated) inside `root`, as a path on this computer.
pub(crate) fn absolute(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

pub(super) fn letter_color(letter: Letter) -> gpui::Hsla {
    match letter {
        Letter::Modified => theme::warning(),
        Letter::Added | Letter::Untracked => theme::accent(),
        Letter::Renamed => theme::info(),
        Letter::Unmerged => theme::danger(),
    }
}

pub(super) fn reveal_label() -> &'static str {
    crate::i18n::text(if cfg!(target_os = "macos") {
        "ui.reveal_in_finder"
    } else if cfg!(windows) {
        "ui.show_in_explorer"
    } else {
        "ui.open_containing_folder"
    })
}

/// Reveals `path` in the file manager; on Linux, which has no common call
/// that selects a file, opens the folder holding it.
pub(super) fn reveal(path: PathBuf) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut select = std::ffi::OsString::from("/select,");
        select.push(&path);
        std::process::Command::new("explorer.exe")
            .arg(select)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
    #[cfg(not(windows))]
    {
        #[cfg(target_os = "macos")]
        let status = std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .status();
        #[cfg(target_os = "linux")]
        let status = std::process::Command::new("xdg-open")
            .arg(path.parent().unwrap_or(&path))
            .status();
        match status {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(format!("exited with {status}")),
            Err(error) => Err(error.to_string()),
        }
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
            recent: Vec::new(),
            choose_field,
            picker: None,
            picker_anchor: Rc::default(),
            menu: None,
            scroll,
            scrollbar,
            viewer: Viewer::default(),
            owner: None,
            reported: None,
            tree_resizing: false,
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
        let remote_changed = self.context.remote != context.remote;
        self.context = context;
        if remote_changed {
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
            cx.notify();
        }
    }

    pub(crate) fn set_owner(&mut self, owner: Option<gpui::EntityId>) {
        self.owner = owner;
    }

    /// Sizes the tree `tree` wide from the divider's drag, trading width
    /// with the file so the panel keeps its width.
    fn resize_tree(&mut self, tree: f32, cx: &mut Context<Self>) {
        let Some(widened) = self.viewer.widened.filter(|widened| widened.tree) else {
            return;
        };
        self.tree_resizing = true;
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

    /// With a file open, the panel widens: whether the tree toggle is on.
    pub(crate) fn widening(&self) -> Option<bool> {
        self.has_tabs().then_some(!self.viewer.tree_hidden)
    }

    pub(crate) fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown == shown {
            return;
        }
        self.shown = shown;
        if !shown {
            self.picker = None;
            self.menu = None;
        }
        self.sync_watch(cx);
    }

    pub(super) fn root_path(&self) -> Option<&Path> {
        match &self.body {
            Body::Root(state) => Some(&state.root.path),
            _ => None,
        }
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

    fn resolve(&mut self, cx: &mut Context<Self>) {
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

    /// Saves `path` as the root. A path the daemon refuses leaves the view as
    /// it was, with the error shown, so another path can be tried.
    pub(super) fn set_root(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.context.remote {
            return;
        }
        self.picker = None;
        self.root_change += 1;
        let change = self.root_change;
        let subject = subject(&self.context);
        let client = self.client.clone();
        let previous = self.root_chain.take();
        #[cfg(test)]
        let gate = self.root_gate.take();
        let (reply, result) = futures::channel::oneshot::channel();
        self.root_chain = Some(cx.background_spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            #[cfg(test)]
            if let Some(gate) = gate {
                let _ = gate.await;
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
                    Ok(outcome) => this.apply_outcome(Ok(outcome), cx),
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
        self.generation += 1;
        self.error = None;
        match outcome {
            Ok(FilesRootOutcome::Root(root)) => {
                let same = self.root_path() == Some(root.path.as_path());
                match &mut self.body {
                    Body::Root(state) if same => state.root = root,
                    _ => {
                        self.reset_viewer();
                        let tree = FileTree::new(root.cwd.clone());
                        let loads = tree.initial_loads();
                        self.body = Body::Root(Box::new(RootState {
                            root,
                            tree,
                            listed: Default::default(),
                            next_listing: 0,
                        }));
                        self.scroll = UniformListScrollHandle::new();
                        let owner = cx.entity_id();
                        let base = self.scroll.0.borrow().base_handle.clone();
                        self.scrollbar = cx.new(|_| Scrollbar::app(base, owner));
                        self.load(loads, cx);
                    }
                }
            }
            Ok(FilesRootOutcome::ChooseFolder) => {
                self.reset_viewer();
                self.body = Body::Choose;
                self.load_recent(cx);
            }
            Ok(FilesRootOutcome::Missing { path }) => {
                self.reset_viewer();
                self.body = Body::Missing { path };
            }
            Err(error) => {
                self.reset_viewer();
                self.body = Body::Failed(error.to_string().into());
            }
        }
        self.watched_root_changed(cx);
        cx.notify();
    }

    fn watched_root_changed(&mut self, cx: &mut Context<Self>) {
        if self.watched.as_deref() != self.root_path() {
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

    /// Watches the root while the view is shown, and reads its status once
    /// watching, since a root someone else already watches reports none.
    fn sync_watch(&mut self, cx: &mut Context<Self>) {
        let wanted = self
            .shown
            .then(|| self.root_path().map(Path::to_path_buf))
            .flatten();
        if self.watched == wanted {
            return;
        }
        self.unwatch(cx);
        let Some(root) = wanted else {
            return;
        };
        self.watched = Some(root.clone());
        // Changes made while unwatched never arrived, so list and read again.
        if let Some(tree) = self.tree_ref() {
            let folders = tree.loaded_folders();
            self.load(folders, cx);
            self.reload_changed(None, cx);
        }
        let (status_tx, status_rx) = futures::channel::oneshot::channel();
        self.chain_watch(cx, move |client| {
            let _ = status_tx.send(
                client
                    .files_watch(&root)
                    .and_then(|()| client.files_status(&root)),
            );
        });
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let Ok(result) = status_rx.await else { return };
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(status) => this.apply_status(status, cx),
                    Err(error) => {
                        this.error = Some(error.to_string().into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn apply_status(&mut self, status: GitStatus, cx: &mut Context<Self>) {
        if let Body::Root(state) = &mut self.body {
            state.tree.set_status(status);
            cx.notify();
        }
    }

    fn load(&mut self, folders: Vec<String>, cx: &mut Context<Self>) {
        let Body::Root(state) = &mut self.body else {
            return;
        };
        if folders.is_empty() {
            return;
        }
        let root = state.root.path.clone();
        state.next_listing += 1;
        let sequence = state.next_listing;
        self.request(
            cx,
            move |client| {
                Ok(folders
                    .into_iter()
                    .map(|folder| {
                        let listing = client.files_list(&root, &folder);
                        (folder, listing)
                    })
                    .collect::<Vec<_>>())
            },
            move |this, result: Result<Listings, _>, cx| {
                let Ok(listings) = result else { return };
                let mut root_gone = false;
                if let Body::Root(state) = &mut this.body {
                    for (folder, listing) in listings {
                        if state
                            .listed
                            .get(&folder)
                            .is_some_and(|newer| *newer > sequence)
                        {
                            continue;
                        }
                        state.listed.insert(folder.clone(), sequence);
                        match listing {
                            Ok(entries) => state.tree.set_listing(&folder, entries),
                            Err(_) if folder.is_empty() => root_gone = true,
                            Err(_) => state.tree.drop_listing(&folder),
                        }
                    }
                }
                if root_gone {
                    this.resolve(cx);
                }
                this.after_listing(cx);
                cx.notify();
            },
        );
    }

    fn load_recent(&mut self, cx: &mut Context<Self>) {
        self.request(
            cx,
            |client| client.files_recent_folders(),
            |this, result, cx| {
                if let Ok(recent) = result {
                    this.recent = recent;
                    this.rebuild_picker(cx);
                    cx.notify();
                }
            },
        );
    }

    fn on_files_event(&mut self, event: &FilesEvent, cx: &mut Context<Self>) {
        match event {
            FilesEvent::Changed(changed) => {
                if self.root_path() != Some(changed.root.as_path()) {
                    return;
                }
                let Body::Root(state) = &self.body else {
                    return;
                };
                let folders = state.tree.listed(&changed.folders);
                self.load(folders, cx);
                self.reload_changed(Some(changed), cx);
            }
            FilesEvent::Status(update) => {
                if self.root_path() == Some(update.root.as_path()) {
                    self.apply_status(update.status.clone(), cx);
                }
            }
            FilesEvent::Resync { rewatch } => {
                if self.context.remote {
                    return;
                }
                let Body::Root(state) = &self.body else {
                    self.resolve(cx);
                    return;
                };
                let folders = state.tree.loaded_folders();
                if *rewatch {
                    // The daemon forgot its watches; nothing to release.
                    self.watched = None;
                    self.sync_watch(cx);
                } else if let Some(root) = self.watched.clone() {
                    self.request(
                        cx,
                        move |client| client.files_status(&root),
                        |this, result, cx| {
                            if let Ok(status) = result {
                                this.apply_status(status, cx);
                            }
                        },
                    );
                }
                self.load(folders, cx);
                self.reload_changed(None, cx);
            }
        }
    }

    /// "chat", "terminal" or "mission", for the copy that names the subject.
    fn noun(&self) -> String {
        crate::i18n::text(match self.context.surface {
            PanelSurface::Chat => "noun.chat",
            PanelSurface::Terminal => "noun.terminal",
            PanelSurface::Mission => "noun.mission",
        })
        .to_owned()
    }

    fn projects(&self, cx: &App) -> Vec<(String, PathBuf)> {
        crate::app_store::global_app_store(cx)
            .read(cx)
            .projects
            .iter()
            .filter(|project| !project.cwd.trim().is_empty())
            .map(|project| (project.name.clone(), PathBuf::from(&project.cwd)))
            .collect()
    }

    fn own_project(&self, cx: &App) -> Option<(String, PathBuf)> {
        let id = self.context.project.as_deref()?;
        crate::app_store::global_app_store(cx)
            .read(cx)
            .projects
            .iter()
            .find(|project| project.id == id && !project.cwd.trim().is_empty())
            .map(|project| (project.name.clone(), PathBuf::from(&project.cwd)))
    }

    fn home(cx: &App) -> Option<PathBuf> {
        crate::app_store::global_app_store(cx)
            .read(cx)
            .home_dir
            .clone()
    }

    fn apply_typed_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.choose_field.read(cx).text().to_owned();
        match picker::typed_path(&text, Self::home(cx).as_deref()) {
            Some(path) => {
                self.set_root(Some(path), cx);
                self.focus.focus(window, cx);
            }
            None => {
                self.error = Some(crate::i18n::text("ui.type_a_full_path").into());
                cx.notify();
            }
        }
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        self.picker = None;
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
        cx.notify();
    }

    // ── The picker ─────────────────────────────────────────────────────

    fn picker_base(&self, cx: &App) -> Option<PathBuf> {
        match &self.body {
            Body::Root(state) => Some(state.root.path.clone()),
            Body::Missing { .. } => self.own_project(cx).map(|(_, path)| path),
            _ => None,
        }
    }

    fn toggle_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
            return;
        }
        let root = cx.weak_entity();
        let field = cx.new(move |field_cx| {
            let mut field = TextField::new(
                field_cx.focus_handle(),
                "",
                crate::i18n::message("ui.select_a_worktree_or_type_a_path"),
                false,
            )
            .text_size(theme::text_ui())
            .key_interceptor(Rc::new(move |event, window, cx| {
                root.update(cx, |this, cx| this.on_picker_key(event, window, cx))
                    .unwrap_or(false)
            }));
            field.set_bare(true, field_cx);
            field
        });
        let observe = cx.observe(&field, |this, field, cx| {
            let query = field.read(cx).text().to_owned();
            if let Some(picker) = &mut this.picker {
                if picker.query != query {
                    picker.query = query;
                    picker.highlighted = 0;
                    picker.reveal.set(true);
                    cx.notify();
                }
            }
        });
        field.read(cx).focus_handle().focus(window, cx);
        self.picker = Some(Picker {
            field,
            query: String::new(),
            worktrees: Vec::new(),
            items: Vec::new(),
            highlighted: 0,
            scroll: ScrollHandle::new(),
            reveal: Cell::new(false),
            _observe: observe,
        });
        self.rebuild_picker(cx);
        self.load_recent(cx);
        if let Some(base) = self.picker_base(cx) {
            self.request(
                cx,
                move |client| client.files_worktrees(&base),
                |this, result, cx| {
                    if let (Some(picker), Ok(worktrees)) = (&mut this.picker, result) {
                        picker.worktrees = worktrees;
                        this.rebuild_picker(cx);
                        cx.notify();
                    }
                },
            );
        }
        cx.notify();
    }

    fn rebuild_picker(&mut self, cx: &mut Context<Self>) {
        let current = match &self.body {
            Body::Root(state) => state.root.path.clone(),
            Body::Missing { path } => path.clone(),
            _ => PathBuf::new(),
        };
        let projects = if self.context.project.is_none() {
            self.projects(cx)
        } else {
            Vec::new()
        };
        let home = Self::home(cx);
        let Some(picker) = &mut self.picker else {
            return;
        };
        picker.items = picker::picker_items(PickerSources {
            current: &current,
            worktrees: &picker.worktrees,
            recent: &self.recent,
            projects: &projects,
            home: home.as_deref(),
            main_checkout_label: crate::i18n::text("ui.main_checkout"),
            detached_label: crate::i18n::text("ui.detached"),
            choose_folder_label: crate::i18n::text("ui.choose_folder"),
        });
        let visible = picker::filter(&picker.items, &picker.query).len();
        picker.highlighted = picker.highlighted.min(visible.saturating_sub(1));
    }

    fn visible_picker_items(&self) -> Vec<usize> {
        self.picker
            .as_ref()
            .map(|picker| picker::filter(&picker.items, &picker.query))
            .unwrap_or_default()
    }

    fn on_picker_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let visible = self.visible_picker_items();
        let Some(picker) = &mut self.picker else {
            return false;
        };
        match event.keystroke.key.as_str() {
            "down" => {
                picker.highlighted = (picker.highlighted + 1).min(visible.len().saturating_sub(1));
                picker.reveal.set(true);
            }
            "up" => {
                picker.highlighted = picker.highlighted.saturating_sub(1);
                picker.reveal.set(true);
            }
            "escape" => {
                self.picker = None;
                self.focus.focus(window, cx);
            }
            "enter" => {
                let typed = picker::typed_path(&picker.query, Self::home(cx).as_deref());
                match typed {
                    Some(path) => self.set_root(Some(path), cx),
                    None => {
                        if let Some(index) = visible.get(picker.highlighted).copied() {
                            self.activate_picker_item(index, window, cx);
                        }
                    }
                }
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    fn activate_picker_item(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self
            .picker
            .as_ref()
            .and_then(|picker| picker.items.get(index))
            .cloned()
        else {
            return;
        };
        match item.path {
            Some(path) => {
                self.set_root(Some(path), cx);
                self.focus.focus(window, cx);
            }
            None => self.browse(cx),
        }
    }

    // ── The tree ───────────────────────────────────────────────────────

    pub(super) fn tree_mut(&mut self) -> Option<&mut FileTree> {
        match &mut self.body {
            Body::Root(state) => Some(&mut state.tree),
            _ => None,
        }
    }

    pub(super) fn after_move(&mut self, moved: Moved, cx: &mut Context<Self>) {
        if let Some(index) = self
            .tree_mut()
            .and_then(|tree| tree.selected_index())
            .filter(|_| moved.changed)
        {
            self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        }
        self.load(moved.load, cx);
        cx.notify();
    }

    /// A click selects the row and opens a folder or the file's preview tab;
    /// a double-click on a file keeps its tab.
    fn click_row(&mut self, row: &Row, clicks: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let Some(tree) = self.tree_mut() else { return };
        tree.select(&row.path);
        let moved = if row.folder {
            if clicks >= 2 {
                return;
            }
            tree.toggle(&row.path)
        } else {
            Moved::default()
        };
        self.after_move(moved, cx);
        if !row.folder {
            self.open_file(row.path.clone(), clicks >= 2, cx);
        }
    }

    fn on_tree_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt || modifiers.function {
            return;
        }
        let Body::Root(state) = &mut self.body else {
            return;
        };
        let tree = &mut state.tree;
        let moved = match event.keystroke.key.as_str() {
            "down" => tree.move_down(),
            "up" => tree.move_up(),
            "right" => tree.move_right(),
            "left" => tree.move_left(),
            key @ ("space" | "enter") => {
                let keep = key == "enter";
                let selected = tree
                    .selected_index()
                    .map(|index| tree.rows()[index].clone());
                match selected {
                    Some(row) if row.folder => tree.toggle(&row.path),
                    Some(row) => {
                        cx.emit(FileOpen {
                            root: state.root.path.clone(),
                            path: row.path.clone(),
                            keep,
                        });
                        cx.stop_propagation();
                        self.open_file(row.path, keep, cx);
                        return;
                    }
                    None => Moved::default(),
                }
            }
            _ => match event.keystroke.key_char.as_deref() {
                Some(text) if !text.trim().is_empty() => tree.type_ahead(text, Instant::now()),
                _ => return,
            },
        };
        cx.stop_propagation();
        self.after_move(moved, cx);
    }

    fn open_row_menu(
        &mut self,
        row: Row,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.root_path().map(Path::to_path_buf) else {
            return;
        };
        if let Some(tree) = self.tree_mut() {
            tree.select(&row.path);
        }
        let path = absolute(&root, &row.path);
        let mut items = Vec::new();
        let mut actions: Vec<MenuAction> = Vec::new();
        if row.folder {
            items.push(
                MenuItem::new(crate::i18n::message("ui.show_files_from_here"))
                    .icon("folder-tree.svg"),
            );
            let folder = path.clone();
            actions.push(Box::new(move |this, _, cx| {
                this.set_root(Some(folder.clone()), cx)
            }));
        }
        items.push(MenuItem::new(crate::i18n::message("ui.copy_path")).icon("copy.svg"));
        let copied = path.clone();
        actions.push(Box::new(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(
                copied.to_string_lossy().into_owned(),
            ))
        }));
        items.push(MenuItem::new(reveal_label()).icon("app-window.svg"));
        actions.push(Box::new(move |_, _, cx| {
            let path = path.clone();
            let task = cx.background_spawn(async move { reveal(path) });
            cx.spawn(async move |this, cx| {
                if let Err(error) = task.await {
                    let _ = this.update(cx, |this, cx| {
                        this.error = Some(error.into());
                        cx.notify();
                    });
                }
            })
            .detach();
        }));
        let actions = Rc::new(actions);
        let root = cx.weak_entity();
        let dismiss_root = root.clone();
        let return_focus = self.focus.clone();
        let menu = cx.new(move |menu_cx| {
            ContextMenu::new(
                "files-row-menu",
                menu_cx.focus_handle(),
                event.position,
                items,
                Rc::new(move |index, window, cx| {
                    let actions = actions.clone();
                    let _ = root.update(cx, |this, cx| {
                        if let Some(action) = actions.get(index) {
                            action(this, window, cx);
                        }
                    });
                }),
                Rc::new(move |_, cx| {
                    let _ = dismiss_root.update(cx, |this, cx| {
                        this.menu = None;
                        cx.notify();
                    });
                }),
            )
            .return_focus(Some(return_focus))
        });
        menu.read(cx).focus_handle().focus(window, cx);
        self.menu = Some(menu);
        cx.notify();
    }

    // ── Rendering ──────────────────────────────────────────────────────

    pub(super) fn file_icon(&self, name: &str) -> AnyElement {
        self.icon(&Row {
            path: String::new(),
            name: name.to_owned(),
            depth: 0,
            folder: false,
            expanded: false,
            ignored: false,
            letter: None,
            dot: false,
            cwd: false,
        })
    }

    fn icon(&self, row: &Row) -> AnyElement {
        let dark = !theme::active_variant().is_light();
        let set = self
            .look
            .as_ref()
            .map_or_else(IconSet::default, |look| look.icons);
        let icon = if row.folder {
            file_icons::folder_icon(set, dark, &row.name, row.expanded)
        } else {
            file_icons::file_icon(set, dark, &row.name)
        };
        let opacity = if row.ignored { 0.5 } else { 1. };
        match icon {
            Some(icon) if icon.monochrome => svg()
                .flex_none()
                .size(px(16.))
                .path(icon.path)
                .text_color(if row.ignored {
                    theme::faint()
                } else {
                    theme::muted()
                })
                .into_any_element(),
            Some(icon) => img(ImageSource::Resource(Resource::Embedded(icon.path)))
                .flex_none()
                .size(px(16.))
                .opacity(opacity)
                .into_any_element(),
            None => svg()
                .flex_none()
                .size(px(16.))
                .path(if row.folder {
                    "folder.svg"
                } else {
                    "file-text.svg"
                })
                .text_color(theme::muted())
                .into_any_element(),
        }
    }

    fn render_row(
        &self,
        index: usize,
        row: Row,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self
            .tree_ref()
            .and_then(|tree| tree.selected())
            .is_some_and(|selected| selected == row.path);
        let click_row = row.clone();
        let menu_row = row.clone();
        let chevron = if row.folder {
            svg()
                .flex_none()
                .size(px(12.))
                .path(if row.expanded {
                    "chevron-down.svg"
                } else {
                    "chevron-right.svg"
                })
                .text_color(theme::faint())
                .into_any_element()
        } else {
            div().flex_none().size(px(12.)).into_any_element()
        };
        let name_color = if row.ignored {
            theme::faint()
        } else {
            theme::text()
        };
        let path = row.path.clone();
        div()
            .id(("files-row", index))
            .when(cfg!(test), |element| {
                let path = path.clone();
                element.debug_selector(move || format!("FILES_ROW {path}"))
            })
            .h(px(ROW_HEIGHT))
            .mx(px(8.))
            .pl(px(6. + row.depth as f32 * INDENT))
            .pr(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded_md()
            .border_1()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .text_size(theme::text_ui())
            .when(selected, |element| element.bg(theme::sidebar_selected()))
            .when(selected && focused, |element| {
                element.border_color(theme::faint())
            })
            .when(!selected, |element| {
                element
                    .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
            })
            .map(|element| {
                #[cfg(test)]
                let element = if selected {
                    crate::theme_snapshot::record_border(
                        "FILES_ROW_SELECTED",
                        crate::theme_snapshot::record_fill("FILES_ROW_SELECTED", element),
                    )
                } else {
                    element
                };
                element
            })
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    this.click_row(&click_row, event.click_count(), window, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_row_menu(menu_row.clone(), event, window, cx)
                }),
            )
            .child(chevron)
            .child(self.icon(&row))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_color(name_color)
                    .map(|element| {
                        #[cfg(test)]
                        let element = if row.ignored {
                            crate::theme_snapshot::record_text("FILES_ROW_IGNORED", element)
                        } else {
                            element
                        };
                        element
                    })
                    .child(row.name.clone()),
            )
            .when(row.cwd, |element| {
                element.child(
                    div()
                        .flex_none()
                        .px(px(5.))
                        .rounded_sm()
                        .border_1()
                        .border_color(theme::border())
                        .text_size(theme::text_micro())
                        .text_color(theme::muted())
                        .when(cfg!(test), |tag| {
                            tag.debug_selector(|| "FILES_CWD_TAG".into())
                        })
                        .child("cwd"),
                )
            })
            .when_some(row.letter, |element, letter| {
                let mark = div()
                    .text_color(letter_color(letter))
                    .map(|element| {
                        #[cfg(test)]
                        let element = {
                            let id = match letter {
                                Letter::Modified => "FILES_LETTER M",
                                Letter::Added => "FILES_LETTER A",
                                Letter::Renamed => "FILES_LETTER R",
                                Letter::Untracked => "FILES_LETTER U",
                                Letter::Unmerged => "FILES_LETTER !",
                            };
                            crate::theme_snapshot::record_text(id, element)
                        };
                        element
                    })
                    .child(letter.text());
                element.child(
                    div()
                        .flex_none()
                        .w(px(12.))
                        .text_center()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_meta())
                        .when(cfg!(test), |element| {
                            let path = path.clone();
                            element.debug_selector(move || format!("FILES_LETTER {path}"))
                        })
                        .child(mark),
                )
            })
            .when(row.dot, |element| {
                element.child(
                    div()
                        .flex_none()
                        .w(px(12.))
                        .flex()
                        .justify_center()
                        .child(div().size(px(6.)).rounded_full().bg(theme::warning()))
                        .when(cfg!(test), |element| {
                            let path = path.clone();
                            element.debug_selector(move || format!("FILES_DOT {path}"))
                        }),
                )
            })
            .into_any_element()
    }

    pub(super) fn tree_ref(&self) -> Option<&FileTree> {
        match &self.body {
            Body::Root(state) => Some(&state.tree),
            _ => None,
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let (name, icon, missing) = match &self.body {
            Body::Root(state) => (
                state.root.name.clone(),
                match &state.root.git {
                    Some(git) if git.worktree == WorktreeKind::Linked => "git-branch.svg",
                    Some(_) => "folder-git-2.svg",
                    None => "folder.svg",
                },
                false,
            ),
            Body::Missing { path } => (picker::folder_name(path), "git-branch.svg", true),
            _ => return div().into_any_element(),
        };
        let anchor = self.picker_anchor.clone();
        let open = self.picker.is_some();
        let ink = if missing {
            theme::faint()
        } else {
            theme::text()
        };
        let trigger = div()
            .id("files-root-picker")
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILES_ROOT_BUTTON".into())
            })
            .relative()
            .min_w(px(0.))
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded_md()
            .cursor_pointer()
            .text_color(ink)
            .when(open, |element| {
                element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
            })
            .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_picker(window, cx)))
            .child(
                canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .child(svg().flex_none().size(px(14.)).path(icon).text_color(ink))
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(theme::text_ui())
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(name),
            )
            .child(
                svg()
                    .flex_none()
                    .size(px(12.))
                    .path("chevron-down.svg")
                    .text_color(theme::muted()),
            );
        let status = match self.tree_ref() {
            Some(tree) if self.root_is_git() => Some(match tree.status() {
                None => div()
                    .when(cfg!(test), |element| {
                        element.debug_selector(|| "FILES_STATUS_READING".into())
                    })
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_color(theme::faint())
                    .child(
                        svg()
                            .size(px(12.))
                            .path("loader.svg")
                            .text_color(theme::faint()),
                    )
                    .child(crate::i18n::text("ui.reading_status"))
                    .into_any_element(),
                Some(status) if status.changed > 0 => div()
                    .when(cfg!(test), |element| {
                        let count = status.changed;
                        element.debug_selector(move || format!("FILES_CHANGED {count}"))
                    })
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_color(theme::muted())
                    .child(div().size(px(6.)).rounded_full().bg(theme::warning()))
                    .child(crate::i18n::tn("count.changed", status.changed))
                    .into_any_element(),
                Some(_) => div().into_any_element(),
            }),
            _ => None,
        };
        div()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILES_ROOT_HEADER".into())
            })
            .flex_none()
            .h(px(44.))
            .pl(px(10.))
            .pr(px(16.))
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(theme::border())
            .text_size(theme::text_meta())
            .child(div().min_w(px(0.)).flex_shrink(1.).child(trigger))
            .child(div().flex_1())
            .children(status)
            .into_any_element()
    }

    fn root_is_git(&self) -> bool {
        matches!(&self.body, Body::Root(state) if state.root.git.is_some())
    }

    fn render_picker(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let picker = self.picker.as_ref()?;
        let visible = picker::filter(&picker.items, &picker.query);
        let mut rows = Vec::new();
        let mut last_kind: Option<PickerKind> = None;
        for (position, index) in visible.iter().copied().enumerate() {
            let item = &picker.items[index];
            let section = match (last_kind, item.kind) {
                (Some(PickerKind::Project), PickerKind::Project)
                | (Some(PickerKind::Recent), PickerKind::Recent)
                | (Some(PickerKind::Worktree(_)), PickerKind::Worktree(_)) => None,
                (_, PickerKind::Project) => Some(crate::i18n::text("ui.projects")),
                (_, PickerKind::Recent) => Some(crate::i18n::text("ui.recent")),
                _ => None,
            };
            let divider = last_kind.is_some()
                && last_kind != Some(item.kind)
                && !matches!(
                    (last_kind, item.kind),
                    (Some(PickerKind::Worktree(_)), PickerKind::Worktree(_))
                );
            last_kind = Some(item.kind);
            if divider {
                rows.push(
                    div()
                        .my(px(4.))
                        .h(px(1.))
                        .bg(theme::border())
                        .into_any_element(),
                );
            }
            if let Some(section) = section {
                rows.push(
                    div()
                        .px(px(10.))
                        .pt(px(4.))
                        .pb(px(2.))
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(section)
                        .into_any_element(),
                );
            }
            let highlighted = position == picker.highlighted;
            if highlighted && picker.reveal.replace(false) {
                picker.scroll.scroll_to_item(rows.len());
            }
            let icon = if item.current {
                "check.svg"
            } else {
                match item.kind {
                    PickerKind::Worktree(WorktreeKind::Main) => "folder-git-2.svg",
                    PickerKind::Worktree(WorktreeKind::Linked) => "git-branch.svg",
                    PickerKind::Recent | PickerKind::Project => "folder.svg",
                    PickerKind::ChooseFolder => "folder-search.svg",
                }
            };
            rows.push(
                div()
                    .id(("files-picker-item", position))
                    .when(cfg!(test), |element| {
                        element.debug_selector(move || format!("FILES_PICKER_ITEM {position}"))
                    })
                    .px(px(10.))
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded_md()
                    .cursor_pointer()
                    .when(highlighted, |element| element.bg(theme::sidebar_selected()))
                    .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_picker_item(index, window, cx)
                    }))
                    .child(
                        svg()
                            .flex_none()
                            .size(px(14.))
                            .path(icon)
                            .text_color(if item.current {
                                theme::accent()
                            } else {
                                theme::muted()
                            }),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme::text_ui())
                                    .text_color(theme::text())
                                    .child(item.title.clone()),
                            )
                            .when(!item.subtitle.is_empty(), |element| {
                                element.child(
                                    div()
                                        .truncate()
                                        .text_size(theme::text_meta())
                                        .text_color(theme::faint())
                                        .child(item.subtitle.clone()),
                                )
                            }),
                    )
                    .into_any_element(),
            );
        }
        let content = div()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILES_PICKER".into())
            })
            .flex()
            .flex_col()
            .rounded(rems(8. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::raised())
            .shadow_2xl()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_none()
                    .h(px(40.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(theme::border())
                    .child(
                        svg()
                            .flex_none()
                            .size(px(14.))
                            .path("search.svg")
                            .text_color(theme::faint()),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(picker.field.clone())),
            )
            .child(
                div()
                    .id("files-picker-list")
                    .when(cfg!(test), |element| {
                        element.debug_selector(|| "FILES_PICKER_LIST".into())
                    })
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .track_scroll(&picker.scroll)
                    .p(px(6.))
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .into_any_element();
        let root = cx.entity();
        Some(popup_layer_sized(
            self.picker_anchor.get(),
            window,
            PopupWidth::Fixed(px(PICKER_WIDTH)),
            content,
            Rc::new(move |window, cx| {
                root.update(cx, |this, cx| {
                    this.picker = None;
                    this.focus.focus(window, cx);
                    cx.notify();
                })
            }),
        ))
    }

    fn render_tree(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(tree) = self.tree_ref() else {
            return div().into_any_element();
        };
        let focused = self.focus.contains_focused(window, cx);
        let count = tree.rows().len();
        let list = gpui::uniform_list(
            "files-tree",
            count,
            cx.processor(move |this, range: Range<usize>, _window, cx| {
                let rows = this
                    .tree_ref()
                    .map(|tree| tree.rows()[range.clone()].to_vec())
                    .unwrap_or_default();
                #[cfg(test)]
                crate::render_counts::count_render("files-tree-rows", cx);
                rows.into_iter()
                    .zip(range)
                    .map(|(row, index)| this.render_row(index, row, focused, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .py(px(6.))
        .size_full();
        div()
            .id("files-tree-scroll")
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILES_TREE".into())
            })
            .relative()
            .flex_1()
            .min_h(px(0.))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_tree_key))
            .child(list)
            .child(self.scrollbar.clone())
            .into_any_element()
    }

    fn render_message(
        &self,
        selector: &'static str,
        icon: &'static str,
        title: &'static str,
        body: String,
        actions: Option<AnyElement>,
    ) -> AnyElement {
        div()
            .when(cfg!(test), |element| {
                element.debug_selector(move || selector.into())
            })
            .flex_1()
            .min_h(px(0.))
            .px(px(28.))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .text_center()
            .child(svg().size(px(28.)).path(icon).text_color(theme::faint()))
            .child(
                div()
                    .text_size(theme::text_body())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .map(|element| {
                        #[cfg(test)]
                        let element = crate::theme_snapshot::record_text("FILES_TITLE", element);
                        element
                    })
                    .child(title),
            )
            .child(
                div()
                    .text_size(theme::text_ui())
                    .text_color(theme::muted())
                    .map(|element| {
                        #[cfg(test)]
                        let element = crate::theme_snapshot::record_text("FILES_BODY", element);
                        element
                    })
                    .child(body),
            )
            .children(actions.map(|actions| div().pt(px(8.)).child(actions)))
            .into_any_element()
    }

    fn render_choose(&self, cx: &mut Context<Self>) -> AnyElement {
        let home = Self::home(cx);
        let entry = |id: (&'static str, usize),
                     name: String,
                     path: PathBuf,
                     cx: &mut Context<Self>| {
            let subtitle = picker::display_path(&path, home.as_deref());
            div()
                .id(id)
                .when(cfg!(test), |element| {
                    element.debug_selector(move || format!("FILES_CHOOSE {} {}", id.0, id.1))
                })
                .px(px(10.))
                .py(px(7.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded_md()
                .cursor_pointer()
                .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_root(Some(path.clone()), cx)))
                .child(
                    svg()
                        .flex_none()
                        .size(px(16.))
                        .path("folder.svg")
                        .text_color(theme::muted()),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .text_size(theme::text_ui())
                                .text_color(theme::text())
                                .child(name),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(theme::text_meta())
                                .text_color(theme::faint())
                                .child(subtitle),
                        ),
                )
                .into_any_element()
        };
        let section = |label: &'static str| {
            div()
                .px(px(12.))
                .pt(px(14.))
                .pb(px(4.))
                .text_size(theme::text_meta())
                .text_color(theme::faint())
                .child(label)
        };
        let projects = self
            .projects(cx)
            .into_iter()
            .enumerate()
            .map(|(index, (name, path))| entry(("files-choose-project", index), name, path, cx))
            .collect::<Vec<_>>();
        let recent = self
            .recent
            .clone()
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                entry(
                    ("files-choose-recent", index),
                    picker::folder_name(&path),
                    path,
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let browse = cx.entity();
        div()
            .id("files-choose")
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILES_CHOOSE".into())
            })
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .px(px(16.))
            .pt(px(28.))
            .pb(px(16.))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(4.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        svg()
                            .size(px(22.))
                            .path("folder-open.svg")
                            .text_color(theme::faint()),
                    )
                    .child(
                        div()
                            .pt(px(4.))
                            .text_size(theme::text_body())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .map(|element| {
                                #[cfg(test)]
                                let element =
                                    crate::theme_snapshot::record_text("FILES_TITLE", element);
                                element
                            })
                            .child(crate::i18n::text("ui.choose_a_folder_to_show")),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(crate::i18n::tf(
                                "format.files_choose_folder_body",
                                &[("subject", self.noun())],
                            )),
                    ),
            )
            .child(
                div()
                    .pt(px(16.))
                    .relative()
                    .child(self.choose_field.clone())
                    .child(
                        div()
                            .absolute()
                            .top(px(16.))
                            .right(px(4.))
                            .bottom_0()
                            .flex()
                            .items_center()
                            .child(
                                IconButton::new("files-browse", "folder-search.svg")
                                    .tooltip(crate::i18n::text("ui.browse"))
                                    .on_press(move |_, cx| {
                                        browse.update(cx, |this, cx| this.browse(cx))
                                    }),
                            ),
                    ),
            )
            .when(!projects.is_empty(), |element| {
                element
                    .child(section(crate::i18n::text("ui.projects")))
                    .children(projects)
            })
            .when(!recent.is_empty(), |element| {
                element
                    .child(section(crate::i18n::text("ui.recent")))
                    .children(recent)
            })
            .into_any_element()
    }

    fn render_missing(&self, path: &Path, cx: &mut Context<Self>) -> AnyElement {
        let project = self.own_project(cx);
        let show_root = cx.entity();
        let choose_root = cx.entity();
        let actions = div()
            .flex()
            .gap_2()
            .justify_center()
            .children(project.map(|(name, path)| {
                Button::new(
                    "files-show-main-checkout",
                    crate::i18n::tf("format.show_folder", &[("name", name)]),
                )
                .variant(ButtonVariant::Secondary)
                .debug_selector("FILES_SHOW_MAIN")
                .on_press(move |_, cx| {
                    let path = path.clone();
                    show_root.update(cx, |this, cx| this.set_root(Some(path), cx));
                })
            }))
            .child(
                Button::new("files-choose-root", crate::i18n::text("ui.choose_ellipsis"))
                    .variant(ButtonVariant::Outline)
                    .on_press(move |window, cx| {
                        choose_root.update(cx, |this, cx| this.toggle_picker(window, cx))
                    }),
            )
            .into_any_element();
        let shown = compact_missing_path(path);
        self.render_message(
            "FILES_MISSING",
            "folder-x.svg",
            crate::i18n::text("ui.this_folder_no_longer_exists"),
            crate::i18n::tf("format.folder_was_moved_or_deleted", &[("path", shown)]),
            Some(actions),
        )
    }
}

/// The last two parts of a missing path, so the message names the folder
/// without the whole path: `…/.worktrees/fix-1116`.
fn compact_missing_path(path: &Path) -> String {
    let parts = path
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if parts.len() <= 3 {
        return path.display().to_string();
    }
    format!(
        "…{}{}",
        std::path::MAIN_SEPARATOR,
        parts[parts.len() - 2..].join(std::path::MAIN_SEPARATOR_STR)
    )
}

#[cfg(test)]
impl FilesView {
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
        let widening = self.widening();
        if widening != self.reported {
            self.reported = widening;
            if let Some(owner) = self.owner {
                cx.defer(move |cx| cx.notify(owner));
            }
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
        let tree_column = div()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .flex()
            .flex_col()
            .child(header)
            .children(self.error.clone().map(|error| {
                div()
                    .when(cfg!(test), |element| {
                        element.debug_selector(|| "FILES_ERROR".into())
                    })
                    .px_4()
                    .py_2()
                    .text_size(theme::text_meta())
                    .text_color(theme::danger())
                    .child(error)
            }))
            .child(body);
        let viewing = self.has_tabs();
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
                        .when(cfg!(test), |element| {
                            element.debug_selector(|| "FILES_TREE_COLUMN".into())
                        })
                        .when(viewing, |column| {
                            column
                                .flex_none()
                                .w(rems(tree_width / 16.))
                                .border_l_1()
                                .border_color(theme::border())
                        }),
                )
            })
            .relative()
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
            .children(self.menu.clone())
            .children(popups)
    }
}
