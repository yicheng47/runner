//! The side panel's Files view (#634): the root and its states, the root
//! picker and the tree with git status, served by `runnerd`'s file service.

pub(crate) mod code;
pub(crate) mod document;
mod finder;
pub(crate) mod highlight;
mod links;
mod markdown;
mod open_file;
mod panel_view;
pub(crate) mod picker;
pub(crate) mod syntax;
mod tabs;
pub(crate) mod tree;

#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{
    canvas, AnyElement, App, AppContext as _, Context, Entity, EntityId, EventEmitter, FocusHandle,
    Global, IntoElement as _, Styled as _, Subscription, Task, Window,
};
use runner_core::protocol::changes::{ChangesViewed, CHANGES_VIEWED_EVENT};
use runner_core::protocol::files::{
    FilesChanged, FilesRootOutcome, FilesStatusChanged, FilesSubject, FILES_CHANGED_EVENT,
    FILES_STATUS_EVENT,
};
use runner_core::protocol::DaemonClient;

use crate::surfaces::changes::{ChangesView, SavedChangesState};
use crate::surfaces::side_panel::{self, PanelContext, PanelSubject, Widened};
pub(crate) use links::ShowPreview;
pub(crate) use open_file::{editor_icon, format_size, found_editors};
use panel_view::SavedFilesState;
pub(crate) use panel_view::{absolute, compact_missing_path, FileTreeResizeDrag, FilesView};
use tree::PreparedStatus;

/// The app's settings; `None` in a test that runs without a store.
pub(crate) fn settings(cx: &App) -> Option<&crate::app_settings::AppSettings> {
    cx.try_global::<crate::app_store::GlobalAppStore>()
        .map(|store| &store.0.read(cx).settings)
}

/// What Settings → Side panel → Files sets that the view draws with; a
/// change redraws the open files.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Look {
    pub(crate) icons: crate::file_icons::IconSet,
    font: String,
    size: u16,
}

impl Look {
    pub(crate) fn current(cx: &App) -> Option<Self> {
        let settings = settings(cx)?;
        Some(Self {
            icons: settings.file_icons,
            font: settings
                .files_font_family
                .as_ref()
                .unwrap_or(&settings.terminal_font_family)
                .family()
                .to_owned(),
            size: settings.files_font_size(),
        })
    }
}

/// How many subjects' trees a panel keeps while they are not shown, so
/// switching back keeps their expansion and scroll.
const KEPT_VIEWS: usize = 12;

#[derive(Clone, Debug)]
pub(crate) enum FilesEvent {
    Changed(FilesChanged),
    Status {
        root: PathBuf,
        status: Arc<PreparedStatus>,
    },
    Viewed(ChangesViewed),
    RootChanged {
        subject: PanelSubject,
        origin: EntityId,
    },
    /// Events may have been missed; with `rewatch` the daemon restarted and
    /// forgot every watch.
    Resync {
        rewatch: bool,
    },
}

/// Forwards the daemon's `files/*` and `changes/*` events to every Files and Changes view.
pub(crate) struct FilesHub;

impl EventEmitter<FilesEvent> for FilesHub {}

struct GlobalFilesHub(Entity<FilesHub>);

impl Global for GlobalFilesHub {}

pub(crate) fn hub(client: &DaemonClient, cx: &mut App) -> Entity<FilesHub> {
    if let Some(hub) = cx.try_global::<GlobalFilesHub>() {
        return hub.0.clone();
    }
    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    let mut events = client.subscribe();
    cx.background_spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(event) => match event.name.as_str() {
                    FILES_CHANGED_EVENT => serde_json::from_value(event.payload)
                        .ok()
                        .map(FilesEvent::Changed),
                    FILES_STATUS_EVENT => serde_json::from_value(event.payload).ok().map(
                        |update: FilesStatusChanged| FilesEvent::Status {
                            root: update.root,
                            status: PreparedStatus::new(update.status),
                        },
                    ),
                    CHANGES_VIEWED_EVENT => serde_json::from_value(event.payload)
                        .ok()
                        .map(FilesEvent::Viewed),
                    "daemon/reconnected" => Some(FilesEvent::Resync { rewatch: true }),
                    _ => None,
                },
                Err(runner_core::protocol::EventError::Lagged(_)) => {
                    Some(FilesEvent::Resync { rewatch: false })
                }
                Err(runner_core::protocol::EventError::Closed) => break,
            };
            if event.is_some_and(|event| tx.unbounded_send(event).is_err()) {
                break;
            }
        }
    })
    .detach();
    let hub = cx.new(|_| FilesHub);
    let weak = hub.downgrade();
    cx.spawn(async move |cx| {
        while let Some(event) = rx.next().await {
            if weak.update(cx, |_, cx| cx.emit(event)).is_err() {
                break;
            }
        }
    })
    .detach();
    cx.set_global(GlobalFilesHub(hub.clone()));
    hub
}

