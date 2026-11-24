//! The side panel's Files view (#634) and unified File + Diff viewer (#1421):
//! shared per-subject tab strip, root resolution, watch ownership, and side-panel
//! composition (`[Unified Viewer | Right Rail]`).

pub(crate) mod code;
pub(crate) mod document;
mod finder;
pub(crate) mod highlight;
mod links;
pub(crate) mod open_file;
mod panel_view;
pub(crate) mod picker;
pub(crate) mod syntax;
pub(crate) mod tabs;
pub(crate) mod tree;

#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::prelude::*;
use gpui::{
    canvas, div, px, rems, AnyElement, App, Bounds, Context, Entity, EntityId, EventEmitter,
    FocusHandle, Global, MouseButton, Pixels, Subscription, Task, Window,
};
use runner_app::ui::resize::{resize_strip, resize_strip_inset, ResizeAxis};
use runner_core::protocol::changes::{ChangesViewed, CHANGES_VIEWED_EVENT};
use runner_core::protocol::files::{
    FilesChanged, FilesRootOutcome, FilesStatusChanged, FilesSubject, FILES_CHANGED_EVENT,
    FILES_STATUS_EVENT,
};
use runner_core::protocol::DaemonClient;

use crate::surfaces::changes::{ChangesView, SavedChangesState};
use crate::surfaces::side_panel::{self, PanelContext, PanelSubject, PanelViewId, Widened};
use crate::theme;
pub(crate) use links::ShowPreview;
pub(crate) use open_file::{
    editor_icon, format_size, found_editors, SavedViewerState, TitlebarWrapper, UnifiedViewer,
    ViewerChrome,
};
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

/// How many subjects' viewer & rail state a panel keeps live while inactive,
/// so switching back keeps expansion, scroll, and open tabs without re-reading.
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

/// Forwards the daemon's `files/*` and `changes/*` events to every Files,
/// Changes, and UnifiedViewer entity.
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
/// indicator (#1403, #1421) when neither `FilesView`, `ChangesView`, nor
/// `UnifiedViewer` currently owns a watch (e.g. a rail-only mission open on
/// Sessions). Acquires a scoped daemon watch + initial status while active,
/// consumes `FilesEvent::Status` updates, and releases both its watch and
/// `FilesHub` subscription as soon as another owner takes over, Changes is
/// disabled, or the panel/workspace hides.
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
        external_watch_owner: bool,
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
        if external_watch_owner {
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

struct SubjectBundle {
    subject: PanelSubject,
    viewer: Entity<UnifiedViewer>,
    files: Option<Entity<FilesView>>,
    changes: Option<Entity<ChangesView>>,
    saved_files: Option<SavedFilesState>,
    saved_changes: Option<SavedChangesState>,
}

#[derive(Clone, Debug)]
struct SavedSubjectState {
    viewer: Option<SavedViewerState>,
    files: Option<SavedFilesState>,
    changes: Option<SavedChangesState>,
}

/// Properties for composing the aligned `40 px` unified viewer + right rail
/// inside chat/terminal and mission side panels (#1421).
pub(crate) struct SidePanelRenderProps {
    pub(crate) surface: side_panel::PanelSurface,
    pub(crate) settings: crate::app_settings::AppSettings,
    pub(crate) selection: side_panel::PanelSelection,
    pub(crate) shown: Option<PanelViewId>,
    pub(crate) context: Option<PanelContext>,
    pub(crate) widened: Option<Widened>,
    pub(crate) maximized: bool,
    pub(crate) remote: bool,
    pub(crate) owner: EntityId,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) header_selector: &'static str,
    pub(crate) titlebar_id: &'static str,
    pub(crate) round_header_tl_when_compact: bool,
    pub(crate) macos_titlebar_inset: Option<(f32, Option<AnyElement>)>,
    pub(crate) wrap_titlebar: TitlebarWrapper,
    pub(crate) on_surface_action: side_panel::SurfaceHandler,
    pub(crate) maximize_control: Option<side_panel::MaximizeControl>,
    pub(crate) on_request_maximize: side_panel::PanelMaximizeCallback,
    pub(crate) make_collapse_button: Rc<dyn Fn(&mut App) -> AnyElement>,
    pub(crate) sessions_body: Option<AnyElement>,
    pub(crate) sessions_focus: Option<FocusHandle>,
    pub(crate) loading_fallback: Option<AnyElement>,
}

