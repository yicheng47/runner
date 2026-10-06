use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use super::terminal::*;
use super::wire::{self, Binary, Frame};
use super::*;
use crate::app_paths::IpcEndpoint;

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const QUEUE_CAPACITY: usize = 64;

#[cfg(unix)]
type Stream = std::os::unix::net::UnixStream;
#[cfg(windows)]
#[path = "socket_windows.rs"]
mod windows;
#[cfg(windows)]
use windows::Stream;

#[derive(Debug)]
pub enum ConnectError {
    NotRunning,
    Blocked,
    Mismatch(wire::Welcome),
    Protocol(String),
}
impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRunning => f.write_str("runnerd is not running"),
            Self::Blocked => f.write_str("runnerd connection was denied"),
            Self::Mismatch(_) => f.write_str("runnerd build mismatch"),
            Self::Protocol(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for ConnectError {}
fn io_error(error: io::Error) -> ConnectError {
    if error.kind() == io::ErrorKind::PermissionDenied {
        ConnectError::Blocked
    } else {
        ConnectError::NotRunning
    }
}

fn open(endpoint: &IpcEndpoint, deadline: Instant) -> io::Result<Stream> {
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let stream = unsafe { Stream::from_raw_fd(fd) };
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        stream.set_nonblocking(true)?;
        use std::os::unix::ffi::OsStrExt;
        let bytes = endpoint.0.as_os_str().as_bytes();
        let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        addr.sun_family = libc::AF_UNIX as _;
        if bytes.len() >= addr.sun_path.len() {
            return Err(wire::invalid("socket path too long"));
        }
        for (dest, src) in addr.sun_path.iter_mut().zip(bytes) {
            *dest = *src as _;
        }
        let result = unsafe {
            libc::connect(
                stream.as_raw_fd(),
                &addr as *const _ as *const libc::sockaddr,
                std::mem::size_of_val(&addr) as _,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EINPROGRESS) {
                return Err(error);
            }
            let mut poll = libc::pollfd {
                fd,
                events: libc::POLLOUT,
                revents: 0,
            };
            let ms = deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(i32::MAX as u128) as i32;
            if unsafe { libc::poll(&mut poll, 1, ms) } <= 0 {
                return Err(io::ErrorKind::TimedOut.into());
            }
            if let Some(error) = stream.take_error()? {
                return Err(error);
            }
        }
        stream.set_nonblocking(false)?;
        Ok(stream)
    }
    #[cfg(windows)]
    loop {
        match Stream::open(&endpoint.0) {
            Ok(file) => return Ok(file),
            Err(error) if error.raw_os_error() == Some(231) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(error),
        }
    }
}

struct PendingAttach {
    done: mpsc::Sender<Result<TerminalSnapshot, ClientError>>,
    snapshot: Option<TerminalSnapshot>,
    queue: Arc<Frames>,
    subscriber: Arc<AtomicU64>,
}
#[derive(Default)]
struct FrameState {
    frames: VecDeque<TerminalFrame>,
    closed: bool,
    output: Vec<u8>,
}
#[derive(Default)]
struct Frames {
    state: Mutex<FrameState>,
    ready: Condvar,
}
impl Frames {
    fn push(&self, frame: TerminalFrame) {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return;
        }
        if state.frames.len() == QUEUE_CAPACITY {
            state.frames.clear();
            state.output.clear();
            state.frames.push_back(TerminalFrame::Resync);
            state.closed = true;
        } else {
            state.frames.push_back(frame);
        }
        self.ready.notify_all();
    }
    fn close(&self) {
        self.state.lock().unwrap().closed = true;
        self.ready.notify_all();
    }
}
struct Subscription {
    queue: Arc<Frames>,
    owner: Weak<SocketTransport>,
    id: u64,
}
impl Drop for Subscription {
    fn drop(&mut self) {
        self.queue.close();
        if let Some(owner) = self.owner.upgrade() {
            owner.frames.lock().unwrap().remove(&self.id);
            let _ = owner.send(wire::numbered(wire::DETACH, self.id));
        }
    }
}
impl TerminalSubscription for Subscription {
    fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
        let queue = self.queue.clone();
        Arc::new(move || queue.close())
    }
    fn recv(&mut self) -> Result<TerminalFrame, ClientError> {
        let mut state = self.queue.state.lock().unwrap();
        loop {
            if let Some(frame) = state.frames.pop_front() {
                return Ok(frame);
            }
            if state.closed {
                return Err(ClientError::msg("terminal connection closed"));
            }
            state = self.queue.ready.wait(state).unwrap();
        }
    }
}

