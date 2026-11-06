//! The side panel's Files view (#634): the root and its states, the root
//! picker and the tree with git status, served by `runnerd`'s file service.

mod code;
mod document;
mod finder;
mod highlight;
mod links;
mod markdown;
pub(crate) mod picker;
mod syntax;
mod tabs;
pub(crate) mod tree;
mod view;
mod viewer;

#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use futures::StreamExt as _;
use gpui::{
    canvas, AnyElement, App, AppContext as _, Entity, EntityId, EventEmitter, Global,
    IntoElement as _, Styled as _,
};
use runner_core::protocol::files::{
    FilesChanged, FilesStatusChanged, FILES_CHANGED_EVENT, FILES_STATUS_EVENT,
};
use runner_core::protocol::DaemonClient;

use crate::surfaces::side_panel::{self, PanelContext, PanelSubject, Widened};
pub(crate) use links::ShowPreview;
pub(crate) use view::FilesView;

/// The app's settings; `None` in a test that runs without a store.
pub(super) fn settings(cx: &App) -> Option<&crate::app_settings::AppSettings> {
    cx.try_global::<crate::app_store::GlobalAppStore>()
        .map(|store| &store.0.read(cx).settings)
}

/// What Settings → Side panel → Files sets that the view draws with; a
/// change redraws the open files.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Look {
    pub(super) icons: crate::file_icons::IconSet,
    font: String,
    size: u16,
}

impl Look {
    pub(super) fn current(cx: &App) -> Option<Self> {
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
    Status(FilesStatusChanged),
    /// Events may have been missed; with `rewatch` the daemon restarted and
    /// forgot every watch.
    Resync {
        rewatch: bool,
    },
}

/// Forwards the daemon's `files/*` events to every Files view.
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
                    FILES_STATUS_EVENT => serde_json::from_value(event.payload)
                        .ok()
                        .map(FilesEvent::Status),
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

/// Where Files stands among the panel's surfaces. Closing its tab keeps the
/// view and its file tabs for when it opens again, but gives the width back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FilesSurface {
    Shown,
    /// It has a tab, behind another surface's.
    Open,
    Closed,
}

impl FilesSurface {
    pub(crate) fn of(
        surface: side_panel::PanelSurface,
        settings: &crate::app_settings::AppSettings,
        open: &side_panel::OpenSurfaces,
    ) -> Self {
        let files = side_panel::PanelViewId::Files;
        if side_panel::shown(surface, settings, open) == Some(files) {
            Self::Shown
        } else if side_panel::tabs(surface, settings, open)
            .iter()
            .any(|(id, _)| *id == files)
        {
            Self::Open
        } else {
            Self::Closed
        }
    }
}

/// A side panel's Files views, one per subject, and which one is shown. The
/// owner calls [`FilesPanel::show`] while it draws the Files view and
/// [`FilesPanel::hide`] whenever it draws something else or stops drawing,
/// which is what starts and stops the shown view's watch.
#[derive(Default)]
pub(crate) struct FilesPanel {
    views: RefCell<Vec<(PanelSubject, Entity<FilesView>)>>,
    /// The width the panel shares with its chat or mission, unzoomed, as
    /// last drawn.
    surface: Rc<Cell<f32>>,
}

impl FilesPanel {
    /// Shows the subject's view, telling it how the owner widened the panel
    /// for an open file, and which entity to redraw when opening or closing
    /// files changes the panel's width.
    pub(crate) fn show_with(
        &self,
        context: &PanelContext,
        widened: Option<Widened>,
        owner: Option<EntityId>,
        cx: &mut App,
    ) -> AnyElement {
        let mut views = self.views.borrow_mut();
        let view = match views
            .iter()
            .position(|(subject, _)| *subject == context.subject)
        {
            Some(index) => views.remove(index).1,
            None => {
                let context = context.clone();
                cx.new(|cx| FilesView::new(context, cx))
            }
        };
        for (_, other) in views.iter() {
            other.update(cx, |other, cx| other.set_shown(false, cx));
        }
        view.update(cx, |view, cx| {
            view.set_context(context.clone(), cx);
            view.set_widened(widened, cx);
            view.set_owner(owner);
            view.set_shown(true, cx);
        });
        views.push((context.subject.clone(), view.clone()));
        let excess = views.len().saturating_sub(KEPT_VIEWS);
        views.drain(..excess);
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
        let view = cx.new(|cx| FilesView::new(context.clone(), cx));
        views.push((context.subject.clone(), view.clone()));
        let excess = views.len().saturating_sub(KEPT_VIEWS);
        views.drain(..excess);
        view
    }

    /// Whether the subject's view widens the panel, and if so whether its
    /// tree toggle is on: a root does while Files shows, and an open file
    /// does beside the panel's other surfaces too, while Files has its tab.
    pub(crate) fn widening(
        &self,
        subject: &PanelSubject,
        files: FilesSurface,
        cx: &App,
    ) -> Option<bool> {
        self.views
            .borrow()
            .iter()
            .find(|(shown, _)| shown == subject)
            .and_then(|(_, view)| {
                let view = view.read(cx);
                view.widening().filter(|_| match files {
                    FilesSurface::Shown => true,
                    FilesSurface::Open => view.has_tabs(),
                    FilesSurface::Closed => false,
                })
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

    /// The panel's width for `subject`: `standard`, or widened by the file
    /// area and its tree into the room the chat or mission leaves, less `gap`.
    /// Without a file open, a tree on that has no room beside the file area
    /// takes the panel at `standard` instead, as the tree is the way to a file.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn layout(
        &self,
        subject: Option<&PanelSubject>,
        files: FilesSurface,
        standard: f32,
        file: f32,
        tree_width: f32,
        gap: f32,
        cx: &App,
    ) -> (f32, Option<Widened>) {
        let Some(tree) = subject.and_then(|subject| self.widening(subject, files, cx)) else {
            return (standard, None);
        };
        let available = self.surface.get() - gap;
        let measured = self.surface.get() > 0.;
        let fits = available >= side_panel::MAIN_MIN_WIDTH + standard + tree_width;
        if tree && measured && !fits && !self.has_file_open(subject, cx) {
            return (standard, None);
        }
        let layout = side_panel::widened(available, standard, file, tree_width, tree);
        (layout.width(), Some(layout))
    }

    fn has_file_open(&self, subject: Option<&PanelSubject>, cx: &App) -> bool {
        self.views
            .borrow()
            .iter()
            .any(|(shown, view)| Some(shown) == subject && view.read(cx).has_tabs())
    }

    pub(crate) fn hide(&self, cx: &mut App) {
        for (_, view) in self.views.borrow().iter() {
            view.update(cx, |view, cx| view.set_shown(false, cx));
        }
    }

    /// Hides every view and drops them, for a window that is closing.
    pub(crate) fn release(&self, cx: &mut App) {
        self.hide(cx);
        self.views.borrow_mut().clear();
    }

    #[cfg(test)]
    pub(crate) fn view(&self, subject: &PanelSubject) -> Option<Entity<FilesView>> {
        self.views
            .borrow()
            .iter()
            .find(|(shown, _)| shown == subject)
            .map(|(_, view)| view.clone())
    }
}