/// A side panel's shared per-subject `UnifiedViewer` plus lazy `FilesView` and
/// `ChangesView` rails (#1421).
#[derive(Default)]
pub(crate) struct FilesPanel {
    subjects: RefCell<Vec<SubjectBundle>>,
    /// Lightweight viewer/rail state preserved when an inactive subject is
    /// evicted past [`KEPT_VIEWS`], so still-open tabs keep their open
    /// files/diffs without retaining background watches.
    saved: RefCell<HashMap<PanelSubject, SavedSubjectState>>,
    dirty_tracker: RefCell<Option<Entity<DirtyTracker>>>,
    /// The width the panel shares with its chat or mission, unzoomed, as
    /// last drawn.
    surface: Rc<Cell<f32>>,
    /// Full composed side-panel bounds, so `⇧⌘O` centers over viewer + rail.
    panel_bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl FilesPanel {
    fn touch_or_create_bundle(
        &self,
        context: &PanelContext,
        create_if_absent: bool,
        cx: &mut App,
    ) -> Option<usize> {
        let mut subjects = self.subjects.borrow_mut();
        if let Some(index) = subjects.iter().position(|b| b.subject == context.subject) {
            let bundle = subjects.remove(index);
            subjects.push(bundle);
            return Some(subjects.len() - 1);
        }
        let has_saved_tabs = self
            .saved
            .borrow()
            .get(&context.subject)
            .is_some_and(|s| s.viewer.as_ref().is_some_and(|v| !v.tabs.is_empty()));
        if !create_if_absent && !has_saved_tabs {
            return None;
        }
        let saved = self.saved.borrow_mut().remove(&context.subject);
        let ctx_clone = context.clone();
        let saved_viewer = saved.as_ref().and_then(|s| s.viewer.clone());
        let viewer = cx.new(|cx| {
            let mut viewer = UnifiedViewer::new(ctx_clone, cx);
            if let Some(state) = saved_viewer {
                viewer.restore_saved_state(state, cx);
            }
            viewer
        });
        subjects.push(SubjectBundle {
            subject: context.subject.clone(),
            viewer,
            files: None,
            changes: None,
            saved_files: saved.as_ref().and_then(|s| s.files.clone()),
            saved_changes: saved.and_then(|s| s.changes),
        });
        let excess = subjects.len().saturating_sub(KEPT_VIEWS);
        if excess > 0 {
            let mut saved_map = self.saved.borrow_mut();
            for evicted in subjects.drain(..excess) {
                let viewer_state = evicted.viewer.read(cx).saved_state();
                let files_state = evicted
                    .files
                    .as_ref()
                    .and_then(|f| f.read(cx).saved_state())
                    .or(evicted.saved_files);
                let changes_state = evicted
                    .changes
                    .as_ref()
                    .and_then(|c| c.read(cx).saved_state())
                    .or(evicted.saved_changes);
                if viewer_state.is_some() || files_state.is_some() || changes_state.is_some() {
                    saved_map.insert(
                        evicted.subject,
                        SavedSubjectState {
                            viewer: viewer_state,
                            files: files_state,
                            changes: changes_state,
                        },
                    );
                } else {
                    saved_map.remove(&evicted.subject);
                }
            }
        }
        Some(subjects.len() - 1)
    }

    fn hide_other_subjects(&self, keep: Option<&PanelSubject>, cx: &mut App) {
        let bundles: Vec<_> = self
            .subjects
            .borrow()
            .iter()
            .filter(|b| keep.is_none_or(|k| &b.subject != k))
            .map(|b| (b.viewer.clone(), b.files.clone(), b.changes.clone()))
            .collect();
        for (viewer, files, changes) in bundles {
            if let Some(files) = files {
                files.update(cx, |v, cx| v.set_shown(false, cx));
            }
            if let Some(changes) = changes {
                changes.update(cx, |v, cx| v.set_shown(false, cx));
            }
            viewer.update(cx, |v, cx| v.set_shown(false, false, false, cx));
        }
    }

    fn ensure_files_in_bundle(
        &self,
        index: usize,
        context: &PanelContext,
        cx: &mut App,
    ) -> Entity<FilesView> {
        let mut subjects = self.subjects.borrow_mut();
        let bundle = &mut subjects[index];
        if let Some(files) = &bundle.files {
            return files.clone();
        }
        let viewer = bundle.viewer.clone();
        let ctx_clone = context.clone();
        let saved = bundle.saved_files.take();
        let files = cx.new(|cx| {
            let mut view = FilesView::new(ctx_clone, viewer, cx);
            if let Some(state) = saved {
                view.restore_saved_state(state, cx);
            }
            view
        });
        bundle.files = Some(files.clone());
        files
    }

    fn ensure_changes_in_bundle(
        &self,
        index: usize,
        context: &PanelContext,
        cx: &mut App,
    ) -> Entity<ChangesView> {
        let mut subjects = self.subjects.borrow_mut();
        let bundle = &mut subjects[index];
        if let Some(changes) = &bundle.changes {
            return changes.clone();
        }
        let viewer = bundle.viewer.clone();
        let ctx_clone = context.clone();
        let saved = bundle.saved_changes.take();
        let changes = cx.new(|cx| {
            let mut view = ChangesView::new(ctx_clone, viewer, cx);
            if let Some(state) = saved {
                view.restore_saved_state(state, cx);
            }
            view
        });
        bundle.changes = Some(changes.clone());
        changes
    }

    #[allow(clippy::too_many_arguments)]
    fn build_viewer_chrome(
        shown: Option<PanelViewId>,
        show_rail_column: bool,
        files_opt: Option<&Entity<FilesView>>,
        changes_opt: Option<&Entity<ChangesView>>,
        sessions_focus: Option<FocusHandle>,
        maximize_control: Option<side_panel::MaximizeControl>,
        collapse_when_rail_hidden: Option<AnyElement>,
        macos_titlebar_inset: Option<(f32, Option<AnyElement>)>,
        wrap_titlebar: Option<TitlebarWrapper>,
        cx: &App,
    ) -> ViewerChrome {
        // Weak handles avoid a strong reference cycle (FilesView/ChangesView hold Entity<UnifiedViewer>)
        // and deferred updates avoid re-entrant borrows when a rail updates UnifiedViewer.
        let files_weak = files_opt.map(Entity::downgrade);
        let changes_weak = changes_opt.map(Entity::downgrade);
        let hidden_rail_focus = (!show_rail_column)
            .then(|| match shown {
                Some(PanelViewId::Files) => files_opt.map(|f| f.read(cx).root_focus.clone()),
                Some(PanelViewId::Changes) => changes_opt.map(|c| c.read(cx).root_focus.clone()),
                Some(PanelViewId::Sessions) => sessions_focus.clone(),
                None => None,
            })
            .flatten();
        let finder_host = (!show_rail_column).then(|| files_weak.clone()).flatten();
        let esc_files = files_weak.clone();
        let esc_changes = changes_weak.clone();
        let esc_sessions = sessions_focus;
        let on_escape_to_rail: side_panel::PanelCallback = Rc::new(move |window, cx| {
            let esc_files = esc_files.clone();
            let esc_changes = esc_changes.clone();
            let esc_sessions = esc_sessions.clone();
            window.defer(cx, move |window, cx| match shown {
                Some(PanelViewId::Files) => {
                    if let Some(files) = &esc_files {
                        let _ = files.update(cx, |v, cx| v.focus_tree(window, cx));
                    }
                }
                Some(PanelViewId::Changes) => {
                    if let Some(changes) = &esc_changes {
                        let _ = changes.update(cx, |v, cx| v.focus_list(window, cx));
                    }
                }
                Some(PanelViewId::Sessions) => {
                    if let Some(focus) = &esc_sessions {
                        focus.focus(window, cx);
                    }
                }
                None => {}
            });
        });
        let focus_changes = changes_weak.clone();
        let on_diff_focus_in: side_panel::PanelCallback = Rc::new(move |_, cx| {
            if let Some(changes) = focus_changes.clone() {
                cx.defer(move |cx| {
                    let _ = changes.update(cx, |v, cx| v.on_viewer_diff_focused(cx));
                });
            }
        });
        let tab_files = files_weak;
        let tab_changes = changes_weak.clone();
        let on_tab_activated: open_file::TabActivatedCallback = Rc::new(move |key, cx| {
            let key = key.cloned();
            let tab_files = tab_files.clone();
            let tab_changes = tab_changes.clone();
            cx.defer(move |cx| {
                if let Some(files) = &tab_files
                    && let Some(key) = &key
                {
                    let path = key.path.clone();
                    let _ = files.update(cx, |v, cx| v.reveal_in_tree(&path, cx));
                }
                if let Some(changes) = &tab_changes {
                    let _ = changes.update(cx, |v, cx| v.sync_active_tab(key.as_ref(), cx));
                }
            });
        });
        let viewed_changes = changes_weak.clone();
        let on_viewed_toggled: open_file::ViewedToggledCallback =
            Rc::new(move |root, target, path, old_blob, new_blob, viewed, cx| {
                if let Some(changes) = viewed_changes.clone() {
                    let root = root.to_path_buf();
                    let target = target.clone();
                    let path = path.to_owned();
                    let old_blob = old_blob.map(str::to_owned);
                    let new_blob = new_blob.map(str::to_owned);
                    cx.defer(move |cx| {
                        let _ = changes.update(cx, |v, cx| {
                            v.apply_viewer_viewed_optimistic(
                                &root,
                                &target,
                                &path,
                                old_blob.as_deref(),
                                new_blob.as_deref(),
                                viewed,
                                cx,
                            );
                        });
                    });
                }
            });
        let viewed_err_changes = changes_weak;
        let on_viewed_error: open_file::ViewedErrorCallback = Rc::new(move |msg, cx| {
            if let Some(changes) = viewed_err_changes.clone() {
                cx.defer(move |cx| {
                    let _ = changes.update(cx, |v, cx| v.on_viewer_viewed_error(msg, cx));
                });
            }
        });
        ViewerChrome {
            show_rail_column,
            hidden_rail_focus,
            finder_host,
            maximize_control,
            collapse_when_rail_hidden,
            macos_titlebar_inset,
            wrap_titlebar,
            on_escape_to_rail: Some(on_escape_to_rail),
            on_diff_focus_in: Some(on_diff_focus_in),
            on_tab_activated: Some(on_tab_activated),
            on_viewed_toggled: Some(on_viewed_toggled),
            on_viewed_error: Some(on_viewed_error),
        }
    }

    fn compose_split(
        &self,
        viewer: Entity<UnifiedViewer>,
        rail_column: Option<AnyElement>,
        widened: Option<Widened>,
        zoom: f32,
        cx: &App,
    ) -> AnyElement {
        let panel_bounds = self.panel_bounds.clone();
        let measure = canvas(
            move |bounds, _, _| panel_bounds.set(bounds),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();
        let Some(rail_column) = rail_column else {
            return div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .flex()
                .flex_row()
                .child(viewer)
                .child(measure)
                .into_any_element();
        };
        let tree_width = widened.map_or(crate::app_settings::FILE_TREE_DEFAULT, |w| w.tree_width);
        let tree_resizing = viewer.read(cx).tree_resizing;
        let resize = resize_strip(
            "file-tree-resize",
            ResizeAxis::Columns,
            tree_resizing,
            runner_app::ui::resize::RESIZE_GRAB,
            zoom,
            Some(theme::border()),
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
        );
        let drag_viewer = viewer.clone();
        let drop_viewer = viewer.clone();
        let up_viewer = viewer.clone();
        let up_out_viewer = viewer.clone();
        div()
            .relative()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .flex()
            .flex_row()
            .child(viewer)
            .child(
                div()
                    .flex_none()
                    .w(rems(tree_width / 16.))
                    .h_full()
                    .min_h(px(0.))
                    .bg(theme::panel())
                    .rounded_tr(rems(11. / 16.))
                    .rounded_br(rems(11. / 16.))
                    .flex()
                    .flex_col()
                    .child(rail_column),
            )
            .child(resize)
            .child(measure)
            .on_drag_move::<FileTreeResizeDrag>(
                move |event: &gpui::DragMoveEvent<FileTreeResizeDrag>, _, cx| {
                    let tree = f32::from(event.bounds.right() - event.event.position.x) / zoom;
                    drag_viewer.update(cx, |v, cx| v.resize_tree(tree, cx));
                },
            )
            .on_drop(move |_: &FileTreeResizeDrag, _, cx| {
                drop_viewer.update(cx, |v, cx| v.finish_tree_resize(cx));
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                up_viewer.update(cx, |v, cx| v.finish_tree_resize(cx));
            })
            .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                up_out_viewer.update(cx, |v, cx| v.finish_tree_resize(cx));
            })
            .into_any_element()
    }