/// Authoritative git-dirty tracker for the side panel's Changes activity-bar
/// indicator (#1403) when neither Files nor Changes currently owns a watch
/// (e.g. a mission open on Sessions). Acquires a scoped daemon watch + initial
/// status while active, consumes `FilesEvent::Status` updates, and releases
/// both its watch and `FilesHub` subscription as soon as Files/Changes takes
/// over, Changes is disabled, or the panel/workspace hides.
struct DirtyTracker {
    client: DaemonClient,
    active: bool,
    key: Option<(PanelSubject, Option<String>, Option<String>)>,
    last_subject: Option<PanelSubject>,
    watched: Option<PathBuf>,
    watch_chain: Option<Task<()>>,
    dirty: bool,
    owner: Option<EntityId>,
    generation: u64,
    hub: Option<Subscription>,
}

impl DirtyTracker {
    fn new(cx: &mut Context<Self>) -> Self {
        let client = crate::app_store::global_app_store(cx)
            .read(cx)
            .client
            .clone();
        Self {
            client,
            active: false,
            key: None,
            last_subject: None,
            watched: None,
            watch_chain: None,
            dirty: false,
            owner: None,
            generation: 0,
            hub: None,
        }
    }

    fn notify_owner(&self, cx: &mut Context<Self>) {
        if let Some(owner) = self.owner {
            cx.defer(move |cx| cx.notify(owner));
        }
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

    fn deactivate(&mut self, cx: &mut Context<Self>) {
        self.active = false;
        self.hub = None;
        self.key = None;
        self.generation += 1;
        self.unwatch(cx);
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        self.deactivate(cx);
        if let Some(task) = self.watch_chain.take() {
            task.detach();
        }
    }

    fn sync(
        &mut self,
        context: &PanelContext,
        owner: Option<EntityId>,
        shown: Option<side_panel::PanelViewId>,
        active_dirty: Option<bool>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.owner = owner;
        if context.remote {
            self.deactivate(cx);
            self.last_subject = None;
            self.dirty = false;
            return false;
        }
        let subject_changed = self.last_subject.as_ref() != Some(&context.subject);
        let tracker_owns_watch = !matches!(
            shown,
            Some(side_panel::PanelViewId::Files | side_panel::PanelViewId::Changes)
        );
        if !tracker_owns_watch {
            self.deactivate(cx);
            self.last_subject = Some(context.subject.clone());
            if let Some(dirty) = active_dirty {
                self.dirty = dirty;
            } else if subject_changed {
                self.dirty = false;
            }
            return self.dirty;
        }

        self.active = true;
        if self.hub.is_none() {
            let hub = hub(&self.client, cx);
            self.hub = Some(cx.subscribe(&hub, |this, _: Entity<FilesHub>, event, cx| {
                this.on_event(event, cx);
            }));
        }
        let next_key = (
            context.subject.clone(),
            context.cwd.clone(),
            context.project.clone(),
        );
        if self.key.as_ref() != Some(&next_key) {
            self.key = Some(next_key);
            self.last_subject = Some(context.subject.clone());
            if subject_changed {
                self.dirty = false;
            }
            self.generation += 1;
            self.start_watch(context.subject.clone(), cx);
        }
        self.dirty
    }

    fn start_watch(&mut self, subject: PanelSubject, cx: &mut Context<Self>) {
        let generation = self.generation;
        let client = self.client.clone();
        let files_subject = match subject {
            PanelSubject::Session(id) => FilesSubject::Session(id),
            PanelSubject::Mission(id) => FilesSubject::Mission(id),
        };
        let work = cx.background_spawn(async move {
            let outcome = client.files_root(&files_subject).ok()?;
            match outcome {
                FilesRootOutcome::Root(root) if root.git.is_some() => Some(Some(root.path)),
                _ => Some(None),
            }
        });
        cx.spawn(async move |this, cx| {
            let Some(root) = work.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if !this.active || this.generation != generation {
                    return;
                }
                this.set_watched_root(root, generation, cx);
            });
        })
        .detach();
    }

    fn set_watched_root(&mut self, root: Option<PathBuf>, generation: u64, cx: &mut Context<Self>) {
        let same_watched = match (self.watched.as_deref(), root.as_deref()) {
            (Some(a), Some(b)) => runner_core::app_paths::same_normalized_checkout_path(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same_watched {
            self.unwatch(cx);
        }
        let Some(root) = root else {
            if self.dirty {
                self.dirty = false;
                self.notify_owner(cx);
            }
            return;
        };
        let need_watch = !same_watched;
        if need_watch {
            self.watched = Some(root.clone());
        }
        let (status_tx, status_rx) = futures::channel::oneshot::channel();
        self.chain_watch(cx, move |client| {
            let result = if need_watch {
                client
                    .files_watch(&root)
                    .and_then(|()| client.files_status(&root))
            } else {
                client.files_status(&root)
            };
            let _ = status_tx.send(result.ok().map(|status| status.changed > 0));
        });
        cx.spawn(async move |this, cx| {
            let Ok(Some(dirty)) = status_rx.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if !this.active || this.generation != generation {
                    return;
                }
                if this.dirty != dirty {
                    this.dirty = dirty;
                    this.notify_owner(cx);
                }
            });
        })
        .detach();
    }

    fn on_event(&mut self, event: &FilesEvent, cx: &mut Context<Self>) {
        if !self.active {
            return;
        }
        match event {
            FilesEvent::Status { root, status } => {
                if self
                    .watched
                    .as_deref()
                    .is_some_and(|w| runner_core::app_paths::same_normalized_checkout_path(w, root))
                {
                    let dirty = status.status.changed > 0;
                    if self.dirty != dirty {
                        self.dirty = dirty;
                        self.notify_owner(cx);
                    }
                }
            }
            FilesEvent::RootChanged { subject, .. } => {
                if self.key.as_ref().is_some_and(|(s, _, _)| s == subject) {
                    self.generation += 1;
                    self.start_watch(subject.clone(), cx);
                }
            }
            FilesEvent::Resync { rewatch } => {
                if let Some((subject, _, _)) = self.key.clone() {
                    if *rewatch {
                        self.watched = None;
                    }
                    self.generation += 1;
                    self.start_watch(subject, cx);
                }
            }
            FilesEvent::Changed(_) | FilesEvent::Viewed(_) => {}
        }
    }
}

