pub mod core;

use crate::events::AppEvent;
use core::{Control, Core, Failure};
use runner_core::{
    companion::*,
    protocol::terminal::{
        InputEvent, InputKind, TerminalAttachment, TerminalController, TerminalFrame,
        TerminalSnapshot,
    },
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};
use tokio::{
    sync::{broadcast, Notify},
    time::{Duration, Instant},
};

pub const FEATURES: &[&str] = &[
    "overview",
    "attach",
    "input",
    "takeControl",
    "viewport",
    "detach",
];

pub fn classify_input(bytes: &[u8]) -> InputEvent {
    if let Some(paste) = bytes
        .strip_prefix(b"\x1b[200~")
        .and_then(|bytes| bytes.strip_suffix(b"\x1b[201~"))
    {
        InputEvent::Paste {
            text: String::from_utf8_lossy(paste).into_owned(),
        }
    } else if bytes == b"\r" {
        InputEvent::Key {
            kind: InputKind::Submit,
        }
    } else if let Ok(text) = std::str::from_utf8(bytes) {
        if !text.is_empty() && text.chars().all(|c| !c.is_control()) {
            InputEvent::Key {
                kind: InputKind::Content { text: text.into() },
            }
        } else {
            InputEvent::Key {
                kind: InputKind::Navigate,
            }
        }
    } else {
        InputEvent::Key {
            kind: InputKind::Navigate,
        }
    }
}

async fn blocking<T: Send + 'static>(
    core: Arc<dyn Core>,
    f: impl FnOnce(Arc<dyn Core>) -> core::Result<T> + Send + 'static,
) -> core::Result<T> {
    tokio::task::spawn_blocking(move || f(core))
        .await
        .unwrap_or(Err(Failure(ErrorCode::Failed, "The host worker failed.")))
}

#[derive(Default)]
struct Queue {
    frames: VecDeque<TerminalFrame>,
    bytes: usize,
    resync: bool,
    ended: bool,
}
impl Queue {
    fn push(&mut self, frame: TerminalFrame) -> bool {
        let bytes = match &frame {
            TerminalFrame::Output { bytes, .. } => bytes.len() + 64,
            _ => 64,
        };
        if matches!(frame, TerminalFrame::Resync) || self.bytes + bytes > MAX_MESSAGE_SIZE {
            self.frames.clear();
            self.bytes = 0;
            self.resync = true;
            false
        } else {
            self.bytes += bytes;
            self.frames.push_back(frame);
            true
        }
    }
}
struct Stream {
    session: String,
    resync: Arc<dyn Fn() -> core::Result<TerminalAttachment> + Send + Sync>,
    subscriber: u64,
    cancel: Arc<dyn Fn() + Send + Sync>,
    queue: Arc<Mutex<Queue>>,
    snapshot: Option<(TerminalSnapshot, usize)>,
    output: Option<(Vec<u8>, usize)>,
    seq: u64,
    initial_size: Option<(u16, u16, TerminalController)>,
    core: Arc<dyn Core>,
    channel: u64,
    release: bool,
}
impl Drop for Stream {
    fn drop(&mut self) {
        (self.cancel)();
        self.core
            .detach(&self.session, self.subscriber, self.channel, self.release);
    }
}
impl Stream {
    fn new(
        core: Arc<dyn Core>,
        channel: u64,
        session: String,
        attachment: core::Attachment,
        ready: Arc<Notify>,
    ) -> Self {
        let core::Attachment {
            mut terminal,
            resync,
        } = attachment;
        let queue = Arc::new(Mutex::new(Queue::default()));
        let producer = queue.clone();
        let cancel = terminal.frames.cancellation();
        std::thread::spawn(move || loop {
            let keep = match terminal.frames.recv() {
                Ok(frame) => producer.lock().unwrap().push(frame),
                Err(_) => {
                    producer.lock().unwrap().ended = true;
                    false
                }
            };
            ready.notify_one();
            if !keep {
                break;
            }
        });
        let snapshot = terminal.snapshot;
        Self {
            session,
            resync,
            subscriber: terminal.subscriber_id,
            cancel,
            queue,
            seq: snapshot.seq,
            initial_size: Some((snapshot.cols, snapshot.rows, snapshot.controller.clone())),
            snapshot: Some((snapshot, 0)),
            output: None,
            core,
            channel,
            release: true,
        }
    }
    fn controller(&self, controller: TerminalController) -> Controller {
        match controller {
            TerminalController::Desktop => Controller::Desktop,
            TerminalController::None => Controller::None,
            TerminalController::Browser { channel, .. } if channel == self.channel => {
                Controller::You
            }
            TerminalController::Browser { name, .. } => Controller::Browser { name },
        }
    }
    fn chunk(&mut self) -> Option<HostMessage> {
        if let Some((snapshot, offset)) = &mut self.snapshot {
            let end = (*offset + TERMINAL_CHUNK_SIZE).min(snapshot.bytes.len());
            let message = HostMessage::Snapshot(Snapshot {
                session: self.session.clone(),
                seq: self.seq,
                cols: snapshot.cols,
                rows: snapshot.rows,
                data: TerminalBytes(snapshot.bytes[*offset..end].to_vec()),
                more: end < snapshot.bytes.len(),
                preceding_char: snapshot.preceding_char,
                unfinished_len: snapshot.unfinished_len,
            });
            *offset = end;
            if end == snapshot.bytes.len() {
                self.snapshot = None;
            }
            return Some(message);
        }
        if let Some((cols, rows, controller)) = self.initial_size.take() {
            return Some(HostMessage::Resized(Resized {
                session: self.session.clone(),
                seq: self.seq,
                cols,
                rows,
                controller: self.controller(controller),
            }));
        }
        loop {
            if let Some((bytes, offset)) = &mut self.output {
                let end = (*offset + TERMINAL_CHUNK_SIZE).min(bytes.len());
                self.seq += 1;
                let message = HostMessage::Output(Output {
                    session: self.session.clone(),
                    seq: self.seq,
                    data: TerminalBytes(bytes[*offset..end].to_vec()),
                });
                *offset = end;
                if end == bytes.len() {
                    self.output = None;
                }
                return Some(message);
            }
            let frame = {
                let mut queue = self.queue.lock().unwrap();
                let frame = queue.frames.pop_front()?;
                queue.bytes -= match &frame {
                    TerminalFrame::Output { bytes, .. } => bytes.len() + 64,
                    _ => 64,
                };
                frame
            };
            match frame {
                TerminalFrame::Output { bytes, .. } => {
                    self.output = Some((bytes, 0));
                }
                TerminalFrame::Resized {
                    cols,
                    rows,
                    controller,
                    ..
                } => {
                    self.seq += 1;
                    return Some(HostMessage::Resized(Resized {
                        session: self.session.clone(),
                        seq: self.seq,
                        cols,
                        rows,
                        controller: self.controller(controller),
                    }));
                }
                TerminalFrame::Resync => unreachable!(),
            }
        }
    }
}

