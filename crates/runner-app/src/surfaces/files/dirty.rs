use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{Context, Entity, EntityId, Subscription, Task};
use runner_core::protocol::files::{FilesRootOutcome, FilesSubject};
use runner_core::protocol::DaemonClient;

use super::{hub, FilesEvent, FilesHub};
use crate::surfaces::side_panel::{PanelContext, PanelSubject};

/// Authoritative git-dirty tracker for the side panel's Changes activity-bar
/// indicator (#1403, #1421) when neither `FilesView`, `ChangesView`, nor
/// `UnifiedViewer` currently owns a watch (e.g. a rail-only mission open on
/// Sessions). Acquires a scoped daemon watch + initial status while active,
/// consumes `FilesEvent::Status` updates, and releases both its watch and
/// `FilesHub` subscription as soon as another owner takes over, Changes is
/// disabled, or the panel/workspace hides.
pub(super) struct DirtyTracker {
    client: DaemonClient,
    pub(super) active: bool,
    key: Option<(PanelSubject, Option<String>, Option<String>)>,
    last_subject: Option<PanelSubject>,
    pub(super) watched: Option<PathBuf>,
    watch_chain: Option<Task<()>>,
    dirty: bool,
    owner: Option<EntityId>,
    generation: u64,
    pub(super) hub: Option<Subscription>,
}

impl DirtyTracker {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
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

    pub(super) fn deactivate(&mut self, cx: &mut Context<Self>) {
        self.active = false;
        self.hub = None;
        self.key = None;
        self.generation += 1;
        self.unwatch(cx);
    }

    pub(super) fn release(&mut self, cx: &mut Context<Self>) {
        self.deactivate(cx);
        if let Some(task) = self.watch_chain.take() {
            task.detach();
        }
    }

    pub(super) fn sync(
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
