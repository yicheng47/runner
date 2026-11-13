mod actions;
pub mod core;
pub(crate) mod feed;

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
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::{
    sync::{broadcast, Notify},
    time::{Duration, Instant},
};

pub const FEATURES: &[&str] = &[
    "overview",
    "feed",
    "follow",
    "unfollow",
    "answer",
    "post",
    "role",
    "crew",
    "createRole",
    "updateRole",
    "deleteRole",
    "createCrew",
    "updateCrew",
    "deleteCrew",
    "addSlot",
    "updateSlot",
    "removeSlot",
    "reorderSlots",
    "runtimeTaskStart",
    "runtimeTask",
    "catalog",
    "usage",
    "folders",
    "startChat",
    "startShell",
    "stop",
    "markViewed",
    "resume",
    "restart",
    "fork",
    "close",
    "rename",
    "archive",
    "layout",
    "pin",
    "move",
    "reorderPinned",
    "createProject",
    "deleteProject",
    "startMission",
    "stopMission",
    "resumeMission",
    "linkMission",
    "unlinkMission",
    "attach",
    "input",
    "takeControl",
    "viewport",
    "detach",
    "ping",
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
        .unwrap_or(Err(Failure(
            ErrorCode::Failed,
            "The host worker failed.",
            None,
        )))
}

// One redraw can arrive as several PTY reads. Output following a send waits this long, so that
// the reads leave as one message.
const OUTPUT_WINDOW: Duration = Duration::from_millis(5);

