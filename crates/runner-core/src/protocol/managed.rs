use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use super::socket::SocketTransport;
use super::terminal::*;
use super::*;
use crate::daemon_process::{system_is_shutting_down, wait_unlocked, Launch, DAEMON_STOP_TIMEOUT};

pub const RESTART_LIMIT_NOTICE: &str =
    "Runner's background service keeps crashing. Sessions are stopped until it runs again.";

pub struct ManagedTransport {
    active: Mutex<Arc<SocketTransport>>,
    launch: Launch,
    events: tokio::sync::broadcast::Sender<ClientEvent>,
    observer: Mutex<Option<Weak<dyn TerminalLifecycle>>>,
    stopping: AtomicBool,
    restart_notice: Mutex<Option<usize>>,
    recovery_paused: AtomicBool,
    manual_recovery: Mutex<()>,
    resume_recovery: mpsc::Sender<mpsc::Sender<Result<(), ClientError>>>,
}
impl ManagedTransport {
    pub fn connect(launch: Launch, hash: String) -> Result<Arc<Self>, super::socket::ConnectError> {
        let (active, mut restarted) = launch.connect_or_spawn_with_restart(&hash)?;
        let mut stream = active.event_stream();
        let restart_notice = restarted
            .then(|| active.launch_resume_report())
            .flatten()
            .map(|report| report.resumed.len());
        restarted &= restart_notice.is_none();
        let (events, _) = tokio::sync::broadcast::channel(8192);
        let (resume_recovery, resumes) = mpsc::channel::<mpsc::Sender<Result<(), ClientError>>>();
        let transport = Arc::new(Self {
            active: Mutex::new(active),
            launch,
            events,
            observer: Mutex::default(),
            stopping: AtomicBool::new(false),
            restart_notice: Mutex::new(restart_notice),
            recovery_paused: AtomicBool::new(false),
            manual_recovery: Mutex::new(()),
            resume_recovery,
        });
        let weak = Arc::downgrade(&transport);
        std::thread::Builder::new()
            .name("runnerd-reconnect".into())
            .spawn(move || {
                let mut restarts = VecDeque::new();
                let mut manual_retry = false;
                'events: loop {
                    let disconnected = weak
                        .upgrade()
                        .is_none_or(|owner| owner.active().is_closed());
                    let received = if disconnected {
                        match stream.try_recv() {
                            Ok(event) => Ok(event),
                            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(count)) => {
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(count))
                            }
                            Err(_) => Err(tokio::sync::broadcast::error::RecvError::Closed),
                        }
                    } else {
                        stream.blocking_recv()
                    };
                    let Some(mut owner) = weak.upgrade() else {
                        break;
                    };
                    if owner.stopping.load(Ordering::Acquire) {
                        break;
                    }
                    let event = match received {
                        Ok(event) => match event.event {
                            Some(event) => event,
                            None => ClientEvent {
                                name: "daemon/lagged".into(),
                                payload: serde_json::Value::Null,
                            },
                        },
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => ClientEvent {
                            name: "daemon/lagged".into(),
                            payload: serde_json::Value::Null,
                        },
                        Err(_) => ClientEvent {
                            name: "daemon/disconnected".into(),
                            payload: serde_json::Value::Null,
                        },
                    };
                    if event.name == "daemon/launch-resumed" {
                        if restarted {
                            let count = event.payload["resumed"].as_array().map_or(0, Vec::len);
                            *owner.restart_notice.lock().unwrap() = Some(count);
                            owner.emit(restart_event(count));
                            restarted = false;
                        }
                        continue;
                    }
                    if event.name != "daemon/disconnected" {
                        owner.emit(event);
                        continue;
                    }
                    if system_is_shutting_down() {
                        log::info!("Windows is shutting down; runnerd recovery stopped");
                        owner.disconnect();
                        break;
                    }
                    owner.emit(ClientEvent {
                        name: "daemon/disconnected".into(),
                        payload: serde_json::Value::Null,
                    });
                    let now = Instant::now();
                    while restarts
                        .front()
                        .is_some_and(|time| now.duration_since(*time) > Duration::from_secs(300))
                    {
                        restarts.pop_front();
                    }
                    if manual_retry || restarts.len() >= 2 {
                        owner.recovery_paused.store(true, Ordering::Release);
                        owner.emit(ClientEvent {
                            name: "daemon/disconnected".into(),
                            payload: serde_json::json!({"restart_limit":true}),
                        });
                    } else {
                        restarts.push_back(now);
                    }
                    loop {
                        let reply = if owner.recovery_paused.load(Ordering::Acquire) {
                            drop(owner);
                            let Ok(reply) = resumes.recv() else {
                                break 'events;
                            };
                            let Some(next_owner) = weak.upgrade() else {
                                break 'events;
                            };
                            owner = next_owner;
                            if owner.stopping.load(Ordering::Acquire) {
                                let _ =
                                    reply.send(Err(ClientError::msg("runnerd connection closed")));
                                break 'events;
                            }
                            Some(reply)
                        } else {
                            None
                        };
                        if reply.is_some() {
                            restarts.clear();
                            manual_retry = true;
                        }
                        if system_is_shutting_down() {
                            owner.disconnect();
                            if let Some(reply) = reply {
                                let _ =
                                    reply.send(Err(ClientError::msg("Windows is shutting down")));
                            }
                            break 'events;
                        }
                        match owner.launch.connect_or_spawn_with_restart(&hash) {
                            Ok((active, did_restart)) => {
                                restarted = did_restart;
                                stream = active.event_stream();
                                if restarted {
                                    if let Some(report) = active.launch_resume_report() {
                                        *owner.restart_notice.lock().unwrap() =
                                            Some(report.resumed.len());
                                        owner.emit(restart_event(report.resumed.len()));
                                        restarted = false;
                                    }
                                }
                                *owner.active.lock().unwrap() = active;
                                owner.recovery_paused.store(false, Ordering::Release);
                                owner.emit(ClientEvent {
                                    name: "daemon/reconnected".into(),
                                    payload: serde_json::Value::Null,
                                });
                                if let Some(reply) = reply {
                                    let _ = reply.send(Ok(()));
                                }
                                break;
                            }
                            Err(error) => {
                                if system_is_shutting_down() {
                                    owner.disconnect();
                                    if let Some(reply) = reply {
                                        let _ = reply.send(Err(ClientError::msg(
                                            "Windows is shutting down",
                                        )));
                                    }
                                    break 'events;
                                }
                                log::error!(
                                    "runnerd reconnect failed: {error}; log: {}",
                                    owner.launch.paths.log_dir.join("runnerd.log").display()
                                );
                                owner.recovery_paused.store(true, Ordering::Release);
                                if let Some(reply) = reply {
                                    let _ = reply.send(Err(ClientError::msg(error.to_string())));
                                } else {
                                    owner.emit(ClientEvent {
                                        name: "daemon/disconnected".into(),
                                        payload: serde_json::json!({"restart_limit":true}),
                                    });
                                }
                            }
                        }
                    }
                }
            })
            .map_err(|error| super::socket::ConnectError::Protocol(error.to_string()))?;
        Ok(transport)
    }
    fn active(&self) -> Arc<SocketTransport> {
        self.active.lock().unwrap().clone()
    }
    fn connected(&self) -> Result<Arc<SocketTransport>, ClientError> {
        let active = self.active();
        if active.is_closed() && self.recovery_paused.load(Ordering::Acquire) {
            return Err(ClientError::msg(RESTART_LIMIT_NOTICE));
        }
        Ok(active)
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
    pub fn disconnect(&self) {
        self.stopping.store(true, Ordering::Release);
    }
    pub fn shutdown(&self) -> Result<(), ClientError> {
        let started = Instant::now();
        if !self.stopping.swap(true, Ordering::AcqRel) {
            self.active().shutdown(true)?;
        }
        wait_unlocked(
            &self.launch.paths.app_data_dir,
            DAEMON_STOP_TIMEOUT.saturating_sub(started.elapsed()),
        )
        .map_err(|error| ClientError::msg(format!("wait for runnerd exit: {error}")))
    }
}
impl Transport for ManagedTransport {
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        self.connected()?.call(request)
    }
    fn reconnect(&self) -> Result<(), ClientError> {
        if self.active().is_closed() {
            let _recovery = self.manual_recovery.lock().unwrap();
            if self.recovery_paused.load(Ordering::Acquire)
                && !self.stopping.load(Ordering::Acquire)
            {
                let (done, result) = mpsc::channel();
                self.resume_recovery
                    .send(done)
                    .map_err(|_| ClientError::msg("runnerd recovery stopped"))?;
                result
                    .recv()
                    .map_err(|_| ClientError::msg("runnerd recovery stopped"))??;
            }
        }
        Ok(())
    }
    fn subscribe(&self) -> Box<dyn EventSubscription> {
        let stream = self.events.subscribe();
        let initial = self.restart_notice.lock().unwrap().map(restart_event);
        Box::new(Events { stream, initial })
    }
    fn attach(&self, id: &str) -> Result<TerminalAttachment, ClientError> {
        self.connected()?.attach(id)
    }
    fn input(&self, id: &str, bytes: &[u8]) -> Result<(), ClientError> {
        self.connected()?.input(id, bytes)
    }
    fn resize(&self, id: &str, origin: u64, cols: u16, rows: u16) {
        self.active().resize(id, origin, cols, rows);
    }
    fn observe_terminals(&self, observer: Weak<dyn TerminalLifecycle>) {
        *self.observer.lock().unwrap() = Some(observer);
    }
}
fn restart_event(count: usize) -> ClientEvent {
    ClientEvent {
        name: "daemon/restarted".into(),
        payload: serde_json::json!({"count":count}),
    }
}
struct Events {
    stream: tokio::sync::broadcast::Receiver<ClientEvent>,
    initial: Option<ClientEvent>,
}
impl EventSubscription for Events {
    fn recv(
        &mut self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ClientEvent, EventError>> + Send + '_>,
    > {
        Box::pin(async move {
            if let Some(initial) = self.initial.take() {
                return Ok(initial);
            }
            match self.stream.recv().await {
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