struct Lease {
    core: Arc<dyn Core>,
    channel: u64,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.core.release_channel(self.channel);
    }
}

pub struct Adapter {
    lease: Arc<Lease>,
    core: Arc<dyn Core>,
    channel: u64,
    name: String,
    events: broadcast::Receiver<AppEvent>,
    ready: Arc<Notify>,
    streams: VecDeque<Stream>,
    priority: VecDeque<Vec<u8>>,
    queued: usize,
    in_flight: usize,
    statuses: BTreeMap<String, SessionStatus>,
    overview_due: Option<Instant>,
    last_overview: Option<Instant>,
}
impl Adapter {
    pub fn new(core: Arc<dyn Core>, channel: u64, name: String) -> core::Result<Self> {
        let events = core.subscribe()?;
        Ok(Self {
            lease: Arc::new(Lease {
                core: core.clone(),
                channel,
            }),
            core,
            channel,
            name,
            events,
            ready: Arc::new(Notify::new()),
            streams: VecDeque::new(),
            priority: VecDeque::new(),
            queued: 0,
            in_flight: 0,
            statuses: BTreeMap::new(),
            overview_due: None,
            last_overview: None,
        })
    }
    pub async fn initialize(&mut self) -> Result<(), &'static str> {
        let overview = self.overview().await.map_err(|_| "core unavailable")?;
        self.statuses = overview
            .sessions
            .into_iter()
            .map(|(id, info)| (id, info.status))
            .collect();
        Ok(())
    }
    pub fn in_flight(&mut self, bytes: usize) {
        self.in_flight = bytes;
    }
    pub fn push(&mut self, message: HostMessage) -> Result<(), &'static str> {
        let bytes = serde_json::to_vec(&message).map_err(|_| "companion encoding failed")?;
        if self.queued + self.in_flight + bytes.len() > MAX_MESSAGE_SIZE {
            return Err("reply queue full");
        }
        self.queued += bytes.len();
        self.priority.push_back(bytes);
        Ok(())
    }
    fn remove(&mut self, session: &str, release: bool) {
        if let Some(index) = self
            .streams
            .iter()
            .position(|stream| stream.session == session)
        {
            let mut stream = self.streams.remove(index).unwrap();
            stream.release = release;
        }
    }
    async fn overview(&self) -> core::Result<Overview> {
        blocking(self.core.clone(), |core| core.overview()).await
    }
    fn status_changes(&mut self, overview: &Overview) -> Result<(), &'static str> {
        for stream in &self.streams {
            if overview
                .sessions
                .get(&stream.session)
                .is_none_or(|session| {
                    matches!(session.status.status, Status::Stopped | Status::Error)
                })
            {
                (stream.cancel)();
            }
        }
        for (session, info) in &overview.sessions {
            if self
                .statuses
                .get(session)
                .is_some_and(|old| old != &info.status)
            {
                self.push(HostMessage::Status(StatusPush {
                    session: session.clone(),
                    status: info.status.clone(),
                }))?;
            }
        }
        self.statuses = overview
            .sessions
            .iter()
            .map(|(id, info)| (id.clone(), info.status.clone()))
            .collect();
        Ok(())
    }
    pub async fn request(&mut self, message: ClientMessage) -> Result<(), &'static str> {
        let message = match message {
            ClientMessage::TakeControl(request) => return self.viewport(request, true).await,
            ClientMessage::Viewport(request) => return self.viewport(request, false).await,
            message => message,
        };
        let id;
        let result: core::Result<serde_json::Value> = match message {
            ClientMessage::Hello(_) => return Err("unexpected hello"),
            ClientMessage::Overview(request) => {
                id = request.id;
                match self.overview().await {
                    Ok(overview) => {
                        self.status_changes(&overview)?;
                        Ok(serde_json::to_value(overview)
                            .map_err(|_| "companion encoding failed")?)
                    }
                    Err(error) => Err(error),
                }
            }
            ClientMessage::Attach(request) => {
                id = request.id;
                let session = request.session.clone();
                let channel = self.channel;
                let name = self.name.clone();
                let history = request.history_lines() as usize;
                let size = request.clamped_size();
                let control = if request.control.effective() == ControlMode::Take {
                    Control::Take
                } else {
                    Control::IfFree
                };
                let lease = self.lease.clone();
                let ready = self.ready.clone();
                let result = blocking(self.core.clone(), move |core| {
                    core.resize(&session, channel, &name, size, control)?;
                    let attachment = match core.clone().attach(&session, history) {
                        Ok(attachment) => attachment,
                        Err(error) => {
                            core.detach(&session, 0, channel, true);
                            return Err(error);
                        }
                    };
                    Ok((
                        Stream::new(lease.core.clone(), channel, session, attachment, ready),
                        lease,
                    ))
                })
                .await;
                match result {
                    Ok((stream, _lease)) => {
                        self.remove(&request.session, false);
                        self.streams.push_back(stream);
                        Ok(serde_json::Value::Null)
                    }
                    Err(error) => Err(error),
                }
            }
            ClientMessage::Input(request) => {
                id = request.id;
                if !self.streams.iter().any(|s| s.session == request.session) {
                    Err(Failure(
                        ErrorCode::BadRequest,
                        "Attach the terminal before sending input.",
                    ))
                } else {
                    blocking(self.core.clone(), move |core| {
                        core.input(&request.session, &request.data.0)
                    })
                    .await
                    .map(|_| serde_json::Value::Null)
                }
            }
            ClientMessage::Detach(request) => {
                id = request.id;
                self.remove(&request.session, true);
                Ok(serde_json::Value::Null)
            }
            message => {
                id = match message {
                    ClientMessage::Feed(r) => r.id,
                    ClientMessage::Follow(r) | ClientMessage::Unfollow(r) => r.id,
                    ClientMessage::Answer(r) => r.id,
                    ClientMessage::Post(r) => r.id,
                    ClientMessage::Unknown(r) => r.id,
                    _ => unreachable!(),
                };
                Err(Failure(
                    ErrorCode::UnknownRequest,
                    "The host does not serve that request.",
                ))
            }
        };
        self.push(match result {
            Ok(result) => HostMessage::Result(ResultMessage { id, result }),
            Err(Failure(code, message)) => HostMessage::Error(ErrorMessage {
                id: Some(id),
                code,
                message: message.into(),
                versions: None,
            }),
        })
    }
    pub async fn viewport(
        &mut self,
        request: ViewportRequest,
        take: bool,
    ) -> Result<(), &'static str> {
        let id = request.id;
        let result = if self.streams.iter().any(|s| s.session == request.session) {
            let channel = self.channel;
            let name = self.name.clone();
            let size = request.clamped_size();
            let lease = self.lease.clone();
            blocking(self.core.clone(), move |core| {
                let _lease = lease;
                core.resize(
                    &request.session,
                    channel,
                    &name,
                    size,
                    if take {
                        Control::Take
                    } else {
                        Control::Viewport
                    },
                )
            })
            .await
        } else {
            Err(Failure(
                ErrorCode::BadRequest,
                "Attach the terminal before changing its viewport.",
            ))
        };
        self.push(match result {
            Ok(()) => HostMessage::Result(ResultMessage {
                id,
                result: serde_json::Value::Null,
            }),
            Err(Failure(code, message)) => HostMessage::Error(ErrorMessage {
                id: Some(id),
                code,
                message: message.into(),
                versions: None,
            }),
        })
    }
    pub async fn next(&mut self, terminal_credit: bool) -> Result<Option<Vec<u8>>, &'static str> {
        if let Some(bytes) = self.priority.pop_front() {
            self.queued -= bytes.len();
            return Ok(Some(bytes));
        }
        if !terminal_credit {
            return Ok(None);
        }
        for _ in 0..self.streams.len() {
            let mut stream = self.streams.pop_front().unwrap();
            let (resync, ended) = {
                let q = stream.queue.lock().unwrap();
                (q.resync, q.ended && q.frames.is_empty())
            };
            if resync {
                let resync = stream.resync.clone();
                let attachment = blocking(self.core.clone(), move |_| resync()).await;
                match attachment {
                    Ok(attachment) => {
                        let seq = stream.seq.max(attachment.snapshot.seq) + 1;
                        stream.release = false;
                        let mut fresh = Stream::new(
                            self.core.clone(),
                            self.channel,
                            stream.session.clone(),
                            core::Attachment {
                                terminal: attachment,
                                resync: stream.resync.clone(),
                            },
                            self.ready.clone(),
                        );
                        fresh.seq = seq;
                        stream = fresh;
                    }
                    Err(_) => {
                        let session = stream.session.clone();
                        let reason = blocking(self.core.clone(), move |core| {
                            Ok(core.closed_reason(&session))
                        })
                        .await
                        .unwrap_or(ClosedReason::Unknown);
                        return serde_json::to_vec(&HostMessage::Closed(Closed {
                            session: stream.session.clone(),
                            reason,
                        }))
                        .map(Some)
                        .map_err(|_| "companion encoding failed");
                    }
                }
            }
            if let Some(message) = stream.chunk() {
                self.streams.push_back(stream);
                return serde_json::to_vec(&message)
                    .map(Some)
                    .map_err(|_| "companion encoding failed");
            }
            if ended && !resync {
                let session = stream.session.clone();
                let reason = blocking(self.core.clone(), move |core| {
                    Ok(core.closed_reason(&session))
                })
                .await
                .unwrap_or(ClosedReason::Unknown);
                return serde_json::to_vec(&HostMessage::Closed(Closed {
                    session: stream.session.clone(),
                    reason,
                }))
                .map(Some)
                .map_err(|_| "companion encoding failed");
            }
            self.streams.push_back(stream);
        }
        Ok(None)
    }
    pub async fn changed(&mut self) -> Result<(), &'static str> {
        let due = self.overview_due;
        tokio::select! {
            _ = self.ready.notified() => {},
            event = self.events.recv() => {
                match event {
                    Ok(event) if relevant(event.name) => {
                        if matches!(event.name, "session/exit" | "session/archived") {
                            if let Some(session) = event.payload.get("session_id").and_then(|id| id.as_str()) {
                                if let Some(stream) = self.streams.iter().find(|stream| stream.session == session) {
                                    (stream.cancel)();
                                }
                            }
                        }
                        if self.overview_due.is_none() { self.overview_due = Some(self.last_overview.map(|last| last + Duration::from_millis(250)).unwrap_or_else(Instant::now)); }
                        let overview = self.overview().await.map_err(|_| "core unavailable")?;
                        self.status_changes(&overview)?;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => { self.overview_due = Some(Instant::now()); }
                    Err(broadcast::error::RecvError::Closed) => return Err("core stopped"),
                    _ => {}
                }
            }
            _ = async { match due { Some(due) => tokio::time::sleep_until(due).await, None => std::future::pending().await } } => {
                let overview = self.overview().await.map_err(|_| "core unavailable")?;
                self.status_changes(&overview)?;
                self.push(HostMessage::Overview(overview))?;
                self.overview_due = None;
                self.last_overview = Some(Instant::now());
            }
        }
        Ok(())
    }
}
fn relevant(name: &str) -> bool {
    [
        "session/",
        "chat/",
        "mission/",
        "project/",
        "crew/",
        "slot/",
        "event/appended",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

#[cfg(test)]
pub(crate) mod tests;
