use futures::StreamExt as _;
use gpui::{App, AppContext as _, Entity, EventEmitter, Global};
use runner_core::protocol::changes::CHANGES_VIEWED_EVENT;
use runner_core::protocol::files::{FilesStatusChanged, FILES_CHANGED_EVENT, FILES_STATUS_EVENT};
use runner_core::protocol::DaemonClient;

use super::tree::PreparedStatus;
use super::FilesEvent;

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