/// A side panel's Files and Changes views, one per subject, and which one is
/// shown. The owner calls [`FilesPanel::show_with_maximize`] /
/// [`FilesPanel::show_changes_with_maximize`] while drawing and
/// [`FilesPanel::hide`] whenever drawing something else or collapsing, which
/// starts and stops the shown view's watch.
#[derive(Default)]
pub(crate) struct FilesPanel {
    views: RefCell<Vec<(PanelSubject, Entity<FilesView>)>>,
    changes: RefCell<Vec<(PanelSubject, Entity<ChangesView>)>>,
    /// Lightweight open-file / diff selection preserved when an inactive view
    /// is evicted past [`KEPT_VIEWS`], so still-open tabs keep their open
    /// files/diffs without retaining background watches (#1403).
    saved_files: RefCell<HashMap<PanelSubject, SavedFilesState>>,
    saved_changes: RefCell<HashMap<PanelSubject, SavedChangesState>>,
    dirty_tracker: RefCell<Option<Entity<DirtyTracker>>>,
    /// The width the panel shares with its chat or mission, unzoomed, as
    /// last drawn.
    surface: Rc<Cell<f32>>,
}

impl FilesPanel {
    #[cfg(test)]
    pub(crate) fn show_with(
        &self,
        context: &PanelContext,
        widened: Option<Widened>,
        owner: Option<EntityId>,
        cx: &mut App,
    ) -> AnyElement {
        self.show_with_maximize(context, widened, owner, false, None, cx)
    }