#[derive(Default)]
struct Queue {
    frames: VecDeque<TerminalFrame>,
    bytes: usize,
    resync: bool,
    ended: bool,
}
impl Queue {
    fn cost(frame: &TerminalFrame) -> usize {
        match frame {
            TerminalFrame::Output { bytes, .. } => bytes.len() + 64,
            _ => 64,
        }
    }
    fn push(&mut self, frame: TerminalFrame) -> bool {
        let bytes = Self::cost(&frame);
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
    fn pop(&mut self) -> Option<TerminalFrame> {
        let frame = self.frames.pop_front()?;
        self.bytes -= Self::cost(&frame);
        Some(frame)
    }
    fn merge_output(&mut self, merged: &mut Vec<u8>) {
        while let Some(TerminalFrame::Output { bytes, .. }) = self.frames.front() {
            if merged.len() + bytes.len() > TERMINAL_CHUNK_SIZE {
                break;
            }
            merged.extend_from_slice(bytes);
            self.pop();
        }
    }
    // Waiting only helps output that can still grow. A full merge would push a flood towards
    // overflow, and a resize or an end behind the output must not be delayed.
    fn can_hold_output(&self) -> bool {
        let mut merged = 0;
        for frame in &self.frames {
            let TerminalFrame::Output { bytes, .. } = frame else {
                return false;
            };
            merged += bytes.len();
            if merged >= TERMINAL_CHUNK_SIZE {
                return false;
            }
        }
        !self.frames.is_empty() && !self.ended
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
    last_output: Option<Instant>,
    clipboard: VecDeque<(String, String, usize)>,
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
            last_output: None,
            clipboard: VecDeque::new(),
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
    fn output_due(&self) -> Option<Instant> {
        let last = self.last_output?;
        let hold = self.queue.lock().unwrap().can_hold_output();
        hold.then_some(last + OUTPUT_WINDOW)
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
        if let Some((target, text, offset)) = self.clipboard.front_mut() {
            let mut end = (*offset + TERMINAL_CHUNK_SIZE).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let message = HostMessage::Clipboard(ClipboardPush {
                session: self.session.clone(),
                target: target.clone(),
                text: text[*offset..end].to_owned(),
                offset: *offset as u64,
                more: end < text.len(),
            });
            *offset = end;
            if end == text.len() {
                self.clipboard.pop_front();
            }
            return Some(message);
        }
        loop {
            if let Some((bytes, offset)) = &mut self.output {
                let end = (*offset + TERMINAL_CHUNK_SIZE).min(bytes.len());
                self.seq += 1;
                self.last_output = Some(Instant::now());
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
            if self.output_due().is_some_and(|due| due > Instant::now()) {
                return None;
            }
            let frame = {
                let mut queue = self.queue.lock().unwrap();
                let mut frame = queue.pop()?;
                if let TerminalFrame::Output { bytes, .. } = &mut frame {
                    queue.merge_output(bytes);
                }
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

struct FeedSubscription {
    cursor: u64,
    dirty: bool,
}

enum StatePush {
    Overview,
    Status(String),
}
struct PendingMessage {
    bytes: Vec<u8>,
    state: Option<StatePush>,
}

enum RequestReply {
    Value(serde_json::Value),
    Follow(String, u64),
    Unfollow(String),
}
struct OverviewSnapshot {
    overview: Overview,
    statuses: BTreeMap<String, SessionStatus>,
    status_revision: u64,
}
enum FeedJob {
    Request(ClientMessage),
    Unfollow {
        mission: String,
        ids: Vec<u64>,
        reserved: usize,
    },
}
struct FeedRead {
    mission: String,
    discard: bool,
    work: tokio::task::JoinHandle<core::Result<feed::Updates>>,
}
struct FeedWork {
    ids: Vec<u64>,
    follow: Option<String>,
    reserved: usize,
    work: tokio::task::JoinHandle<core::Result<RequestReply>>,
}

pub struct Adapter {
    lease: Arc<Lease>,
    core: Arc<dyn Core>,
    channel: u64,
    name: String,
    events: broadcast::Receiver<AppEvent>,
    ready: Arc<Notify>,
    streams: VecDeque<Stream>,
    pong: Option<u64>,
    priority: VecDeque<PendingMessage>,
    queued: usize,
    in_flight: usize,
    statuses: BTreeMap<String, SessionStatus>,
    feeds: BTreeMap<String, FeedSubscription>,
    feed_read: Option<FeedRead>,
    feed_ready: Option<(String, Vec<u8>)>,
    last_feed: Option<String>,
    feed_turn: bool,
    output_due: Option<Instant>,
    overview_due: Option<Instant>,
    last_overview: Option<Instant>,
    overview_work: Option<tokio::task::JoinHandle<core::Result<OverviewSnapshot>>>,
    overview_waiters: Vec<u64>,
    overview_next: Vec<u64>,
    overview_replies: VecDeque<(u64, Arc<serde_json::Value>)>,
    overview_broadcast: bool,
    overview_hash: Option<u64>,
    status_work: Option<tokio::task::JoinHandle<core::Result<BTreeMap<String, SessionStatus>>>>,
    status_dirty: bool,
    status_revision: u64,
    status_started_revision: u64,
    status_epoch: Arc<AtomicU64>,
    requests: VecDeque<FeedJob>,
    request_work: Option<FeedWork>,
    feed_outstanding: usize,
    cleanup_bytes: usize,
    actions: VecDeque<ActionRequest>,
    action_work: Option<(
        u64,
        tokio::task::JoinHandle<core::Result<serde_json::Value>>,
    )>,
    forks: tokio::task::JoinSet<(u64, core::Result<serde_json::Value>)>,
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
            pong: None,
            priority: VecDeque::new(),
            queued: 0,
            in_flight: 0,
            statuses: BTreeMap::new(),
            feeds: BTreeMap::new(),
            feed_read: None,
            feed_ready: None,
            last_feed: None,
            feed_turn: true,
            output_due: None,
            overview_due: None,
            last_overview: None,
            overview_work: None,
            overview_waiters: Vec::new(),
            overview_next: Vec::new(),
            overview_replies: VecDeque::new(),
            overview_broadcast: false,
            overview_hash: None,
            status_work: None,
            status_dirty: false,
            status_revision: 0,
            status_started_revision: 0,
            status_epoch: Default::default(),
            requests: VecDeque::new(),
            request_work: None,
            feed_outstanding: 0,
            cleanup_bytes: 0,
            actions: VecDeque::new(),
            action_work: None,
            forks: tokio::task::JoinSet::new(),
        })
    }
    pub async fn initialize(&mut self) -> Result<(), &'static str> {
        self.statuses = blocking(self.core.clone(), |core| core.statuses())
            .await
            .map_err(|_| "core unavailable")?;
        Ok(())
    }
    pub fn in_flight(&mut self, bytes: usize) {
        self.in_flight = bytes;
    }
    pub fn push(&mut self, message: HostMessage) -> Result<(), &'static str> {
        let bytes = serde_json::to_vec(&message).map_err(|_| "companion encoding failed")?;
        let state = match &message {
            HostMessage::Overview(_) => Some(StatePush::Overview),
            HostMessage::Status(status) => Some(StatePush::Status(status.session.clone())),
            _ => None,
        };
        self.priority.retain(|pending| {
            let superseded = match (&state, &pending.state) {
                (Some(StatePush::Overview), Some(_)) => true,
                (Some(StatePush::Status(session)), Some(StatePush::Status(previous))) => {
                    session == previous
                }
                _ => false,
            };
            if superseded {
                self.queued -= pending.bytes.len();
            }
            !superseded
        });
        if self.queued + self.in_flight + self.cleanup_bytes + bytes.len() > MAX_MESSAGE_SIZE {
            return Err("reply queue full");
        }
        self.queued += bytes.len();
        self.priority.push_back(PendingMessage { bytes, state });
        Ok(())
    }
    /// Queues an OSC 52 write on the channel's stream for that terminal, sent
    /// in terminal-sized pieces under the same credit as its output, one write
    /// at a time in the order written. A newer write to a target replaces that
    /// target's pending one.
    fn clipboard(&mut self, payload: &serde_json::Value) {
        let field = |name| payload.get(name).and_then(serde_json::Value::as_str);
        let (Some(session), Some(target), Some(text)) =
            (field("session_id"), field("target"), field("text"))
        else {
            return;
        };
        if let Some(stream) = self
            .streams
            .iter_mut()
            .find(|stream| stream.session == session)
        {
            stream.clipboard.retain(|(pending, _, _)| pending != target);
            stream
                .clipboard
                .push_back((target.to_owned(), text.to_owned(), 0));
            self.ready.notify_one();
        }
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
    fn start_overview(&mut self) {
        if self.overview_work.is_none() {
            let core = self.core.clone();
            let status_epoch = self.status_epoch.clone();
            self.overview_work = Some(tokio::task::spawn_blocking(move || {
                let overview = core.overview()?;
                let status_revision = status_epoch.load(Ordering::Acquire);
                let statuses = core.statuses()?;
                Ok(OverviewSnapshot {
                    overview,
                    statuses,
                    status_revision,
                })
            }));
        }
    }
    fn start_statuses(&mut self) {
        if self.status_work.is_none() && self.status_dirty {
            self.status_dirty = false;
            self.status_started_revision = self.status_revision;
            let core = self.core.clone();
            self.status_work = Some(tokio::task::spawn_blocking(move || core.statuses()));
        }
    }
    fn start_request(&mut self) {
        if self.request_work.is_none() {
            if let Some(job) = self.requests.pop_front() {
                let core = self.core.clone();
                let (ids, follow, reserved) = match &job {
                    FeedJob::Request(request) => (
                        vec![request.id().unwrap()],
                        match request {
                            ClientMessage::Follow(request) => Some(request.mission.clone()),
                            _ => None,
                        },
                        0,
                    ),
                    FeedJob::Unfollow { ids, reserved, .. } => (ids.clone(), None, *reserved),
                };
                self.request_work = Some(FeedWork {
                    ids,
                    follow,
                    reserved,
                    work: tokio::task::spawn_blocking(move || match job {
                        FeedJob::Unfollow { mission, .. } => Ok(RequestReply::Unfollow(mission)),
                        FeedJob::Request(ClientMessage::Feed(request)) => core
                            .feed(request)
                            .and_then(|page| serde_json::to_value(page).map_err(core::failed))
                            .map(RequestReply::Value),
                        FeedJob::Request(ClientMessage::Follow(request)) => core
                            .feed_cursor(&request.mission)
                            .map(|cursor| RequestReply::Follow(request.mission, cursor)),
                        _ => unreachable!(),
                    }),
                });
            }
        }
    }
    fn start_action(&mut self) {
        if self.action_work.is_none() {
            if let Some(request) = self.actions.pop_front() {
                let core = self.core.clone();
                self.action_work = Some((
                    request.id(),
                    tokio::task::spawn_blocking(move || core.action(request)),
                ));
            }
        }
    }
    fn unfollow(&mut self, request: MissionRequest) -> Result<(), &'static str> {
        let pending = self.request_work.as_ref().is_some_and(|work| work.follow.as_ref() == Some(&request.mission))
            || self.requests.iter().any(|job| matches!(job, FeedJob::Request(ClientMessage::Follow(follow)) if follow.mission == request.mission));
        if !pending {
            self.drop_feed(&request.mission);
            return self.reply(request.id, Ok(serde_json::Value::Null));
        }
        let reserved = serde_json::to_vec(&HostMessage::Result(ResultMessage {
            id: request.id,
            result: serde_json::Value::Null,
        }))
        .map_err(|_| "companion encoding failed")?
        .len();
        if self.queued + self.in_flight + self.cleanup_bytes + reserved > MAX_MESSAGE_SIZE {
            return Err("reply queue full");
        }
        self.cleanup_bytes += reserved;
        for job in self.requests.iter_mut().rev() {
            match job {
                FeedJob::Request(ClientMessage::Follow(follow))
                    if follow.mission == request.mission =>
                {
                    break
                }
                FeedJob::Unfollow {
                    mission,
                    ids,
                    reserved: total,
                } if mission == &request.mission => {
                    ids.push(request.id);
                    *total += reserved;
                    return Ok(());
                }
                _ => {}
            }
        }
        self.requests.push_back(FeedJob::Unfollow {
            mission: request.mission,
            ids: vec![request.id],
            reserved,
        });
        self.start_request();
        Ok(())
    }
    fn status_changes(
        &mut self,
        statuses: BTreeMap<String, SessionStatus>,
    ) -> Result<(), &'static str> {
        for stream in &self.streams {
            if statuses
                .get(&stream.session)
                .is_none_or(|status| matches!(status.status, Status::Stopped | Status::Error))
            {
                (stream.cancel)();
            }
        }
        for (session, status) in &statuses {
            if self.statuses.get(session).is_some_and(|old| old != status) {
                self.push(HostMessage::Status(StatusPush {
                    session: session.clone(),
                    status: status.clone(),
                }))?;
            }
        }
        self.statuses = statuses;
        self.status_revision += 1;
        self.status_epoch
            .store(self.status_revision, Ordering::Release);
        Ok(())
    }
    pub async fn request(&mut self, message: ClientMessage) -> Result<(), &'static str> {
        let message = match message {
            ClientMessage::Action(request) => {
                let request = *request;
                if self.actions.len() + usize::from(self.action_work.is_some()) + self.forks.len()
                    >= 8
                {
                    return self.reply(
                        request.id(),
                        Err(Failure(
                            ErrorCode::Failed,
                            "Too many actions are pending.",
                            None,
                        )),
                    );
                }
                if matches!(request, ActionRequest::Fork { .. }) {
                    let core = self.core.clone();
                    self.forks.spawn(async move {
                        let id = request.id();
                        let result = tokio::task::spawn_blocking(move || core.action(request))
                            .await
                            .unwrap_or(Err(Failure(
                                ErrorCode::Failed,
                                "The host worker failed.",
                                None,
                            )));
                        (id, result)
                    });
                } else {
                    self.actions.push_back(request);
                    self.start_action();
                }
                return Ok(());
            }
            ClientMessage::Unfollow(request) => return self.unfollow(request),
            // Only the newest ping is kept and it counts against no queue, so a ping can neither
            // wait behind host work nor fail the channel.
            ClientMessage::Ping(request) => {
                self.pong = Some(request.id);
                return Ok(());
            }
            ClientMessage::Overview(request) => {
                if self.overview_waiters.len()
                    + self.overview_next.len()
                    + self.overview_replies.len()
                    >= 8
                {
                    return self.reply(
                        request.id,
                        Err(Failure(
                            ErrorCode::Failed,
                            "Too many overview requests are pending.",
                            None,
                        )),
                    );
                }
                if self.overview_work.is_some() {
                    // A running scan can predate a mutation this caller has awaited.
                    self.overview_next.push(request.id);
                } else {
                    self.overview_waiters.push(request.id);
                    self.start_overview();
                }
                return Ok(());
            }
            message @ (ClientMessage::Feed(_) | ClientMessage::Follow(_)) => {
                if self.feed_outstanding >= 8 {
                    return self.reply(
                        message.id().unwrap(),
                        Err(Failure(
                            ErrorCode::Failed,
                            "Too many requests are pending.",
                            None,
                        )),
                    );
                }
                self.feed_outstanding += 1;
                self.requests.push_back(FeedJob::Request(message));
                self.start_request();
                return Ok(());
            }
            ClientMessage::TakeControl(request) => return self.viewport(request, true).await,
            ClientMessage::Viewport(request) => return self.viewport(request, false).await,
            message => message,
        };
        let id;
        let result: core::Result<serde_json::Value> = match message {
            ClientMessage::TakeControl(_)
            | ClientMessage::Viewport(_)
            | ClientMessage::Ping(_)
            | ClientMessage::Action(_)
            | ClientMessage::Feed(_)
            | ClientMessage::Follow(_)
            | ClientMessage::Unfollow(_)
            | ClientMessage::Overview(_) => unreachable!(),
            ClientMessage::Hello(_) => return Err("unexpected hello"),
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
                    let attachment = match core.clone().attach(&session, history, channel) {
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
                        None,
                    ))
                } else {
                    let channel = self.channel;
                    blocking(self.core.clone(), move |core| {
                        core.input(&request.session, channel, &request.data.0)
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
            ClientMessage::Answer(request) => {
                id = request.id;
                blocking(self.core.clone(), move |core| core.answer(request))
                    .await
                    .map(|_| serde_json::Value::Null)
            }
            ClientMessage::Post(request) => {
                id = request.id;
                blocking(self.core.clone(), move |core| core.post(request))
                    .await
                    .map(|_| serde_json::Value::Null)
            }
            ClientMessage::Unknown(request) => {
                id = request.id;
                Err(Failure(
                    ErrorCode::UnknownRequest,
                    "The host does not serve that request.",
                    None,
                ))
            }
        };
        self.reply(id, result)
    }
    fn reply(
        &mut self,
        id: u64,
        result: core::Result<serde_json::Value>,
    ) -> Result<(), &'static str> {
        self.push(match result {
            Ok(result) => HostMessage::Result(ResultMessage { id, result }),
            Err(Failure(code, message, version_gate)) => HostMessage::Error(ErrorMessage {
                version_gate: version_gate.clone(),
                id: Some(id),
                code,
                message: version_gate
                    .as_ref()
                    .map_or_else(|| message.into(), ToString::to_string),
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
                None,
            ))
        };
        self.push(match result {
            Ok(()) => HostMessage::Result(ResultMessage {
                id,
                result: serde_json::Value::Null,
            }),
            Err(Failure(code, message, version_gate)) => HostMessage::Error(ErrorMessage {
                version_gate: version_gate.clone(),
                id: Some(id),
                code,
                message: version_gate
                    .as_ref()
                    .map_or_else(|| message.into(), ToString::to_string),
                versions: None,
            }),
        })
    }
    #[cfg(test)]
    pub async fn next(&mut self, terminal_credit: bool) -> Result<Option<Vec<u8>>, &'static str> {
        self.next_paced(terminal_credit, false).await
    }
    pub async fn next_paced(
        &mut self,
        terminal_credit: bool,
        paced: bool,
    ) -> Result<Option<Vec<u8>>, &'static str> {
        if let Some(id) = self.pong.take() {
            return serde_json::to_vec(&HostMessage::Pong(Pong { id }))
                .map(Some)
                .map_err(|_| "companion encoding failed");
        }
        if let Some(message) = self.priority.pop_front() {
            self.queued -= message.bytes.len();
            return Ok(Some(message.bytes));
        }
        if let Some((id, overview)) = self.overview_replies.pop_front() {
            return serde_json::to_vec(&HostMessage::Result(ResultMessage {
                id,
                result: overview.as_ref().clone(),
            }))
            .map(Some)
            .map_err(|_| "companion encoding failed");
        }
        if !terminal_credit {
            return Ok(None);
        }
        if self.feed_turn || paced {
            if let Some(bytes) = self.next_feed() {
                self.feed_turn = false;
                return Ok(Some(bytes));
            }
        }
        if paced {
            return Ok(None);
        }
        for _ in 0..self.streams.len() {
            let mut stream = self.streams.pop_front().unwrap();
            let (resync, ended) = {
                let q = stream.queue.lock().unwrap();
                (q.resync, q.ended && q.frames.is_empty())
            };
            if resync && stream.snapshot.is_none() && stream.initial_size.is_none() {
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
                        fresh.clipboard = std::mem::take(&mut stream.clipboard);
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
                self.feed_turn = true;
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
        self.output_due = self.streams.iter().filter_map(Stream::output_due).min();
        self.feed_turn = true;
        Ok(self.next_feed())
    }
    fn feed_changes(&mut self) {
        if self.feeds.is_empty() {
            return;
        }
        for feed in self.feeds.values_mut() {
            feed.dirty = true;
        }
        self.ready.notify_one();
    }
    fn drop_feed(&mut self, mission: &str) {
        self.feeds.remove(mission);
        // The read still counts as the one running, but a later follow of this mission is a new
        // subscription that its result must not touch.
        if let Some(read) = self
            .feed_read
            .as_mut()
            .filter(|read| read.mission == mission)
        {
            read.discard = true;
        }
        if self
            .feed_ready
            .as_ref()
            .is_some_and(|(ready, _)| ready == mission)
        {
            self.feed_ready = None;
        }
    }
    /// Reads run beside the channel loop, so a slow mission log holds up none of its other
    /// messages. One read runs at a time and a finished batch waits here until sent, so at most
    /// one extra batch is held.
    fn next_feed(&mut self) -> Option<Vec<u8>> {
        if let Some((_, bytes)) = self.feed_ready.take() {
            return Some(bytes);
        }
        if self.feed_read.is_some()
            || self.queued + self.in_flight + self.cleanup_bytes + feed::MAX_BATCH_BYTES
                > MAX_MESSAGE_SIZE
        {
            return None;
        }
        let (mission, cursor) = self
            .feeds
            .iter()
            .find(|(id, feed)| feed.dirty && self.last_feed.as_ref().is_none_or(|last| *id > last))
            .or_else(|| self.feeds.iter().find(|(_, feed)| feed.dirty))
            .map(|(id, feed)| (id.clone(), feed.cursor))?;
        // An append while this read runs marks the feed dirty again and earns another read.
        self.feeds.get_mut(&mission).unwrap().dirty = false;
        self.last_feed = Some(mission.clone());
        let core = self.core.clone();
        let id = mission.clone();
        self.feed_read = Some(FeedRead {
            mission,
            discard: false,
            work: tokio::task::spawn_blocking(move || core.feed_since(&id, cursor)),
        });
        None
    }
    fn finish_feed_read(
        &mut self,
        read: FeedRead,
        result: core::Result<feed::Updates>,
    ) -> Result<(), &'static str> {
        if read.discard {
            return Ok(());
        }
        let Some(subscription) = self.feeds.get_mut(&read.mission) else {
            return Ok(());
        };
        let Ok(updates) = result else {
            self.feeds.remove(&read.mission);
            return Ok(());
        };
        subscription.cursor = updates.cursor;
        subscription.dirty |= updates.more;
        if !updates.entries.is_empty() {
            let bytes = serde_json::to_vec(&HostMessage::FeedAppended(FeedAppended {
                mission: read.mission.clone(),
                entries: updates.entries,
            }))
            .map_err(|_| "companion encoding failed")?;
            self.feed_ready = Some((read.mission, bytes));
        }
        Ok(())
    }
    pub async fn changed(&mut self) -> Result<(), &'static str> {
        let due = self.overview_due;
        let output_due = self.output_due;
        let overview_running = self.overview_work.is_some();
        tokio::select! {
            _ = self.ready.notified() => {},
            event = self.events.recv() => {
                match event {
                    Ok(event) if event.name == "session/clipboard" => self.clipboard(&event.payload),
                    Ok(event) if relevant(event.name) => {
                        if matches!(event.name, "session/exit" | "session/archived") {
                            if let Some(session) = event.payload.get("session_id").and_then(|id| id.as_str()) {
                                if let Some(stream) = self.streams.iter().find(|stream| stream.session == session) {
                                    (stream.cancel)();
                                }
                            }
                        }
                        if self.overview_due.is_none() { self.overview_due = Some(self.last_overview.map(|last| last + Duration::from_millis(250)).unwrap_or_else(Instant::now)); }
                        if event.name == "event/appended" { self.feed_changes(); }
                        self.status_dirty = true; self.start_statuses();
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        self.feed_changes(); self.overview_due = Some(Instant::now());
                        self.status_dirty = true; self.start_statuses();
                    }
                    Err(broadcast::error::RecvError::Closed) => return Err("core stopped"),
                    _ => {}
                }
            }
            result = async { match &mut self.overview_work { Some(work) => work.await, None => std::future::pending().await } } => {
                self.overview_work = None;
                let waiters = std::mem::take(&mut self.overview_waiters);
                let broadcast = std::mem::take(&mut self.overview_broadcast);
                match result.unwrap_or(Err(Failure(ErrorCode::Failed, "The host worker failed.", None))) {
                    Ok(OverviewSnapshot { mut overview, statuses, status_revision }) => {
                        if self.status_revision == status_revision { self.status_changes(statuses)?; }
                        for (id, session) in &mut overview.sessions { if let Some(status) = self.statuses.get(id) { session.status = status.clone(); } }
                        if !waiters.is_empty() {
                            let value = Arc::new(serde_json::to_value(&overview).map_err(|_| "companion encoding failed")?);
                            self.overview_replies.extend(waiters.into_iter().map(|id| (id, value.clone())));
                        }
                        if broadcast {
                            let hash = overview_hash(&overview)?;
                            if self.overview_hash != Some(hash) {
                                self.push(HostMessage::Overview(overview))?;
                                self.overview_hash = Some(hash);
                            }
                        }
                    }
                    Err(Failure(code, message, version_gate)) => {
                        if waiters.is_empty() && self.overview_next.is_empty() { return Err("core unavailable"); }
                        for id in waiters { self.reply(id, Err(Failure(code.clone(), message, version_gate.clone())))?; }
                    }
                }
                self.last_overview = Some(Instant::now());
                if self.overview_due.is_some() { self.overview_due = Some(Instant::now() + Duration::from_millis(250)); }
                if !self.overview_next.is_empty() {
                    self.overview_waiters = std::mem::take(&mut self.overview_next);
                    self.start_overview();
                }
            }
            result = async { match &mut self.status_work { Some(work) => work.await, None => std::future::pending().await } } => {
                self.status_work = None;
                if self.status_started_revision == self.status_revision {
                    self.status_changes(result.map_err(|_| "core unavailable")?.map_err(|_| "core unavailable")?)?;
                } else { self.status_dirty = true; }
                self.start_statuses();
            }
            result = async { match &mut self.request_work { Some(work) => (&mut work.work).await, None => std::future::pending().await } } => {
                let work = self.request_work.take().unwrap();
                if work.reserved == 0 { self.feed_outstanding -= 1; }
                self.cleanup_bytes -= work.reserved;
                let result = result.unwrap_or(Err(Failure(ErrorCode::Failed, "The host worker failed.", None)))
                    .map(|reply| match reply {
                        RequestReply::Value(value) => value,
                        RequestReply::Follow(mission, cursor) => {
                            self.feeds.entry(mission).and_modify(|feed| feed.dirty = true).or_insert(FeedSubscription { cursor, dirty:true });
                            self.ready.notify_one();
                            serde_json::Value::Null
                        }
                        RequestReply::Unfollow(mission) => { self.drop_feed(&mission); serde_json::Value::Null }
                    });
                for id in work.ids {
                    let result = match &result { Ok(value) => Ok(value.clone()), Err(Failure(code, message, version_gate)) => Err(Failure(code.clone(), message, version_gate.clone())) };
                    self.reply(id, result)?;
                }
                self.start_request();
            }
            result = async { match &mut self.feed_read { Some(read) => (&mut read.work).await, None => std::future::pending().await } } => {
                let read = self.feed_read.take().unwrap();
                self.finish_feed_read(read, result.unwrap_or(Err(Failure(ErrorCode::Failed, "The host worker failed.", None))))?;
            }
            result = async { match &mut self.action_work { Some((id, work)) => (*id, work.await), None => std::future::pending().await } } => {
                self.action_work = None;
                let (id, result) = result;
                self.reply(id, result.unwrap_or(Err(Failure(ErrorCode::Failed, "The host worker failed.", None))))?;
                self.start_action();
            }
            result = self.forks.join_next(), if !self.forks.is_empty() => {
                let (id, result) = result.ok_or("host worker failed")?.map_err(|_| "host worker failed")?;
                self.reply(id, result)?;
            }
            _ = async { match due { Some(due) => tokio::time::sleep_until(due).await, None => std::future::pending().await } }, if !overview_running => {
                self.overview_due = None;
                self.overview_broadcast = true;
                self.start_overview();
            }
            _ = async { match output_due { Some(due) => tokio::time::sleep_until(due).await, None => std::future::pending().await } } => {
                self.output_due = None;
            }
        }
        Ok(())
    }
}
fn overview_hash(overview: &Overview) -> Result<u64, &'static str> {
    use std::hash::{Hash, Hasher};
    let mut value = serde_json::to_value(overview).map_err(|_| "companion encoding failed")?;
    for (id, session) in &overview.sessions {
        let status =
            serde_json::to_value(&session.status).map_err(|_| "companion encoding failed")?;
        if let Some(fields) = value["sessions"][id].as_object_mut() {
            for key in status.as_object().unwrap().keys() {
                fields.remove(key);
            }
        }
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_vec(&value)
        .map_err(|_| "companion encoding failed")?
        .hash(&mut hash);
    Ok(hash.finish())
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
        "terminal/palette",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

#[cfg(test)]
pub(crate) mod tests;