    /// Renders the complete aligned `40 px` side panel (`[UnifiedViewer | Right Rail]`
    /// when a document tab is open, or `[Right Rail]` when rail-only) (#1421).
    pub(crate) fn render_panel(&self, mut props: SidePanelRenderProps, cx: &mut App) -> AnyElement {
        let zoom = props.settings.app_zoom;
        let viewer_enabled =
            !props.remote && (props.settings.files_view || props.settings.changes_view);
        let create_bundle = matches!(props.shown, Some(PanelViewId::Files | PanelViewId::Changes));

        let bundle_index = props
            .context
            .as_ref()
            .and_then(|ctx| self.touch_or_create_bundle(ctx, create_bundle, cx));
        self.hide_other_subjects(props.context.as_ref().map(|c| &c.subject), cx);

        let (viewer_opt, files_opt, changes_opt) =
            if let (Some(idx), Some(ctx)) = (bundle_index, props.context.as_ref()) {
                match props.shown {
                    Some(PanelViewId::Files) => {
                        let _ = self.ensure_files_in_bundle(idx, ctx, cx);
                    }
                    Some(PanelViewId::Changes) => {
                        let _ = self.ensure_changes_in_bundle(idx, ctx, cx);
                    }
                    _ => {}
                }
                let subjects = self.subjects.borrow();
                let bundle = &subjects[idx];
                (
                    Some(bundle.viewer.clone()),
                    bundle.files.clone(),
                    bundle.changes.clone(),
                )
            } else {
                (None, None, None)
            };

        let has_viewer = viewer_enabled
            && viewer_opt
                .as_ref()
                .is_some_and(|viewer| viewer.read(cx).has_tabs());
        let tree_hidden = viewer_opt
            .as_ref()
            .is_some_and(|viewer| viewer.read(cx).tree_hidden);
        let show_rail_column =
            !has_viewer || (props.widened.is_none_or(|w| w.tree) && !tree_hidden);

        let mut rail_watches = false;
        let mut changes_rail_shown = false;
        if let Some(ctx) = props.context.as_ref() {
            if let Some(files) = &files_opt {
                let show_files = show_rail_column && props.shown == Some(PanelViewId::Files);
                let keep_finder = has_viewer && !show_rail_column;
                files.update(cx, |v, cx| {
                    v.set_context(ctx.clone(), cx);
                    v.set_widened(props.widened, cx);
                    v.set_owner(Some(props.owner));
                    v.set_panel_bounds(Some(self.panel_bounds.clone()));
                    v.set_shown_keeping_finder(show_files, keep_finder, cx);
                });
                if show_files {
                    rail_watches = true;
                }
            }
            if let Some(changes) = &changes_opt {
                let show_changes = show_rail_column && props.shown == Some(PanelViewId::Changes);
                let mut not_in_git = false;
                changes.update(cx, |v, cx| {
                    v.set_context(ctx.clone(), cx);
                    v.set_widened(props.widened, cx);
                    v.set_owner(Some(props.owner));
                    v.set_shown(show_changes, cx);
                    not_in_git = v.is_not_in_git();
                });
                if show_changes && !not_in_git {
                    rail_watches = true;
                    changes_rail_shown = true;
                }
            }
            if let Some(viewer) = &viewer_opt {
                let maximize_for_viewer =
                    has_viewer.then(|| props.maximize_control.take()).flatten();
                let macos_for_viewer = has_viewer
                    .then(|| props.macos_titlebar_inset.take())
                    .flatten();
                let collapse_for_viewer =
                    (has_viewer && !show_rail_column).then(|| (props.make_collapse_button)(cx));
                let chrome = Self::build_viewer_chrome(
                    props.shown,
                    show_rail_column,
                    files_opt.as_ref(),
                    changes_opt.as_ref(),
                    props.sessions_focus.take(),
                    maximize_for_viewer,
                    collapse_for_viewer,
                    macos_for_viewer,
                    Some(props.wrap_titlebar.clone()),
                    cx,
                );
                viewer.update(cx, |v, cx| {
                    v.set_context(ctx.clone(), cx);
                    v.set_owner(Some(props.owner));
                    v.set_widened(props.widened, cx);
                    v.set_maximized(props.maximized, cx);
                    v.set_on_request_maximize(Some(props.on_request_maximize.clone()));
                    v.set_chrome(chrome);
                    v.set_shown(has_viewer, rail_watches, changes_rail_shown, cx);
                });
            }
        }

        let dirty_changes = !props.remote
            && self.dirty_changes_for(
                props.context.as_ref(),
                props.shown,
                show_rail_column,
                props.settings.changes_view,
                Some(props.owner),
                cx,
            );

        let rail_column = show_rail_column.then(|| {
            #[cfg(test)]
            let header_selector = props.header_selector;
            let rail_header = (props.wrap_titlebar)(
                props.titlebar_id,
                div()
                    .flex_none()
                    .h(rems(40. / 16.))
                    .pl_2()
                    .pr_2()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(theme::border())
                    .when(
                        !has_viewer && (props.round_header_tl_when_compact || props.maximized),
                        |h| h.rounded_tl(rems(11. / 16.)),
                    )
                    .rounded_tr(rems(11. / 16.))
                    .when_some(
                        props.macos_titlebar_inset.take(),
                        |header, (pad, spacer)| header.pl(px(pad)).children(spacer),
                    )
                    .map(|element| {
                        #[cfg(test)]
                        let element = crate::theme_snapshot::record_fill(header_selector, element);
                        element
                    })
                    .child(side_panel::activity_bar(
                        props.surface,
                        &props.settings,
                        &props.selection,
                        dirty_changes,
                        props.on_surface_action,
                        props.maximize_control.take(),
                        (props.make_collapse_button)(cx),
                    )),
                cx,
            );
            let rail_body = match props.shown {
                Some(PanelViewId::Files) => files_opt
                    .map(IntoElement::into_any_element)
                    .or_else(|| props.loading_fallback.take())
                    .unwrap_or_else(|| div().flex_1().into_any_element()),
                Some(PanelViewId::Changes) => changes_opt
                    .map(IntoElement::into_any_element)
                    .or_else(|| props.loading_fallback.take())
                    .unwrap_or_else(|| div().flex_1().into_any_element()),
                Some(PanelViewId::Sessions) | None => props
                    .sessions_body
                    .take()
                    .or_else(|| props.loading_fallback.take())
                    .unwrap_or_else(|| div().flex_1().into_any_element()),
            };
            div()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .flex()
                .flex_col()
                .child(rail_header)
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .min_w(px(0.))
                        .pb(rems(8. / 16.))
                        .flex()
                        .flex_col()
                        .child(rail_body),
                )
                .into_any_element()
        });

