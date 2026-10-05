pub mod boot;
pub mod server;
use crate::AppCore;
use runner_core::protocol::command;
use runner_core::protocol::terminal::*;
use runner_core::protocol::*;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

macro_rules! define_dispatch {
    ($( $name:ident($( $arg:ident: $client_ty:ty => $wire_ty:ty = $convert:block ),*) -> $out:ty [$fast:literal] => $handler:expr; )*) => {
        pub fn dispatch(core: &AppCore, request: Request) -> Response {
            match request { $( Request::$name { $($arg),* } => Response::$name(
                ($handler)(core, $($arg),*).map_err(|error: crate::error::Error| ClientError::msg(error.to_string()))
            ), )* }
        }
    };
}
runner_core::daemon_api!(define_dispatch);

pub struct InProcessTransport(pub AppCore);
impl InProcessTransport {
    pub fn client(core: AppCore) -> DaemonClient {
        DaemonClient::new(Arc::new(Self(core)))
    }
}
impl Transport for InProcessTransport {
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        let bytes =
            serde_json::to_vec(&request).map_err(|error| ClientError::msg(error.to_string()))?;
        let request =
            serde_json::from_slice(&bytes).map_err(|error| ClientError::msg(error.to_string()))?;
        let response = dispatch(&self.0, request);
        let bytes =
            serde_json::to_vec(&response).map_err(|error| ClientError::msg(error.to_string()))?;
        serde_json::from_slice(&bytes).map_err(|error| ClientError::msg(error.to_string()))
    }
    fn observe_terminals(&self, observer: std::sync::Weak<dyn TerminalLifecycle>) {
        let observer: Arc<dyn crate::session::manager::SessionEvents> =
            Arc::new(TerminalEvents(observer));
        self.0.session_event_observer.install_owned(observer);
    }
    fn attach(&self, id: &str) -> Result<TerminalAttachment, ClientError> {
        self.0
            .sessions
            .attach_terminal(id)
            .map_err(|error| ClientError::msg(error.to_string()))
    }
    fn input(&self, id: &str, bytes: &[u8]) -> Result<(), ClientError> {
        self.0
            .sessions
            .queue_terminal_input(id, bytes)
            .map_err(|error| ClientError::msg(error.to_string()))
    }
    fn resize(&self, id: &str, origin: u64, cols: u16, rows: u16) {
        let _ = self
            .0
            .sessions
            .resize_terminal(id, origin, cols, rows, &self.0.db);
    }
    fn subscribe(&self) -> Box<dyn EventSubscription> {
        Box::new(InProcessEvents(self.0.events.subscribe()))
    }
}
struct InProcessEvents(tokio::sync::broadcast::Receiver<crate::events::AppEvent>);
impl EventSubscription for InProcessEvents {
    fn recv(
        &mut self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ClientEvent, EventError>> + Send + '_>,
    > {
        Box::pin(async move {
            self.0
                .recv()
                .await
                .map(|event| ClientEvent {
                    name: event.name.to_owned(),
                    payload: event.payload,
                })
                .map_err(|error| match error {
                    tokio::sync::broadcast::error::RecvError::Lagged(count) => {
                        EventError::Lagged(count)
                    }
                    tokio::sync::broadcast::error::RecvError::Closed => EventError::Closed,
                })
        })
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    struct ThreadWake(std::thread::Thread);
    impl std::task::Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => return value,
            std::task::Poll::Pending => std::thread::park(),
        }
    }
}

fn discovery_snapshot(core: &AppCore) -> crate::error::Result<DiscoverySnapshot> {
    let discovery = core
        .runtime_discovery
        .read()
        .map_err(|_| crate::error::Error::msg("runtime discovery lock poisoned"))?;
    let shell_env = core
        .runtime_shell_env
        .read()
        .map_err(|_| crate::error::Error::msg("runtime shell environment lock poisoned"))?
        .clone();
    Ok(DiscoverySnapshot {
        checking: discovery.checking,
        result: discovery.result.clone(),
        shell_env,
    })
}

mod commands;

pub mod resume;

struct TerminalEvents(std::sync::Weak<dyn TerminalLifecycle>);
impl TerminalEvents {
    fn emit(&self, name: &str, event: &impl serde::Serialize) {
        if let Some(observer) = self.0.upgrade() {
            observer.event(ClientEvent {
                name: name.into(),
                payload: serde_json::to_value(event).expect("terminal lifecycle event"),
            });
        }
    }
}
impl crate::session::manager::SessionEvents for TerminalEvents {
    fn spawned(&self, event: &crate::session::manager::SessionSpawnedEvent) {
        self.emit("session/spawned", event);
    }
    fn exit(&self, event: &crate::session::manager::ExitEvent) {
        self.emit("session/exit", event);
    }
    fn archived(&self, event: &crate::session::manager::SessionUpdatedEvent) {
        self.emit("session/archived", event);
    }
    fn updated(&self, event: &crate::session::manager::SessionUpdatedEvent) {
        self.emit("session/updated", event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_process_client_preserves_daemon_errors_and_patch_semantics() {
        let core = crate::test_support::test_core();
        crate::test_support::insert_test_role(
            &core.db.get().unwrap(),
            "role",
            "coder",
            "codex",
            "codex",
        );
        let client = InProcessTransport::client(core.clone());
        let backend_error = crate::ops::role::role_get(&core, "missing")
            .unwrap_err()
            .to_string();
        assert_eq!(
            client.role_get("missing").unwrap_err().message,
            backend_error
        );
        let patch = |model| UpdateRoleInput {
            model,
            ..Default::default()
        };
        client
            .role_update("role", patch(Some(Some("gpt-6".into()))))
            .unwrap();
        assert_eq!(
            client.role_get("role").unwrap().model.as_deref(),
            Some("gpt-6")
        );
        client.role_update("role", patch(None)).unwrap();
        assert_eq!(
            client.role_get("role").unwrap().model.as_deref(),
            Some("gpt-6")
        );
        client.role_update("role", patch(Some(None))).unwrap();
        assert_eq!(client.role_get("role").unwrap().model, None);
    }

    #[test]
    fn subscription_maps_payload_lag_and_close() {
        let core = crate::test_support::test_core();
        let client = InProcessTransport::client(core.clone());
        let mut subscription = client.subscribe();
        core.events
            .emit("sample/event", &serde_json::json!({ "value": [1, "样例"] }));
        let event = block_on(subscription.recv()).unwrap();
        assert_eq!(event.name, "sample/event");
        assert_eq!(event.payload, serde_json::json!({ "value": [1, "样例"] }));
        for _ in 0..8193 {
            core.events.emit("sample/event", &());
        }
        assert!(matches!(
            block_on(subscription.recv()),
            Err(EventError::Lagged(1))
        ));
        let mut closed = client.subscribe();
        drop(client);
        drop(core);
        assert!(matches!(block_on(closed.recv()), Err(EventError::Closed)));
    }
}
