//! The side panel's Files view (#634): the root and its states, the root
//! picker and the tree with git status, served by `runnerd`'s file service.

pub(crate) mod picker;
pub(crate) mod tree;
mod view;

#[cfg(test)]
mod tests;

use std::cell::RefCell;

use futures::StreamExt as _;
use gpui::{AnyElement, App, AppContext as _, Entity, EventEmitter, Global, IntoElement as _};
use runner_core::protocol::files::{
    FilesChanged, FilesStatusChanged, FILES_CHANGED_EVENT, FILES_STATUS_EVENT,
};
use runner_core::protocol::DaemonClient;

use crate::surfaces::side_panel::{PanelContext, PanelSubject};
#[allow(unused_imports)]
pub(crate) use view::FileOpen;
pub(crate) use view::FilesView;

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

/// A side panel's Files views, one per subject, and which one is shown. The
/// owner calls [`FilesPanel::show`] while it draws the Files view and
/// [`FilesPanel::hide`] whenever it draws something else or stops drawing,
/// which is what starts and stops the shown view's watch.
#[derive(Default)]
pub(crate) struct FilesPanel {
    views: RefCell<Vec<(PanelSubject, Entity<FilesView>)>>,
}

impl FilesPanel {
    pub(crate) fn show(&self, context: &PanelContext, cx: &mut App) -> AnyElement {
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
            view.set_shown(true, cx);
        });
        views.push((context.subject.clone(), view.clone()));
        let excess = views.len().saturating_sub(KEPT_VIEWS);
        views.drain(..excess);
        view.into_any_element()
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