    pub(crate) fn show_with_maximize(
        &self,
        context: &PanelContext,
        widened: Option<Widened>,
        owner: Option<EntityId>,
        maximized: bool,
        on_request_maximize: Option<side_panel::PanelMaximizeCallback>,
        cx: &mut App,
    ) -> AnyElement {
        let mut views = self.views.borrow_mut();
        let view = match views
            .iter()
            .position(|(subject, _)| *subject == context.subject)
        {
            Some(index) => views.remove(index).1,
            None => {
                let context_clone = context.clone();
                let saved = self.saved_files.borrow_mut().remove(&context.subject);
                cx.new(|cx| {
                    let mut view = FilesView::new(context_clone, cx);
                    if let Some(saved) = saved {
                        view.restore_saved_state(saved, cx);
                    }
                    view
                })
            }
        };
        for (_, other) in views.iter() {
            other.update(cx, |other, cx| other.set_shown(false, cx));
        }
        view.update(cx, |view, cx| {
            view.set_context(context.clone(), cx);
            view.set_maximized(maximized, cx);
            view.set_on_request_maximize(on_request_maximize);
            view.set_widened(widened, cx);
            view.set_owner(owner);
            view.set_shown(true, cx);
        });
        views.push((context.subject.clone(), view.clone()));
        let excess = views.len().saturating_sub(KEPT_VIEWS);
        if excess > 0 {
            let mut saved_files = self.saved_files.borrow_mut();
            for (evicted_subject, evicted_view) in views.drain(..excess) {
                if let Some(state) = evicted_view.read(cx).saved_state() {
                    saved_files.insert(evicted_subject, state);
                } else {
                    saved_files.remove(&evicted_subject);
                }
            }
        }
        view.into_any_element()
    }

    /// The subject's view for a ⌘-clicked link, made when the panel has not
    /// shown it yet, so its root resolves.
    pub(crate) fn link_view(&self, context: &PanelContext, cx: &mut App) -> Entity<FilesView> {
        let mut views = self.views.borrow_mut();
        if let Some((_, view)) = views
            .iter()
            .find(|(subject, _)| *subject == context.subject)
        {
            let view = view.clone();
            view.update(cx, |view, cx| view.set_context(context.clone(), cx));
            return view;
        }
        let saved = self.saved_files.borrow_mut().remove(&context.subject);
        let view = cx.new(|cx| {
            let mut view = FilesView::new(context.clone(), cx);
            if let Some(saved) = saved {
                view.restore_saved_state(saved, cx);
            }
            view
        });
        views.push((context.subject.clone(), view.clone()));
        let excess = views.len().saturating_sub(KEPT_VIEWS);
        if excess > 0 {
            let mut saved_files = self.saved_files.borrow_mut();
            for (evicted_subject, evicted_view) in views.drain(..excess) {
                if let Some(state) = evicted_view.read(cx).saved_state() {
                    saved_files.insert(evicted_subject, state);
                } else {
                    saved_files.remove(&evicted_subject);
                }
            }
        }
        view
    }

    pub(crate) fn files_widening(&self, subject: &PanelSubject, cx: &App) -> Option<bool> {
        if let Some((_, view)) = self
            .views
            .borrow()
            .iter()
            .find(|(shown, _)| shown == subject)
        {
            return view.read(cx).widening();
        }
        self.saved_files
            .borrow()
            .get(subject)
            .filter(|saved| !saved.open_paths.is_empty())
            .map(|saved| !saved.tree_hidden)
    }

    pub(crate) fn changes_widening(&self, subject: &PanelSubject, cx: &App) -> Option<bool> {
        if let Some((_, view)) = self
            .changes
            .borrow()
            .iter()
            .find(|(shown, _)| shown == subject)
        {
            return view.read(cx).widening().filter(|&wide| wide);
        }
        self.saved_changes
            .borrow()
            .contains_key(subject)
            .then_some(true)
    }

    /// Authoritative uncommitted-changes indicator for the Changes activity
    /// icon (#1403): uses the currently-active live tool when resolved, or
    /// holds a scoped daemon watch when neither tool is open (e.g. a mission on
    /// Sessions), releasing that watch as soon as Files/Changes takes over,
    /// Changes is disabled, or the panel/workspace hides.
    pub(crate) fn dirty_changes_for(
        &self,
        context: Option<&PanelContext>,
        shown: Option<side_panel::PanelViewId>,
        changes_enabled: bool,
        owner: Option<EntityId>,
        cx: &mut App,
    ) -> bool {
        let Some(context) = context else {
            self.hide_dirty_tracker(cx);
            return false;
        };
        if !changes_enabled
            || context.remote
            || cx
                .try_global::<crate::app_store::GlobalAppStore>()
                .is_none()
        {
            self.hide_dirty_tracker(cx);
            return false;
        }
        let active_dirty = match shown {
            Some(side_panel::PanelViewId::Files) => self
                .views
                .borrow()
                .iter()
                .find(|(s, _)| *s == context.subject)
                .and_then(|(_, view)| view.read(cx).fresh_dirty_changes()),
            Some(side_panel::PanelViewId::Changes) => self
                .changes
                .borrow()
                .iter()
                .find(|(s, _)| *s == context.subject)
                .and_then(|(_, view)| view.read(cx).fresh_dirty_changes()),
            _ => None,
        };
        let tracker = {
            let mut slot = self.dirty_tracker.borrow_mut();
            slot.get_or_insert_with(|| cx.new(DirtyTracker::new))
                .clone()
        };
        tracker.update(cx, |tracker, cx| {
            tracker.sync(context, owner, shown, active_dirty, cx)
        })
    }