pub struct SocketTransport {
    write: mpsc::SyncSender<Frame>,
    next: AtomicU64,
    one_way: Mutex<()>,
    pending: Mutex<HashMap<u64, mpsc::Sender<Result<Response, ClientError>>>>,
    attaching: Mutex<HashMap<u64, PendingAttach>>,
    frames: Mutex<HashMap<u64, Weak<Frames>>>,
    events: tokio::sync::broadcast::Sender<wire::Event>,
    launch_resume_report: Mutex<Option<AutoResumeReport>>,
    observer: Mutex<Option<Weak<dyn TerminalLifecycle>>>,
    closed: AtomicBool,
    timeout: Duration,
    interrupt: Stream,
    pub welcome: wire::Welcome,
    own: Weak<Self>,
}
impl SocketTransport {
    pub fn connect(endpoint: &IpcEndpoint, hello: wire::Hello) -> Result<Arc<Self>, ConnectError> {
        Self::connect_with_timeout(endpoint, hello, REQUEST_TIMEOUT)
    }
    pub fn connect_with_timeout(
        endpoint: &IpcEndpoint,
        hello: wire::Hello,
        timeout: Duration,
    ) -> Result<Arc<Self>, ConnectError> {
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut stream = open(endpoint, deadline).map_err(io_error)?;
        #[cfg(windows)]
        stream.set_timeout(Some(deadline.saturating_duration_since(Instant::now())));
        #[cfg(unix)]
        {
            stream
                .set_read_timeout(Some(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .max(Duration::from_millis(1)),
                ))
                .map_err(io_error)?;
            stream
                .set_write_timeout(Some(Duration::from_millis(500)))
                .map_err(io_error)?;
        }
        Frame::json(wire::HELLO, &hello)
            .and_then(|frame| frame.write(&mut stream))
            .map_err(io_error)?;
        let frame = Frame::read(&mut stream).map_err(io_error)?;
        let welcome: wire::Welcome = frame
            .decode()
            .map_err(|error| ConnectError::Protocol(error.to_string()))?;
        if frame.kind == wire::MISMATCH {
            return Err(ConnectError::Mismatch(welcome));
        }
        if frame.kind != wire::WELCOME {
            return Err(ConnectError::Protocol("invalid runnerd handshake".into()));
        }
        #[cfg(windows)]
        stream.set_timeout(None);
        #[cfg(unix)]
        {
            stream.set_read_timeout(None).map_err(io_error)?;
            stream.set_write_timeout(Some(timeout)).map_err(io_error)?;
        }
        let writer = stream.try_clone().map_err(io_error)?;
        #[cfg(windows)]
        let writer = writer.with_timeout(timeout);
        let interrupt = stream.try_clone().map_err(io_error)?;
        let (write, requests) = mpsc::sync_channel(256);
        let (events, _) = tokio::sync::broadcast::channel(8192);
        let transport = Arc::new_cyclic(|own| Self {
            write,
            next: AtomicU64::new(1),
            one_way: Mutex::default(),
            pending: Mutex::default(),
            attaching: Mutex::default(),
            frames: Mutex::default(),
            events,
            launch_resume_report: Mutex::default(),
            observer: Mutex::default(),
            closed: AtomicBool::new(false),
            timeout,
            interrupt,
            welcome,
            own: own.clone(),
        });
        let weak = Arc::downgrade(&transport);
        std::thread::Builder::new()
            .name("runnerd-reader".into())
            .spawn(move || {
                let mut reader = stream;
                loop {
                    let frame = Frame::read(&mut reader);
                    let Some(owner) = weak.upgrade() else {
                        break;
                    };
                    match frame.and_then(|frame| owner.receive(frame)) {
                        Ok(()) => (),
                        Err(error) => {
                            owner.disconnect(&error.to_string());
                            break;
                        }
                    }
                }
            })
            .map_err(io_error)?;
        let weak = Arc::downgrade(&transport);
        std::thread::Builder::new()
            .name("runnerd-writer".into())
            .spawn(move || {
                let mut writer = writer;
                while let Ok(frame) = requests.recv() {
                    if let Err(error) = frame.write(&mut writer) {
                        if let Some(owner) = weak.upgrade() {
                            owner.disconnect(&error.to_string());
                        }
                        break;
                    }
                }
            })
            .map_err(io_error)?;
        let weak = Arc::downgrade(&transport);
        let mut lifecycle = transport.events.subscribe();
        std::thread::Builder::new()
            .name("runnerd-lifecycle".into())
            .spawn(move || loop {
                let received = lifecycle.blocking_recv();
                let Some(owner) = weak.upgrade() else {
                    break;
                };
                let event = match received {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                };
                if let Some(event) = event.event {
                    let observer = owner
                        .observer
                        .lock()
                        .unwrap()
                        .as_ref()
                        .and_then(Weak::upgrade);
                    if let Some(observer) = observer {
                        observer.event(event);
                    }
                }
            })
            .map_err(io_error)?;
        Ok(transport)
    }
    pub fn client(self: &Arc<Self>) -> DaemonClient {
        DaemonClient::new(self.clone())
    }
    pub fn launch_resume_report(&self) -> Option<AutoResumeReport> {
        self.launch_resume_report.lock().unwrap().clone()
    }
    pub fn event_stream(&self) -> tokio::sync::broadcast::Receiver<wire::Event> {
        self.events.subscribe()
    }
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
    fn send(&self, frame: Frame) -> Result<(), ClientError> {
        if self.is_closed() {
            return Err(ClientError::msg("runnerd connection closed"));
        }
        self.write
            .try_send(frame)
            .map_err(|error| ClientError::msg(format!("runnerd writer: {error}")))
    }
    pub fn shutdown(&self, stop_sessions: bool) -> Result<(), ClientError> {
        let mut frame =
            Frame::json(wire::SHUTDOWN, &wire::Shutdown { stop_sessions }).map_err(client_error)?;
        let deadline = Instant::now() + self.timeout;
        loop {
            if self.is_closed() {
                return Err(ClientError::msg("runnerd connection closed"));
            }
            match self.write.try_send(frame) {
                Ok(()) => return Ok(()),
                Err(mpsc::TrySendError::Full(queued)) if Instant::now() < deadline => {
                    frame = queued;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => {
                    return Err(ClientError::msg(format!(
                        "runnerd shutdown writer: {error}"
                    )))
                }
            }
        }
    }
    fn disconnect(&self, reason: &str) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        interrupt(&self.interrupt);
        for (_, pending) in self.pending.lock().unwrap().drain() {
            let _ = pending.send(Err(ClientError::msg(reason)));
        }
        for (_, pending) in self.attaching.lock().unwrap().drain() {
            let _ = pending.done.send(Err(ClientError::msg(reason)));
        }
        for (_, queue) in self.frames.lock().unwrap().drain() {
            if let Some(queue) = queue.upgrade() {
                queue.close();
            }
        }
        let _ = self.events.send(wire::Event {
            event: Some(ClientEvent {
                name: "daemon/disconnected".into(),
                payload: serde_json::json!({"message":reason}),
            }),
            lagged: None,
        });
    }
    fn receive(&self, frame: Frame) -> io::Result<()> {
        match frame.kind {
            wire::REQUEST_ERROR => {
                let error: wire::RequestError = frame.decode()?;
                if let Some(pending) = self.pending.lock().unwrap().remove(&error.id) {
                    let _ = pending.send(Err(ClientError::msg(error.message)));
                }
            }
            wire::RESPONSE => {
                let reply: wire::Reply = frame
                    .decode()
                    .map_err(|_| wire::invalid(wire::PROTOCOL_MISMATCH))?;
                if let Some(pending) = self.pending.lock().unwrap().remove(&reply.id) {
                    let _ = pending.send(Ok(reply.response));
                }
            }
            wire::EVENT => {
                let event: wire::Event = frame.decode()?;
                if let Some(report) = event
                    .event
                    .as_ref()
                    .filter(|event| event.name == "daemon/launch-resumed")
                {
                    *self.launch_resume_report.lock().unwrap() =
                        serde_json::from_value(report.payload.clone()).ok();
                }
                let _ = self.events.send(event);
            }
            wire::ATTACH_ERROR => {
                let mut binary = Binary(&frame.payload);
                let id = binary.u64()?;
                if let Some(pending) = self.attaching.lock().unwrap().remove(&id) {
                    let _ = pending
                        .done
                        .send(Err(ClientError::msg(String::from_utf8_lossy(binary.0))));
                }
            }
            wire::SNAPSHOT => {
                let mut binary = Binary(&frame.payload);
                let id = binary.u64()?;
                let subscriber = binary.u64()?;
                let seq = binary.u64()?;
                let cols = binary.u16()?;
                let rows = binary.u16()?;
                let unfinished_len = binary.u64()? as usize;
                let preceding_char = char::from_u32(binary.u32()?);
                if let Some(pending) = self.attaching.lock().unwrap().get_mut(&id) {
                    pending.subscriber.store(subscriber, Ordering::Relaxed);
                    pending.snapshot = Some(TerminalSnapshot {
                        seq,
                        cols,
                        rows,
                        bytes: Vec::new(),
                        unfinished_len,
                        preceding_char,
                    });
                }
            }
            wire::SNAPSHOT_DATA => {
                let mut binary = Binary(&frame.payload);
                let id = binary.u64()?;
                let last = binary.take(1)?[0] != 0;
                let mut attaching = self.attaching.lock().unwrap();
                if let Some(pending) = attaching.get_mut(&id) {
                    let snapshot = pending
                        .snapshot
                        .as_mut()
                        .ok_or_else(|| wire::invalid("snapshot without header"))?;
                    if snapshot.bytes.len() + binary.0.len() > wire::MAX_FRAME {
                        return Err(wire::invalid("snapshot too large"));
                    }
                    snapshot.bytes.extend_from_slice(binary.0);
                    if last {
                        let pending = attaching.remove(&id).unwrap();
                        self.frames
                            .lock()
                            .unwrap()
                            .insert(id, Arc::downgrade(&pending.queue));
                        let _ = pending.done.send(Ok(pending.snapshot.unwrap()));
                    }
                }
            }
            wire::OUTPUT | wire::RESIZED | wire::RESYNC | wire::TERMINAL_CLOSED => {
                let mut binary = Binary(&frame.payload);
                let id = binary.u64()?;
                let queue = self.frames.lock().unwrap().get(&id).and_then(Weak::upgrade);
                if let Some(queue) = queue {
                    match frame.kind {
                        wire::OUTPUT => {
                            let seq = binary.u64()?;
                            let last = binary.take(1)?[0] != 0;
                            let mut state = queue.state.lock().unwrap();
                            if !state.closed {
                                if state.output.len() + binary.0.len() > wire::MAX_FRAME {
                                    return Err(wire::invalid("output too large"));
                                }
                                state.output.extend_from_slice(binary.0);
                                if last {
                                    let bytes = std::mem::take(&mut state.output);
                                    drop(state);
                                    queue.push(TerminalFrame::Output { seq, bytes });
                                }
                            }
                        }
                        wire::RESIZED => {
                            queue.push(TerminalFrame::Resized {
                                seq: binary.u64()?,
                                cols: binary.u16()?,
                                rows: binary.u16()?,
                            });
                        }
                        wire::RESYNC => queue.push(TerminalFrame::Resync),
                        _ => queue.close(),
                    }
                }
            }
            _ => return Err(wire::invalid("unknown runnerd frame")),
        }
        Ok(())
    }
}
fn client_error(error: impl std::fmt::Display) -> ClientError {
    ClientError::msg(error.to_string())
}
fn interrupt(stream: &Stream) {
    #[cfg(unix)]
    let _ = stream.shutdown(std::net::Shutdown::Both);
    #[cfg(windows)]
    {
        stream.interrupt();
    }
}
impl Drop for SocketTransport {
    fn drop(&mut self) {
        interrupt(&self.interrupt);
    }
}
impl Transport for SocketTransport {
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (done, result) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, done);
        let response = Frame::json(wire::REQUEST, &wire::Call { id, request })
            .map_err(client_error)
            .and_then(|frame| self.send(frame))
            .and_then(|()| {
                result
                    .recv_timeout(self.timeout)
                    .map_err(|_| ClientError::msg("runnerd request deadline exceeded"))?
            });
        self.pending.lock().unwrap().remove(&id);
        response
    }
    fn subscribe(&self) -> Box<dyn EventSubscription> {
        Box::new(Events(self.events.subscribe()))
    }
    fn observe_terminals(&self, observer: Weak<dyn TerminalLifecycle>) {
        *self.observer.lock().unwrap() = Some(observer);
    }
    fn attach(&self, session_id: &str) -> Result<TerminalAttachment, ClientError> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (done, result) = mpsc::channel();
        let queue = Arc::new(Frames::default());
        let subscriber = Arc::new(AtomicU64::new(0));
        self.attaching.lock().unwrap().insert(
            id,
            PendingAttach {
                done,
                snapshot: None,
                queue: queue.clone(),
                subscriber: subscriber.clone(),
            },
        );
        let mut frame = wire::numbered(wire::ATTACH, id);
        frame
            .payload
            .extend(wire::session_payload(session_id).map_err(client_error)?);
        let snapshot = self.send(frame).and_then(|()| {
            result
                .recv_timeout(self.timeout)
                .map_err(|_| ClientError::msg("runnerd attach deadline exceeded"))?
        });
        self.attaching.lock().unwrap().remove(&id);
        match snapshot {
            Ok(snapshot) => Ok(TerminalAttachment {
                subscriber_id: subscriber.load(Ordering::Relaxed),
                snapshot,
                frames: Box::new(Subscription {
                    queue,
                    owner: self.own.clone(),
                    id,
                }),
            }),
            Err(error) => {
                let _ = self.send(wire::numbered(wire::DETACH, id));
                Err(error)
            }
        }
    }
    fn input(&self, session_id: &str, bytes: &[u8]) -> Result<(), ClientError> {
        let _ordered = self.one_way.lock().unwrap();
        let prefix = wire::session_payload(session_id).map_err(client_error)?;
        for chunk in bytes.chunks(wire::TERMINAL_CHUNK - prefix.len()) {
            let mut payload = prefix.clone();
            payload.extend_from_slice(chunk);
            self.send(Frame {
                kind: wire::INPUT,
                payload,
            })?;
        }
        Ok(())
    }
    fn resize(&self, session_id: &str, subscriber: u64, cols: u16, rows: u16) {
        let _ordered = self.one_way.lock().unwrap();
        if let Ok(mut payload) = wire::session_payload(session_id) {
            payload.extend_from_slice(&subscriber.to_le_bytes());
            payload.extend_from_slice(&cols.to_le_bytes());
            payload.extend_from_slice(&rows.to_le_bytes());
            if let Err(error) = self.send(Frame {
                kind: wire::RESIZE,
                payload,
            }) {
                let _ = self.events.send(wire::Event {
                    event: Some(ClientEvent { name: "session/input-error".into(), payload: serde_json::json!({"session_id": session_id, "message": error.to_string()}) }), lagged: None,
                });
            }
        }
    }
}
struct Events(tokio::sync::broadcast::Receiver<wire::Event>);
impl EventSubscription for Events {
    fn recv(
        &mut self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ClientEvent, EventError>> + Send + '_>,
    > {
        Box::pin(async move {
            match self.0.recv().await {
                Ok(event) => match event.event {
                    Some(event) => Ok(event),
                    None => Err(EventError::Lagged(event.lagged.unwrap_or(0))),
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    Err(EventError::Lagged(count))
                }
                Err(_) => Err(EventError::Closed),
            }
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn protocol_errors_reach_pending_calls_as_restart_messages() {
        for kind in [wire::REQUEST_ERROR, wire::RESPONSE] {
            let root = tempfile::tempdir().unwrap();
            let endpoint = IpcEndpoint(root.path().join("runnerd.sock"));
            let listener = std::os::unix::net::UnixListener::bind(&endpoint.0).unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                assert_eq!(Frame::read(&mut stream).unwrap().kind, wire::HELLO);
                Frame::json(
                    wire::WELCOME,
                    &wire::Welcome {
                        exe_sha256: "hash".into(),
                        pid: 1,
                        started_at: "now".into(),
                    },
                )
                .unwrap()
                .write(&mut stream)
                .unwrap();
                let call: wire::Call = Frame::read(&mut stream).unwrap().decode().unwrap();
                let payload = if kind == wire::REQUEST_ERROR {
                    serde_json::json!({"id": call.id, "message": wire::PROTOCOL_MISMATCH})
                } else {
                    serde_json::json!({"id": call.id, "response": {"future_operation": {}}})
                };
                Frame::json(kind, &payload)
                    .unwrap()
                    .write(&mut stream)
                    .unwrap();
            });
            let socket = SocketTransport::connect(
                &endpoint,
                wire::Hello {
                    exe_sha256: "hash".into(),
                    client: "test".into(),
                },
            )
            .unwrap();
            assert_eq!(
                socket.client().role_list().unwrap_err().message,
                wire::PROTOCOL_MISMATCH
            );
            server.join().unwrap();
        }
    }
    #[test]
    fn pending_request_has_a_deadline_without_blocking_another_call() {
        let root = tempfile::tempdir().unwrap();
        let endpoint = IpcEndpoint(root.path().join("runnerd.sock"));
        let listener = std::os::unix::net::UnixListener::bind(&endpoint.0).unwrap();
        let (ready, started) = mpsc::channel();
        let (close, closing) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            assert_eq!(Frame::read(&mut stream).unwrap().kind, wire::HELLO);
            Frame::json(
                wire::WELCOME,
                &wire::Welcome {
                    exe_sha256: "hash".into(),
                    pid: 1,
                    started_at: "now".into(),
                },
            )
            .unwrap()
            .write(&mut stream)
            .unwrap();
            let _: wire::Call = Frame::read(&mut stream).unwrap().decode().unwrap();
            ready.send(()).unwrap();
            let second: wire::Call = Frame::read(&mut stream).unwrap().decode().unwrap();
            Frame::json(
                wire::RESPONSE,
                &wire::Reply {
                    id: second.id,
                    response: Response::role_list(Ok(Vec::new())),
                },
            )
            .unwrap()
            .write(&mut stream)
            .unwrap();
            closing.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let socket = SocketTransport::connect_with_timeout(
            &endpoint,
            wire::Hello {
                exe_sha256: "hash".into(),
                client: "test".into(),
            },
            Duration::from_millis(200),
        )
        .unwrap();
        let first = socket.client();
        let call = std::thread::spawn(move || first.role_list());
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(socket.client().role_list().unwrap().is_empty());
        let error = call.join().unwrap().unwrap_err();
        assert!(error.message.contains("deadline"), "{error:?}");
        close.send(()).unwrap();
        server.join().unwrap();
        assert!(socket.client().role_list().is_err());
    }
    #[test]
    fn client_terminal_overflow_requests_resync() {
        let queue = Frames::default();
        for seq in 0..=QUEUE_CAPACITY as u64 {
            queue.push(TerminalFrame::Output {
                seq,
                bytes: vec![42],
            });
        }
        let mut state = queue.state.lock().unwrap();
        assert!(state.closed);
        assert_eq!(state.frames.len(), 1);
        assert!(matches!(
            state.frames.pop_front(),
            Some(TerminalFrame::Resync)
        ));
    }
}