        if has_viewer && let Some(viewer) = viewer_opt {
            self.compose_split(viewer, rail_column, props.widened, zoom, cx)
        } else {
            let panel_bounds = self.panel_bounds.clone();
            div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .flex()
                .flex_col()
                .children(rail_column)
                .child(
                    canvas(
                        move |bounds, _, _| panel_bounds.set(bounds),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
                .into_any_element()
        }
    }

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

    #[cfg(test)]
    pub(crate) fn show_with_maximize(
        &self,
        context: &PanelContext,
        widened: Option<Widened>,
        owner: Option<EntityId>,
        maximized: bool,
        on_request_maximize: Option<side_panel::PanelMaximizeCallback>,
        cx: &mut App,
    ) -> AnyElement {
        let idx = self
            .touch_or_create_bundle(context, true, cx)
            .expect("bundle created");
        self.hide_other_subjects(Some(&context.subject), cx);
        let files = self.ensure_files_in_bundle(idx, context, cx);
        let (viewer, changes_opt) = {
            let subjects = self.subjects.borrow();
            let b = &subjects[idx];
            (b.viewer.clone(), b.changes.clone())
        };
        if let Some(changes) = &changes_opt {
            changes.update(cx, |v, cx| v.set_shown(false, cx));
        }
        let has_viewer = !context.remote && viewer.read(cx).has_tabs();
        let tree_hidden = viewer.read(cx).tree_hidden;
        let show_rail_column = !has_viewer || (widened.is_none_or(|w| w.tree) && !tree_hidden);
        files.update(cx, |view, cx| {
            view.set_context(context.clone(), cx);
            view.set_widened(widened, cx);
            view.set_owner(owner);
            view.set_panel_bounds(Some(self.panel_bounds.clone()));
            view.set_shown_keeping_finder(show_rail_column, has_viewer && !show_rail_column, cx);
        });
        let chrome = Self::build_viewer_chrome(
            Some(PanelViewId::Files),
            show_rail_column,
            Some(&files),
            changes_opt.as_ref(),
            None,
            None,
            None,
            None,
            None,
            cx,
        );
        viewer.update(cx, |v, cx| {
            v.set_context(context.clone(), cx);
            v.set_owner(owner);
            v.set_widened(widened, cx);
            v.set_maximized(maximized, cx);
            v.set_on_request_maximize(on_request_maximize);
            v.set_chrome(chrome);
            v.set_shown(has_viewer, show_rail_column, false, cx);
        });
        let zoom = settings(cx).map_or(1., |s| s.app_zoom);
        if has_viewer {
            self.compose_split(
                viewer,
                show_rail_column.then(|| files.into_any_element()),
                widened,
                zoom,
                cx,
            )
        } else {
            let panel_bounds = self.panel_bounds.clone();
            div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .flex()
                .flex_col()
                .child(files)
                .child(
                    canvas(
                        move |bounds, _, _| panel_bounds.set(bounds),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
                .into_any_element()
        }
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

    #[cfg(test)]
    pub(crate) fn show_changes_with_maximize(
        &self,
        context: &PanelContext,
        widened: Option<Widened>,
        owner: Option<EntityId>,
        maximized: bool,
        on_request_maximize: Option<side_panel::PanelMaximizeCallback>,
        cx: &mut App,
    ) -> AnyElement {
        let idx = self
            .touch_or_create_bundle(context, true, cx)
            .expect("bundle created");
        self.hide_other_subjects(Some(&context.subject), cx);
        let changes = self.ensure_changes_in_bundle(idx, context, cx);
        let (viewer, files_opt) = {
            let subjects = self.subjects.borrow();
            let b = &subjects[idx];
            (b.viewer.clone(), b.files.clone())
        };
        if let Some(files) = &files_opt {
            files.update(cx, |v, cx| v.set_shown(false, cx));
        }
        let has_viewer = !context.remote && viewer.read(cx).has_tabs();
        let tree_hidden = viewer.read(cx).tree_hidden;
        let show_rail_column = !has_viewer || (widened.is_none_or(|w| w.tree) && !tree_hidden);
        let mut not_in_git = false;
        changes.update(cx, |view, cx| {
            view.set_context(context.clone(), cx);
            view.set_widened(widened, cx);
            view.set_owner(owner);
            view.set_shown(show_rail_column, cx);
            not_in_git = view.is_not_in_git();
        });
        let chrome = Self::build_viewer_chrome(
            Some(PanelViewId::Changes),
            show_rail_column,
            files_opt.as_ref(),
            Some(&changes),
            None,
            None,
            None,
            None,
            None,
            cx,
        );
        viewer.update(cx, |v, cx| {
            v.set_context(context.clone(), cx);
            v.set_owner(owner);
            v.set_widened(widened, cx);
            v.set_maximized(maximized, cx);
            v.set_on_request_maximize(on_request_maximize);
            v.set_chrome(chrome);
            v.set_shown(
                has_viewer,
                show_rail_column && !not_in_git,
                show_rail_column && !not_in_git,
                cx,
            );
        });
        let zoom = settings(cx).map_or(1., |s| s.app_zoom);
        if has_viewer {
            self.compose_split(
                viewer,
                show_rail_column.then(|| changes.into_any_element()),
                widened,
                zoom,
                cx,
            )
        } else {
            changes.into_any_element()
        }
    }

    /// The subject's `FilesView` for a ⌘-clicked link or `⇧⌘O` finder, created
    /// lazily on the shared subject bundle so its root resolves and opened
    /// files land in the unified viewer.
    pub(crate) fn link_view(&self, context: &PanelContext, cx: &mut App) -> Entity<FilesView> {
        let idx = self
            .touch_or_create_bundle(context, true, cx)
            .expect("bundle created");
        let files = self.ensure_files_in_bundle(idx, context, cx);
        files.update(cx, |view, cx| {
            view.set_context(context.clone(), cx);
            view.set_panel_bounds(Some(self.panel_bounds.clone()));
        });
        files
    }

    pub(crate) fn viewer_has_tabs(&self, subject: &PanelSubject, cx: &App) -> bool {
        if let Some(bundle) = self
            .subjects
            .borrow()
            .iter()
            .find(|b| &b.subject == subject)
        {
            return bundle.viewer.read(cx).has_tabs();
        }
        self.saved
            .borrow()
            .get(subject)
            .and_then(|s| s.viewer.as_ref())
            .is_some_and(|v| !v.tabs.is_empty())
    }

    pub(crate) fn viewer_widening(&self, subject: &PanelSubject, cx: &App) -> Option<bool> {
        if let Some(bundle) = self
            .subjects
            .borrow()
            .iter()
            .find(|b| &b.subject == subject)
        {
            return bundle.viewer.read(cx).widening();
        }
        self.saved
            .borrow()
            .get(subject)
            .and_then(|s| s.viewer.as_ref())
            .filter(|v| !v.tabs.is_empty())
            .map(|v| !v.tree_hidden)
    }

    #[cfg(test)]
    pub(crate) fn files_widening(&self, subject: &PanelSubject, cx: &App) -> Option<bool> {
        self.viewer_widening(subject, cx)
    }

    #[cfg(test)]
    pub(crate) fn changes_widening(&self, subject: &PanelSubject, cx: &App) -> Option<bool> {
        self.viewer_widening(subject, cx)
    }

    /// Authoritative uncommitted-changes indicator for the Changes activity
    /// icon (#1403, #1421): reads from whichever active entity owns the watch
    /// (`FilesView`, `ChangesView`, or `UnifiedViewer`), or holds a scoped
    /// daemon watch via `DirtyTracker` when none of them is watching (e.g. a
    /// rail-only mission on Sessions).
    pub(crate) fn dirty_changes_for(
        &self,
        context: Option<&PanelContext>,
        shown: Option<PanelViewId>,
        show_rail_column: bool,
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
        let (external_watch_owner, active_dirty) = {
            let subjects = self.subjects.borrow();
            let bundle = subjects.iter().find(|b| b.subject == context.subject);
            match bundle {
                Some(b) if show_rail_column && shown == Some(PanelViewId::Files) => (
                    true,
                    b.files
                        .as_ref()
                        .and_then(|v| v.read(cx).fresh_dirty_changes()),
                ),
                Some(b)
                    if show_rail_column
                        && shown == Some(PanelViewId::Changes)
                        && b.changes
                            .as_ref()
                            .is_none_or(|v| !v.read(cx).is_not_in_git()) =>
                {
                    (
                        true,
                        b.changes
                            .as_ref()
                            .and_then(|v| v.read(cx).fresh_dirty_changes()),
                    )
                }
                Some(b) if b.viewer.read(cx).has_tabs() => {
                    (true, b.viewer.read(cx).fresh_dirty_changes())
                }
                _ => (false, None),
            }
        };
        let tracker = {
            let mut slot = self.dirty_tracker.borrow_mut();
            slot.get_or_insert_with(|| cx.new(DirtyTracker::new))
                .clone()
        };
        tracker.update(cx, |tracker, cx| {
            tracker.sync(context, owner, external_watch_owner, active_dirty, cx)
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

    /// The measured workspace width left for the panel and the main column, past the
    /// docked panel's island gap, or `0` before the first measurement.
    pub(crate) fn available_width(&self) -> f32 {
        let measured = self.surface.get();
        if measured > 0. {
            (measured - runner_app::ui::work_card::ISLAND_GAP).max(0.)
        } else {
            0.
        }
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
    /// presentation for `subject` (#1403, #1331, #1421). Width is derived
    /// strictly from the subject's unified viewer open state — never from the
    /// active `PanelViewId` — so switching `Files ↔ Changes ↔ Sessions` yields
    /// `0 px` width jump.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn panel_layout(
        &self,
        surface: side_panel::PanelSurface,
        settings: &crate::app_settings::AppSettings,
        selection: &side_panel::PanelSelection,
        subject: Option<&PanelSubject>,
        standard: f32,
        split_columns: usize,
        maximized: bool,
        cx: &App,
    ) -> side_panel::PanelLayout {
        let active = side_panel::shown(surface, settings, selection);
        let viewer_enabled = settings.files_view || settings.changes_view;
        let widened_tree = subject
            .filter(|_| viewer_enabled && active.is_some())
            .and_then(|subject| self.viewer_widening(subject, cx));
        let available = self.available_width();
        let file = settings.file_viewer_width;
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

    pub(crate) fn focus_shown(
        &self,
        subject: Option<&PanelSubject>,
        shown: Option<PanelViewId>,
        fallback: &FocusHandle,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(subject) = subject {
            let bundle = self
                .subjects
                .borrow()
                .iter()
                .find(|b| &b.subject == subject)
                .map(|b| (b.viewer.clone(), b.files.clone(), b.changes.clone()));
            if let Some((viewer, files, changes)) = bundle {
                let viewer_only = viewer.read(cx).has_tabs() && viewer.read(cx).tree_hidden;
                if viewer_only {
                    viewer.update(cx, |v, cx| v.focus_active(window, cx));
                    return;
                }
                match shown {
                    Some(PanelViewId::Changes) => {
                        if let Some(view) = changes {
                            view.update(cx, |view, cx| view.focus_default(window, cx));
                            return;
                        }
                    }
                    Some(PanelViewId::Files) => {
                        if let Some(view) = files {
                            view.update(cx, |view, cx| view.focus_default(window, cx));
                            return;
                        }
                    }
                    _ => {}
                }
            }
        }
        fallback.focus(window, cx);
    }

    fn hide_dirty_tracker(&self, cx: &mut App) {
        let tracker = self.dirty_tracker.borrow().clone();
        if let Some(tracker) = tracker {
            tracker.update(cx, |tracker, cx| tracker.deactivate(cx));
        }
    }

    pub(crate) fn hide(&self, cx: &mut App) {
        self.hide_other_subjects(None, cx);
        self.hide_dirty_tracker(cx);
    }

    /// Hides every view and drops them, for a window that is closing.
    pub(crate) fn release(&self, cx: &mut App) {
        self.hide_other_subjects(None, cx);
        if let Some(tracker) = self.dirty_tracker.borrow_mut().take() {
            tracker.update(cx, |tracker, cx| tracker.release(cx));
        }
        self.subjects.borrow_mut().clear();
        self.saved.borrow_mut().clear();
    }

    #[cfg(test)]
    pub(crate) fn viewer(&self, subject: &PanelSubject) -> Option<Entity<UnifiedViewer>> {
        self.subjects
            .borrow()
            .iter()
            .find(|b| &b.subject == subject)
            .map(|b| b.viewer.clone())
    }

    #[cfg(test)]
    pub(crate) fn view(&self, subject: &PanelSubject) -> Option<Entity<FilesView>> {
        self.subjects
            .borrow()
            .iter()
            .find(|b| &b.subject == subject)
            .and_then(|b| b.files.clone())
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
        self.subjects
            .borrow()
            .iter()
            .find(|b| &b.subject == subject)
            .and_then(|b| b.changes.clone())
    }
}