    /// Measures the width the panel shares with its chat or mission; drawn
    /// over that surface. A change redraws the owner on the next frame.
    pub(crate) fn measure(&self, zoom: f32) -> AnyElement {
        let surface = self.surface.clone();
        canvas(
            move |bounds, window, _| {
                let width = f32::from(bounds.size.width) / zoom;
                if (surface.replace(width) - width).abs() > 0.5 {
                    window.request_animation_frame();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }

    pub(crate) fn measured_width(&self) -> f32 {
        self.surface.get()
    }

    pub(crate) fn maximized_width(&self, fallback: f32) -> f32 {
        let measured = self.surface.get();
        if measured > 0. {
            measured
        } else {
            fallback
        }
    }

    /// Computes the panel's width, widened state, and Docked vs Sheet
    /// presentation for `subject` (#1403, #1331). Files widens only when a
    /// file tab is open; Changes widens only when a diff is open (C1).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn panel_layout(
        &self,
        surface: side_panel::PanelSurface,
        settings: &crate::app_settings::AppSettings,
        selection: &side_panel::PanelSelection,
        subject: Option<&PanelSubject>,
        standard: f32,
        gap: f32,
        split_columns: usize,
        maximized: bool,
        cx: &App,
    ) -> side_panel::PanelLayout {
        let active = side_panel::shown(surface, settings, selection);
        let widened_tree = match active {
            Some(side_panel::PanelViewId::Files) => subject
                .filter(|_| settings.files_view)
                .and_then(|subject| self.files_widening(subject, cx)),
            Some(side_panel::PanelViewId::Changes) => subject
                .filter(|_| settings.changes_view)
                .and_then(|subject| self.changes_widening(subject, cx)),
            Some(side_panel::PanelViewId::Sessions) | None => None,
        };
        let measured = self.surface.get();
        let available = if measured > 0. {
            (measured - gap).max(0.)
        } else {
            0.
        };
        let file = match active {
            Some(side_panel::PanelViewId::Changes) => settings.changes_viewer_width,
            _ => settings.file_viewer_width,
        };
        let tree_width = settings.file_tree_width;
        let full = self.maximized_width(
            (side_panel::SPLIT_MIN_DIFF_WIDTH + tree_width)
                .max(file + tree_width)
                .max(standard),
        );
        side_panel::compute_panel_layout(
            available,
            split_columns,
            standard,
            file,
            tree_width,
            widened_tree,
            maximized,
            full,
        )
    }

    #[cfg(test)]
    pub(crate) fn show_changes_with(
        &self,
        context: &PanelContext,
        widened: Option<Widened>,
        owner: Option<EntityId>,
        cx: &mut App,
    ) -> AnyElement {
        self.show_changes_with_maximize(context, widened, owner, false, None, cx)
    }

    pub(crate) fn show_changes_with_maximize(
        &self,
        context: &PanelContext,
        widened: Option<Widened>,
        owner: Option<EntityId>,
        maximized: bool,
        on_request_maximize: Option<side_panel::PanelMaximizeCallback>,
        cx: &mut App,
    ) -> AnyElement {
        let mut changes = self.changes.borrow_mut();
        let view = match changes
            .iter()
            .position(|(subject, _)| *subject == context.subject)
        {
            Some(index) => changes.remove(index).1,
            None => {
                let context_clone = context.clone();
                let saved = self.saved_changes.borrow_mut().remove(&context.subject);
                cx.new(|cx| {
                    let mut view = ChangesView::new(context_clone, cx);
                    if let Some(saved) = saved {
                        view.restore_saved_state(saved, cx);
                    }
                    view
                })
            }
        };
        for (_, other) in changes.iter() {
            other.update(cx, |other, cx| other.set_shown(false, cx));
        }
        view.update(cx, |view, cx| {
            view.set_context(context.clone(), cx);
            view.set_maximized(maximized, cx);
            view.set_on_request_maximize(on_request_maximize);
            view.set_widened(widened, cx);
            view.set_owner(owner);
            view.set_shown(true, cx);
        });
        changes.push((context.subject.clone(), view.clone()));
        let excess = changes.len().saturating_sub(KEPT_VIEWS);
        if excess > 0 {
            let mut saved_changes = self.saved_changes.borrow_mut();
            for (evicted_subject, evicted_view) in changes.drain(..excess) {
                if let Some(state) = evicted_view.read(cx).saved_state() {
                    saved_changes.insert(evicted_subject, state);
                } else {
                    saved_changes.remove(&evicted_subject);
                }
            }
        }
        view.into_any_element()
    }

    pub(crate) fn focus_shown(
        &self,
        subject: Option<&PanelSubject>,
        shown: Option<side_panel::PanelViewId>,
        fallback: &FocusHandle,
        window: &mut Window,
        cx: &mut App,
    ) {
        match (shown, subject) {
            (Some(side_panel::PanelViewId::Changes), Some(subject)) => {
                if let Some(view) = self
                    .changes
                    .borrow()
                    .iter()
                    .find(|(s, _)| s == subject)
                    .map(|(_, v)| v.clone())
                {
                    view.update(cx, |view, cx| view.focus_default(window, cx));
                    return;
                }
            }
            (Some(side_panel::PanelViewId::Files), Some(subject)) => {
                if let Some(view) = self
                    .views
                    .borrow()
                    .iter()
                    .find(|(s, _)| s == subject)
                    .map(|(_, v)| v.clone())
                {
                    view.update(cx, |view, cx| view.focus_default(window, cx));
                    return;
                }
            }
            _ => {}
        }
        fallback.focus(window, cx);
    }

    pub(crate) fn hide_files(&self, cx: &mut App) {
        for (_, view) in self.views.borrow().iter() {
            view.update(cx, |view, cx| view.set_shown(false, cx));
        }
    }

    pub(crate) fn hide_changes(&self, cx: &mut App) {
        for (_, view) in self.changes.borrow().iter() {
            view.update(cx, |view, cx| view.set_shown(false, cx));
        }
    }

    pub(crate) fn hide_tools(&self, cx: &mut App) {
        self.hide_files(cx);
        self.hide_changes(cx);
    }

    fn hide_dirty_tracker(&self, cx: &mut App) {
        let tracker = self.dirty_tracker.borrow().clone();
        if let Some(tracker) = tracker {
            tracker.update(cx, |tracker, cx| tracker.deactivate(cx));
        }
    }

    pub(crate) fn hide(&self, cx: &mut App) {
        self.hide_tools(cx);
        self.hide_dirty_tracker(cx);
    }

    /// Hides every view and drops them, for a window that is closing.
    pub(crate) fn release(&self, cx: &mut App) {
        self.hide_tools(cx);
        if let Some(tracker) = self.dirty_tracker.borrow_mut().take() {
            tracker.update(cx, |tracker, cx| tracker.release(cx));
        }
        self.views.borrow_mut().clear();
        self.changes.borrow_mut().clear();
        self.saved_files.borrow_mut().clear();
        self.saved_changes.borrow_mut().clear();
    }

    #[cfg(test)]
    pub(crate) fn view(&self, subject: &PanelSubject) -> Option<Entity<FilesView>> {
        self.views
            .borrow()
            .iter()
            .find(|(shown, _)| shown == subject)
            .map(|(_, view)| view.clone())
    }

    #[cfg(all(test, unix))]
    pub(crate) fn dirty_tracker_active(&self, cx: &App) -> bool {
        self.dirty_tracker.borrow().as_ref().is_some_and(|tracker| {
            let t = tracker.read(cx);
            t.active || t.watched.is_some() || t.hub.is_some()
        })
    }

    #[cfg(test)]
    pub(crate) fn changes_view(&self, subject: &PanelSubject) -> Option<Entity<ChangesView>> {
        self.changes
            .borrow()
            .iter()
            .find(|(shown, _)| shown == subject)
            .map(|(_, view)| view.clone())
    }
}
