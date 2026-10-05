use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use super::socket::SocketTransport;
use super::terminal::*;
use super::*;
use crate::daemon_process::{wait_unlocked, Launch};

pub struct ManagedTransport {
    active: Mutex<Arc<SocketTransport>>,
    launch: Launch,
    events: tokio::sync::broadcast::Sender<ClientEvent>,
    observer: Mutex<Option<Weak<dyn TerminalLifecycle>>>,
    stopping: AtomicBool,
}
impl ManagedTransport {
    pub fn connect(launch: Launch, hash: String) -> Result<Arc<Self>, super::socket::ConnectError> {
        let active = launch.connect_or_spawn(&hash)?;
        let mut stream = active.event_stream();
        let (events, _) = tokio::sync::broadcast::channel(8192);
        let transport = Arc::new(Self {
            active: Mutex::new(active),
            launch,
            events,
            observer: Mutex::default(),
            stopping: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&transport);
        std::thread::Builder::new().name("runnerd-reconnect".into()).spawn(move || {
            let mut restarts = VecDeque::new();
            loop {
                let disconnected = weak.upgrade().is_none_or(|owner| owner.active().is_closed());
                let received = if disconnected {
                    match stream.try_recv() {
                        Ok(event) => Ok(event),
                        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(count)) => Err(tokio::sync::broadcast::error::RecvError::Lagged(count)),
                        Err(_) => Err(tokio::sync::broadcast::error::RecvError::Closed),
                    }
                } else { stream.blocking_recv() };
                let Some(owner) = weak.upgrade() else { break; };
                if owner.stopping.load(Ordering::Acquire) { break; }
                let event = match received {
                    Ok(event) => match event.event {
                        Some(event) => event,
                        None => ClientEvent { name: "daemon/lagged".into(), payload: serde_json::Value::Null },
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => ClientEvent { name: "daemon/lagged".into(), payload: serde_json::Value::Null },
                    Err(_) => ClientEvent { name: "daemon/disconnected".into(), payload: serde_json::Value::Null },
                };
                if event.name != "daemon/disconnected" { owner.emit(event); continue; }
                owner.emit(ClientEvent { name: "daemon/disconnected".into(), payload: serde_json::json!({"message":"runnerd stopped; reconnecting"}) });
                let now = Instant::now();
                while restarts.front().is_some_and(|time| now.duration_since(*time) > Duration::from_secs(300)) { restarts.pop_front(); }
                if restarts.len() >= 3 {
                    owner.emit(ClientEvent { name: "daemon/disconnected".into(), payload: serde_json::json!({"message":format!("runnerd restart limit reached (three restarts in five minutes). See {}", owner.launch.paths.log_dir.join("runnerd.log").display())}) });
                    break;
                }
                restarts.push_back(now);
                match owner.launch.connect_or_spawn(&hash) {
                    Ok(active) => {
                        stream = active.event_stream();
                        *owner.active.lock().unwrap() = active;
                        owner.emit(ClientEvent { name: "daemon/reconnected".into(), payload: serde_json::Value::Null });
                    }
                    Err(error) => {
                        owner.emit(ClientEvent { name: "daemon/disconnected".into(), payload: serde_json::json!({"message":format!("runnerd reconnect failed: {error}. See {}", owner.launch.paths.log_dir.join("runnerd.log").display())}) });
                        break;
                    }
                }
            }
        }).map_err(|error| super::socket::ConnectError::Protocol(error.to_string()))?;
        Ok(transport)
    }
    fn active(&self) -> Arc<SocketTransport> {
        self.active.lock().unwrap().clone()
    }
    fn emit(&self, event: ClientEvent) {
        let observer = self
            .observer
            .lock()
            .unwrap()
            .as_ref()
            .and_then(Weak::upgrade);
        if let Some(observer) = observer {
            observer.event(event.clone());
        }
        let _ = self.events.send(event);
    }
    pub fn client(self: &Arc<Self>) -> DaemonClient {
        DaemonClient::new(self.clone())
    }
    pub fn shutdown(&self) -> Result<(), ClientError> {
        let started = Instant::now();
        if !self.stopping.swap(true, Ordering::AcqRel) {
            self.active().shutdown(true)?;
        }
        wait_unlocked(
            &self.launch.paths.app_data_dir,
            Duration::from_secs(10).saturating_sub(started.elapsed()),
        )
        .map_err(|error| ClientError::msg(format!("wait for runnerd exit: {error}")))
    }
}
impl Transport for ManagedTransport {
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        self.active().call(request)
    }
    fn subscribe(&self) -> Box<dyn EventSubscription> {
        Box::new(Events(self.events.subscribe()))
    }
    fn attach(&self, id: &str) -> Result<TerminalAttachment, ClientError> {
        self.active().attach(id)
    }
    fn input(&self, id: &str, bytes: &[u8]) -> Result<(), ClientError> {
        self.active().input(id, bytes)
    }
    fn resize(&self, id: &str, origin: u64, cols: u16, rows: u16) {
        self.active().resize(id, origin, cols, rows);
    }
    fn observe_terminals(&self, observer: Weak<dyn TerminalLifecycle>) {
        *self.observer.lock().unwrap() = Some(observer);
    }
}
struct Events(tokio::sync::broadcast::Receiver<ClientEvent>);
impl EventSubscription for Events {
    fn recv(
        &mut self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ClientEvent, EventError>> + Send + '_>,
    > {
        Box::pin(async move {
            match self.0.recv().await {
                Ok(event) if event.name == "daemon/lagged" => Err(EventError::Lagged(1)),
                Ok(event) => Ok(event),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    Err(EventError::Lagged(count))
                }
                Err(_) => Err(EventError::Closed),
            }
        })
    }
}
