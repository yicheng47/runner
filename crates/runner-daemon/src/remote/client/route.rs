use super::*;
use futures_util::{SinkExt, StreamExt};
use runner_core::companion::channel::{
    self, BrowserConnect, BrowserPair, NoiseKeys, Records, Reject,
};
use std::{
    collections::{HashSet, VecDeque},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio_tungstenite::tungstenite::Message;

const PING_INTERVAL: Duration = Duration::from_secs(5);
const PING_TIMEOUT: Duration = Duration::from_secs(10);
const LATENCY_SAMPLES: usize = 3;

pub(super) enum Command {
    Request(c::ClientMessage, oneshot::Sender<Result<serde_json::Value>>),
    Input(u64, String, Vec<u8>),
    Follow(u64, bool),
    Attach(RemoteTerminal, oneshot::Sender<Result<TerminalAttachment>>),
    Detach(String, u64),
}
#[derive(Clone)]
pub(super) struct Handle {
    pub fingerprint: String,
    pub computer: String,
    pub tx: mpsc::Sender<Command>,
    pub cancel: CancellationToken,
    pub ready: Arc<AtomicBool>,
    pub features: Arc<std::sync::Mutex<Vec<String>>>,
    generation: Arc<AtomicU64>,
    next: Arc<AtomicU64>,
    failure: Arc<std::sync::Mutex<Option<String>>>,
}
impl Handle {
    pub fn failure(&self) -> String {
        self.failure
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| "Remote channel closed".into())
    }
    pub fn next_id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }
    pub async fn send(&self, command: Command) -> Result<()> {
        self.tx.send(command).await.map_err(|_| self.failure())
    }
    pub async fn request(
        &self,
        message: impl FnOnce(u64) -> c::ClientMessage,
    ) -> Result<serde_json::Value> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (reply, receive) = oneshot::channel();
        let message = message(id);
        let timeout = runner_core::protocol::socket::companion_request_timeout(&message);
        self.send(Command::Request(message, reply)).await?;
        tokio::time::timeout(timeout + Duration::from_secs(5), receive)
            .await
            .map_err(|_| "Remote request timed out")?
            .map_err(|_| self.failure())?
    }
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    pub async fn input(&self, generation: u64, session: String, bytes: Vec<u8>) -> Result<()> {
        if generation != self.generation() {
            return Err("Remote route changed; input was discarded.".into());
        }
        self.tx
            .try_send(Command::Input(generation, session, bytes))
            .map_err(|_| "Remote input queue is full or closed; input was discarded.")?;
        Ok(())
    }
}
fn input_error(client: &Client, computer: &str, session: &str, message: &str) {
    client.0.events.emit(
        "session/input-error",
        &serde_json::json!({
            "session_id":remote::terminal_address(computer, session), "message":message,
        }),
    );
}
/// Round-trip timing for one open route. Each connection builds its own, so a reconnect starts
/// with no samples and a pong from an earlier route matches nothing.
struct Latency {
    next_ping: Instant,
    pings: VecDeque<(u64, Instant)>,
    samples: VecDeque<u64>,
}
impl Latency {
    fn new(now: Instant) -> Self {
        Self {
            next_ping: now,
            pings: VecDeque::new(),
            samples: VecDeque::new(),
        }
    }
    fn deadline(&self) -> Instant {
        self.pings.front().map_or(self.next_ping, |(_, sent)| {
            self.next_ping.min(*sent + PING_TIMEOUT)
        })
    }
    fn ping_due(&self, now: Instant) -> bool {
        now >= self.next_ping
    }
    fn sent(&mut self, id: u64, now: Instant) {
        self.pings.push_back((id, now));
        self.next_ping = now + PING_INTERVAL;
    }
    /// Counts each ping unanswered for the whole timeout as one sample of that length.
    fn expire(&mut self, now: Instant) -> Option<u64> {
        let mut median = None;
        while self
            .pings
            .front()
            .is_some_and(|(_, sent)| now >= *sent + PING_TIMEOUT)
        {
            self.pings.pop_front();
            median = Some(self.sample(PING_TIMEOUT.as_millis() as u64));
        }
        median
    }
    fn pong(&mut self, id: u64, now: Instant) -> Option<u64> {
        let index = self.pings.iter().position(|(ping, _)| *ping == id)?;
        let (_, sent) = self.pings.remove(index)?;
        Some(self.sample(now.duration_since(sent).as_millis() as u64))
    }
    fn sample(&mut self, ms: u64) -> u64 {
        if self.samples.len() == LATENCY_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(ms);
        let mut sorted: Vec<_> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        sorted[(sorted.len() - 1) / 2]
    }
}
fn latency(client: &Client, computer: &str, ms: u64) {
    client.0.events.emit(
        "remote/latency",
        &serde_json::json!({"computer": computer, "ms": ms}),
    );
}
/// Reassembles the pieces of each terminal's OSC 52 writes, per target, on one route.
#[derive(Default)]
struct Clipboards(HashMap<(String, String), String>);
impl Clipboards {
    /// The whole write once its last piece arrives. A piece that does not
    /// continue the write in progress, or would take it past the cap, drops it.
    fn receive(&mut self, mut push: c::ClipboardPush) -> Option<c::ClipboardPush> {
        let key = (push.session.clone(), push.target.clone());
        let partial = self.0.remove(&key);
        let mut text = if push.offset == 0 {
            String::new()
        } else {
            partial.filter(|text| text.len() as u64 == push.offset)?
        };
        if text.len() + push.text.len() > runner_terminal::terminal::MAX_CLIPBOARD_BYTES {
            return None;
        }
        text.push_str(&push.text);
        if push.more {
            self.0.insert(key, text);
            return None;
        }
        push.text = text;
        push.offset = 0;
        Some(push)
    }
}
fn clipboard(client: &Client, computer: &str, push: c::ClipboardPush) {
    client.0.events.emit(
        "session/clipboard",
        &serde_json::json!({
            "session_id": remote::terminal_address(computer, &push.session),
            "target": push.target,
            "text": push.text,
        }),
    );
}
pub(super) fn spawn(
    client: Client,
    computer: Computer,
    context: Context,
    initial: Option<(transport::Socket, Records, c::Welcome)>,
) -> Handle {
    let (tx, rx) = mpsc::channel(32);
    let handle = Handle {
        fingerprint: computer.fingerprint.clone(),
        computer: computer.id.clone(),
        tx,
        cancel: client.0.cancel.child_token(),
        ready: Arc::new(AtomicBool::new(false)),
        features: Default::default(),
        generation: Arc::new(AtomicU64::new(0)),
        next: Arc::new(AtomicU64::new(1)),
        failure: Default::default(),
    };
    let task = handle.clone();
    tokio::spawn(async move {
        run(client, computer, context, task.clone(), rx, initial).await;
        task.ready.store(false, Ordering::Release);
        task.cancel.cancel();
    });
    handle
}
fn keys(key: &remote::HostKey) -> NoiseKeys<'_> {
    NoiseKeys {
        private: &key.private,
        fixed_ephemeral: None,
    }
}
fn rejection(bytes: &[u8]) -> Option<&'static str> {
    let reason = channel::body(bytes, channel::Kind::Reject).ok()?;
    [
        Reject::NoPin,
        Reject::PinLocked,
        Reject::WrongPin,
        Reject::UnknownDevice,
        Reject::Busy,
        Reject::BadHandshake,
    ]
    .into_iter()
    .find(|r| r.reason().as_bytes() == reason)
    .map(Reject::reason)
}
fn bad_handshake(_: Reject) -> String {
    "Remote channel: badHandshake.".into()
}
async fn send(socket: &mut transport::Socket, bytes: Vec<u8>) -> Result<()> {
    tokio::time::timeout(
        Duration::from_secs(10),
        socket.send(Message::Binary(bytes.into())),
    )
    .await
    .map_err(|_| "Remote route write timed out")?
    .map_err(|_| "Remote route closed".into())
}
fn request_error(error: &c::ErrorMessage) -> &'static str {
    match error.message.as_str() {
        "The project has running chats or missions." => {
            "The project has running chats or missions."
        }
        "Choose an accessible folder on the host." => "Choose an accessible folder on the host.",
        "That question is no longer pending." => "That question is no longer pending.",
        "The host does not serve that request." => "The host does not serve that request.",
        _ => "The remote computer refused the request.",
    }
}
fn close_error(code: u16) -> String {
    let reason = match code {
        4401 => "unauthenticated",
        4404 => "notFound",
        4409 => "offline",
        4426 => "channelsUnsupported",
        4429 => "tooManyChannels",
        _ => "routeClosed",
    };
    format!("Remote route closed: {reason} ({code}).")
}
async fn read(socket: &mut transport::Socket) -> Result<Vec<u8>> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Binary(bytes))) => {
                if let Some(reason) = rejection(&bytes) {
                    if reason == "unknownDevice" {
                        return Err("Remote channel: unknownDevice. This computer is not recognized. Verify the host fingerprint before choosing Pair again; the saved pairing is kept until you explicitly forget it.".into());
                    }
                    return Err(format!("Remote channel: {reason}."));
                }
                return Ok(bytes.to_vec());
            }
            Some(Ok(Message::Ping(bytes))) => {
                socket
                    .send(Message::Pong(bytes))
                    .await
                    .map_err(|_| "Remote route closed")?;
            }
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(Message::Close(frame))) => {
                return Err(close_error(frame.map(|f| f.code.into()).unwrap_or(1006)))
            }
            _ => return Err("Remote route closed".into()),
        }
    }
}
async fn flush(socket: &mut transport::Socket, records: &mut Records) -> Result<()> {
    while let Some(frame) = records.next_frame().map_err(bad_handshake)? {
        send(socket, frame).await?;
    }
    Ok(())
}
async fn welcome(socket: &mut transport::Socket, records: &mut Records) -> Result<c::Welcome> {
    records
        .queue(
            serde_json::to_vec(&c::ClientMessage::Hello(c::Hello {
                versions: vec![1],
                client: "runner".into(),
                build: env!("CARGO_PKG_VERSION").into(),
            }))
            .map_err(|_| "Could not encode companion hello")?,
        )
        .map_err(bad_handshake)?;
    loop {
        flush(socket, records).await?;
        if let Some(bytes) = records
            .receive(&read(socket).await?)
            .map_err(bad_handshake)?
        {
            let message: c::HostMessage =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid companion welcome")?;
            match message {
                c::HostMessage::Welcome(w) if w.version == 1 => {
                    records.handled();
                    flush(socket, records).await?;
                    return Ok(w);
                }
                c::HostMessage::Error(_) => {
                    return Err(
                        "The remote computer does not support this companion protocol version."
                            .into(),
                    )
                }
                _ => return Err("Invalid companion welcome".into()),
            }
        }
    }
}
pub(super) async fn pair(
    mut socket: transport::Socket,
    key: &remote::HostKey,
    pin: &Secret,
    name: &str,
) -> Result<(transport::Socket, Records, Vec<u8>, c::Welcome)> {
    tokio::time::timeout(Duration::from_secs(20), async {
        let (pair, first) = BrowserPair::start(keys(key), &pin.0, rand::random(), rand::random())
            .map_err(bad_handshake)?;
        send(&mut socket, first).await?;
        let (transport, finish, public) = pair
            .finish(&read(&mut socket).await?, name)
            .map_err(bad_handshake)?;
        send(&mut socket, finish).await?;
        let mut records = Records::new(transport);
        let welcome = welcome(&mut socket, &mut records).await?;
        Ok((socket, records, public, welcome))
    })
    .await
    .map_err(|_| "Pairing timed out; enter the PIN again to retry.".to_owned())?
}
struct ConnectFailure {
    message: String,
    after: Option<Duration>,
    fatal: bool,
}
impl From<String> for ConnectFailure {
    fn from(message: String) -> Self {
        let fatal = ["unknownDevice", "pinned", "badHandshake", "Pair with"]
            .iter()
            .any(|s| message.contains(s));
        Self {
            message,
            after: None,
            fatal,
        }
    }
}
impl From<&str> for ConnectFailure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
async fn connect(
    client: &Client,
    computer: &Computer,
    context: &Context,
) -> std::result::Result<(transport::Socket, Records, c::Welcome), ConnectFailure> {
    let keys = client.keys().await?;
    let key = keys
        .0
        .get(&computer.fingerprint)
        .ok_or("Pair with this computer first.")?;
    if remote::fingerprint(&key.public) != computer.fingerprint {
        return Err("The pinned computer key does not match.".into());
    }
    let private = remote::HostKey::from_private(&key.private.0)?;
    let mut socket = client
        .service()
        .await?
        .open(context, &computer.id)
        .await
        .map_err(|f| ConnectFailure {
            after: if let remote::Retry::After(seconds) = f.retry {
                Some(Duration::from_secs(seconds))
            } else {
                None
            },
            fatal: matches!(f.retry, remote::Retry::Stop(_)) || f.code == "not_found",
            message: f.message,
        })?;
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        let (connect, first) =
            BrowserConnect::start(self::keys(&private), &key.public).map_err(bad_handshake)?;
        send(&mut socket, first).await?;
        let mut records = Records::new(
            connect
                .accept(&read(&mut socket).await?)
                .map_err(bad_handshake)?,
        );
        let welcome = welcome(&mut socket, &mut records).await?;
        Ok::<_, String>((socket, records, welcome))
    })
    .await
    .map_err(|_| "Remote connect timed out".to_owned())?;
    result.map_err(ConnectFailure::from)
}
struct Pending {
    subscription: Option<(String, bool)>,
    reply: oneshot::Sender<Result<serde_json::Value>>,
    deadline: Instant,
}
pub(super) struct Session {
    socket: transport::Socket,
    records: Records,
    outgoing: VecDeque<Vec<u8>>,
    queued: usize,
    pending: HashMap<u64, Pending>,
    latency: Latency,
}
impl Session {
    pub(super) fn queue(&mut self, message: c::ClientMessage) -> Result<()> {
        let bytes = serde_json::to_vec(&message).map_err(|_| "Could not encode remote request")?;
        if self.queued + self.records.queued_bytes() + bytes.len() > c::MAX_MESSAGE_SIZE {
            return Err("Remote request queue is full".into());
        }
        self.queued += bytes.len();
        self.outgoing.push_back(bytes);
        Ok(())
    }
    fn request(
        &mut self,
        message: c::ClientMessage,
        reply: oneshot::Sender<Result<serde_json::Value>>,
    ) {
        let Some(id) = message.id() else {
            let _ = reply.send(Err("Unsupported remote request".into()));
            return;
        };
        let subscription = match &message {
            c::ClientMessage::Follow(request) => Some((request.mission.clone(), true)),
            c::ClientMessage::Unfollow(request) => Some((request.mission.clone(), false)),
            _ => None,
        };
        let timeout = runner_core::protocol::socket::companion_request_timeout(&message);
        match self.queue(message) {
            Ok(()) => {
                self.pending.insert(
                    id,
                    Pending {
                        subscription,
                        reply,
                        deadline: Instant::now() + timeout,
                    },
                );
            }
            Err(e) => {
                let _ = reply.send(Err(e));
            }
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        for (_, pending) in self.pending.drain() {
            let _ = pending
                .reply
                .send(Err("Remote route dropped; request was not replayed.".into()));
        }
    }
}
async fn run(
    client: Client,
    computer: Computer,
    context: Context,
    handle: Handle,
    mut commands: mpsc::Receiver<Command>,
    mut initial: Option<(transport::Socket, Records, c::Welcome)>,
) {
    let mut followers = HashSet::new();
    let mut feeds = HashSet::new();
    let mut terminals = terminal::Terminals::default();
    let mut status = client.0.status.clone();
    let mut attempt = 0u32;
    let mut last_use = Instant::now();
    loop {
        let current = status.borrow().clone();
        if !signed_in(&current)
            || current.origin != context.origin
            || current.host_id.as_ref() != Some(&context.host_id)
        {
            break;
        }
        let connecting = async {
            match initial.take() {
                Some(connection) => Ok(connection),
                None => connect(&client, &computer, &context).await,
            }
        };
        let result = tokio::select! {
            biased;
            _ = handle.cancel.cancelled() => break,
            result = connecting => result,
        };
        let (socket, records, welcome) = match result {
            Ok(connection) => connection,
            Err(error) => {
                *handle.failure.lock().unwrap() = Some(error.message.clone());
                while let Ok(command) = commands.try_recv() {
                    match command {
                        Command::Request(message, reply) => {
                            terminals.viewport(&message);
                            let _ = reply.send(Err(error.message.clone()));
                        }
                        Command::Attach(_, reply) => {
                            let _ = reply.send(Err(error.message.clone()));
                        }
                        Command::Input(_, session, _) => {
                            input_error(
                                &client,
                                &computer.id,
                                &session,
                                "Remote route changed; input was discarded.",
                            );
                        }
                        Command::Follow(owner, follow) => {
                            if follow {
                                followers.insert(owner);
                            } else {
                                followers.remove(&owner);
                            }
                        }
                        Command::Detach(target, subscriber) => {
                            terminals.remove(&target, subscriber)
                        }
                    }
                }
                client.0.events.emit("remote/connection", &serde_json::json!({"computer":computer.id,"state":"reconnecting","error":error.message}));
                if error.fatal {
                    break;
                }
                if followers.is_empty() && terminals.is_empty() && commands.is_empty() {
                    break;
                }
                let delay = error
                    .after
                    .unwrap_or_else(|| Duration::from_secs((1u64 << attempt.min(5)).min(30)));
                attempt = attempt.saturating_add(1);
                tokio::select! { _ = handle.cancel.cancelled() => break, _ = tokio::time::sleep(delay) => {} }
                continue;
            }
        };
        let answers_ping = welcome.features.iter().any(|feature| feature == "ping");
        *handle.features.lock().unwrap() = welcome.features;
        handle.generation.fetch_add(1, Ordering::AcqRel);
        handle.ready.store(true, Ordering::Release);
        client.0.events.emit(
            "remote/connection",
            &serde_json::json!({"computer":computer.id,"state":"connected"}),
        );
        let mut session = Session {
            socket,
            records,
            outgoing: VecDeque::new(),
            queued: 0,
            pending: HashMap::new(),
            latency: Latency::new(Instant::now()),
        };
        if terminals.reattach(&mut session, &handle).is_err() {
            break;
        }
        if !followers.is_empty() {
            let id = handle.next.fetch_add(1, Ordering::Relaxed);
            let _ = session.queue(c::ClientMessage::Overview(c::OverviewRequest { id }));
        }
        let connected_at = Instant::now();
        let mut clipboards = Clipboards::default();
        let outcome: Result<()> = async {
            loop {
                if !session.records.sending() {
                    if let Some(bytes) = session.outgoing.pop_front() { session.queued -= bytes.len(); session.records.queue(bytes).map_err(bad_handshake)?; }
                }
                flush(&mut session.socket, &mut session.records).await?;
                let deadline = session.pending.values().map(|p| p.deadline).min().unwrap_or_else(|| Instant::now() + Duration::from_secs(60));
                tokio::select! {
                    biased;
                    _ = handle.cancel.cancelled() => return Err("Remote channel closed".into()),
                    changed = status.changed() => {
                        let current = status.borrow_and_update();
                        if changed.is_err() || !signed_in(&current) || current.origin != context.origin || current.host_id.as_ref() != Some(&context.host_id) { return Err("Remote account changed".into()); }
                    }
                    _ = tokio::time::sleep_until(last_use + Duration::from_secs(60)), if followers.is_empty() && feeds.is_empty() && terminals.is_empty() && session.pending.is_empty() => { handle.cancel.cancel(); return Ok(()); }
                    _ = tokio::time::sleep_until(deadline), if !session.pending.is_empty() => {
                        if session.pending.values().any(|p| p.deadline <= Instant::now()) { return Err("Remote request timed out".into()); }
                    }
                    _ = tokio::time::sleep_until(session.latency.deadline()), if answers_ping => {
                        let now = Instant::now();
                        if let Some(ms) = session.latency.expire(now) { latency(&client, &computer.id, ms); }
                        if session.latency.ping_due(now) {
                            let id = handle.next_id();
                            // A ping that cannot be queued goes unanswered like a lost one: it lowers the reading and never fails the route.
                            let _ = session.queue(c::ClientMessage::Ping(c::PingRequest { id }));
                            session.latency.sent(id, now);
                        }
                    }
                    command = commands.recv() => {
                        last_use = Instant::now();
                        match command.ok_or("Remote channel closed")? {
                            Command::Request(message, reply) => { terminals.viewport(&message); session.request(message, reply); },
                            Command::Input(generation, target, bytes) => {
                                let result = if generation != handle.generation.load(Ordering::Acquire) { Err("Remote route changed; input was discarded.".into()) }
                                else { session.queue(c::ClientMessage::Input(c::InputRequest { id: handle.next.fetch_add(1, Ordering::Relaxed), session: target.clone(), data: c::TerminalBytes(bytes) })) };
                                if let Err(error) = result { input_error(&client, &computer.id, &target, &error); }
                            }
                            Command::Follow(owner, follow) => {
                                if follow {
                                    if followers.insert(owner) { session.queue(c::ClientMessage::Overview(c::OverviewRequest { id: handle.next_id() }))?; }
                                } else { followers.remove(&owner); }
                            }
                            Command::Attach(config, reply) => terminals.attach(config, reply, &mut session, &handle)?,
                            Command::Detach(target, subscriber) => terminals.detach(&target, subscriber, &mut session, &handle)?,
                        }
                    }
                    bytes = read(&mut session.socket) => {
                        if let Some(bytes) = session.records.receive(&bytes?).map_err(bad_handshake)? {
                            let message: c::HostMessage = serde_json::from_slice(&bytes).map_err(|_| "Invalid companion message")?;
                            match message {
                                c::HostMessage::Result(result) => {
                                    if let Some(pending) = session.pending.remove(&result.id) {
                                        if let Some((mission,follow))=pending.subscription { if follow { feeds.insert(mission); } else { feeds.remove(&mission); } }
                                        last_use = Instant::now(); let _ = pending.reply.send(Ok(result.result));
                                    }
                                    else if !followers.is_empty() { if let Ok(overview) = serde_json::from_value::<c::Overview>(result.result) { client.0.events.emit("remote/overview", &serde_json::json!({"computer":computer.id,"overview":overview})); } }
                                }
                                c::HostMessage::Error(error) => {
                                    if let Some(id) = error.id {
                                        if let Some(pending) = session.pending.remove(&id) { let _ = pending.reply.send(Err(request_error(&error).into())); }
                                        terminals.refused(id);
                                    } else { return Err("The remote computer refused the channel.".into()); }
                                }
                                c::HostMessage::FeedAppended(feed) => { client.0.events.emit("remote/feed", &serde_json::json!({"computer":computer.id,"feed":feed})); }
                                c::HostMessage::Overview(overview) => { if !followers.is_empty() { client.0.events.emit("remote/overview", &serde_json::json!({"computer":computer.id,"overview":overview})); } }
                                c::HostMessage::Status(status) => { if !followers.is_empty() { client.0.events.emit("remote/session-status", &serde_json::json!({"computer":computer.id,"status":status})); } }
                                c::HostMessage::Clipboard(push) => {
                                    if let Some(push) = clipboards.receive(push) { clipboard(&client, &computer.id, push); }
                                }
                                c::HostMessage::Pong(pong) => {
                                    if let Some(ms) = session.latency.pong(pong.id, Instant::now()) { latency(&client, &computer.id, ms); }
                                }
                                message => terminals.message(message, &handle)?,
                            }
                            session.records.handled();
                        }
                    }
                }
            }
        }.await;
        handle.ready.store(false, Ordering::Release);
        handle.generation.fetch_add(1, Ordering::AcqRel);
        terminals.disconnected(
            outcome
                .as_ref()
                .err()
                .map(String::as_str)
                .unwrap_or("Remote route closed"),
        );
        if let Err(error) = &outcome {
            *handle.failure.lock().unwrap() = Some(error.clone());
            for (_, pending) in session.pending.drain() {
                let _ = pending.reply.send(Err(error.clone()));
            }
        }
        drop(session);
        feeds.clear();
        // Commands already queued against the old route must fail, particularly input.
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Input(_, target, _) => {
                    input_error(
                        &client,
                        &computer.id,
                        &target,
                        "Remote route dropped; input was discarded.",
                    );
                }
                Command::Request(message, reply) => {
                    terminals.viewport(&message);
                    let _ =
                        reply.send(Err("Remote route dropped; request was not replayed.".into()));
                }
                Command::Attach(_, reply) => {
                    let _ = reply.send(Err("Remote route dropped; attach again.".into()));
                }
                Command::Follow(owner, follow) => {
                    if follow {
                        followers.insert(owner);
                    } else {
                        followers.remove(&owner);
                    }
                }
                Command::Detach(target, subscriber) => terminals.remove(&target, subscriber),
            }
        }
        if handle.cancel.is_cancelled() || outcome.is_ok() || !signed_in(&status.borrow()) {
            break;
        }
        if connected_at.elapsed() >= Duration::from_secs(60) {
            attempt = 0;
        }
        if let Err(error) = outcome {
            client.0.events.emit(
                "remote/connection",
                &serde_json::json!({"computer":computer.id,"state":"reconnecting","error":error}),
            );
        }
        if followers.is_empty() && terminals.is_empty() {
            break;
        }
        let delay = Duration::from_secs((1u64 << attempt.min(5)).min(30));
        attempt = attempt.saturating_add(1);
        tokio::select! { _ = handle.cancel.cancelled() => break, _ = tokio::time::sleep(delay) => {} }
    }
    commands.close();
    while let Ok(command) = commands.try_recv() {
        if let Command::Input(_, target, _) = command {
            input_error(
                &client,
                &computer.id,
                &target,
                "Remote route closed; input was discarded.",
            );
        }
    }
    terminals.close();
    client.0.events.emit(
        "remote/connection",
        &serde_json::json!({"computer":computer.id,"state":"disconnected"}),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_reports_the_median_of_the_last_three_samples() {
        let start = Instant::now();
        let mut latency = Latency::new(start);
        let mut round_trip = |id, ms| {
            latency.sent(id, start);
            latency.pong(id, start + Duration::from_millis(ms)).unwrap()
        };
        assert_eq!(round_trip(1, 40), 40);
        assert_eq!(round_trip(2, 90), 40);
        assert_eq!(round_trip(3, 60), 60);
        assert_eq!(round_trip(4, 500), 90);
        assert_eq!(round_trip(5, 70), 70);
    }

    #[test]
    fn a_ping_unanswered_for_ten_seconds_counts_once_as_ten_seconds() {
        let start = Instant::now();
        let mut latency = Latency::new(start);
        latency.sent(1, start);
        assert_eq!(latency.pong(1, start + Duration::from_millis(20)), Some(20));
        latency.sent(2, start);
        assert_eq!(latency.deadline(), start + PING_INTERVAL);
        assert_eq!(latency.expire(start + Duration::from_millis(9_999)), None);
        assert_eq!(latency.expire(start + PING_TIMEOUT), Some(20));
        assert_eq!(latency.expire(start + PING_TIMEOUT), None);
        assert_eq!(latency.pong(2, start + Duration::from_secs(11)), None);

        latency.sent(3, start);
        assert_eq!(latency.expire(start + PING_TIMEOUT), Some(10_000));
    }

    #[test]
    fn pings_keep_their_interval_when_the_host_stops_answering() {
        let start = Instant::now();
        let mut latency = Latency::new(start);
        assert!(latency.ping_due(start));
        latency.sent(1, start);
        assert!(!latency.ping_due(start + Duration::from_millis(4_999)));
        assert!(latency.ping_due(start + PING_INTERVAL));
        latency.sent(2, start + PING_INTERVAL);
        assert_eq!(latency.deadline(), start + PING_TIMEOUT);
        assert_eq!(latency.expire(start + PING_TIMEOUT), Some(10_000));
        assert!(latency.ping_due(start + PING_TIMEOUT));
    }

    #[test]
    fn a_pong_matching_no_ping_of_this_route_is_not_a_sample() {
        let start = Instant::now();
        let mut latency = Latency::new(start);
        assert_eq!(latency.pong(7, start + Duration::from_secs(1)), None);
        latency.sent(8, start);
        assert_eq!(latency.pong(7, start + Duration::from_secs(1)), None);
        assert!(latency.samples.is_empty());
        assert_eq!(latency.pings.len(), 1);
    }

    #[test]
    fn clipboard_pieces_reassemble_only_in_order_and_within_the_cap() {
        let piece = |session: &str, text: &str, offset: u64, more: bool| c::ClipboardPush {
            session: session.into(),
            target: "clipboard".into(),
            text: text.into(),
            offset,
            more,
        };
        let mut clipboards = Clipboards::default();
        assert!(clipboards.receive(piece("a", "he", 0, true)).is_none());
        assert!(clipboards.receive(piece("b", "other", 0, true)).is_none());
        let whole = clipboards.receive(piece("a", "llo", 2, false)).unwrap();
        assert_eq!(
            (whole.text.as_str(), whole.offset, whole.more),
            ("hello", 0, false)
        );

        assert!(clipboards
            .receive(piece("b", "skipped", 9, false))
            .is_none());
        assert!(clipboards.receive(piece("b", "late", 5, false)).is_none());

        assert!(clipboards.receive(piece("a", "old", 0, true)).is_none());
        assert_eq!(
            clipboards
                .receive(piece("a", "new", 0, false))
                .unwrap()
                .text,
            "new"
        );

        let mut selection = piece("a", "pri", 0, true);
        selection.target = "selection".into();
        assert!(clipboards.receive(piece("a", "cli", 0, true)).is_none());
        assert!(clipboards.receive(selection.clone()).is_none());
        selection.text = "mary".into();
        selection.offset = 3;
        selection.more = false;
        let primary = clipboards.receive(selection).unwrap();
        assert_eq!(
            (primary.target.as_str(), primary.text.as_str()),
            ("selection", "primary")
        );
        let board = clipboards.receive(piece("a", "pboard", 3, false)).unwrap();
        assert_eq!(
            (board.target.as_str(), board.text.as_str()),
            ("clipboard", "clipboard")
        );

        let cap = runner_terminal::terminal::MAX_CLIPBOARD_BYTES;
        assert!(clipboards
            .receive(piece("a", &"x".repeat(cap), 0, true))
            .is_none());
        assert!(clipboards
            .receive(piece("a", "x", cap as u64, false))
            .is_none());
        assert!(clipboards.0.is_empty());
    }
    #[tokio::test]
    async fn relay_closures_have_fixed_diagnostics_without_the_close_reason() {
        use tokio_tungstenite::{
            tungstenite::protocol::{frame::coding::CloseCode, CloseFrame, Role},
            WebSocketStream,
        };
        for (code, reason) in [
            (4401, "unauthenticated"),
            (4404, "notFound"),
            (4409, "offline"),
            (4426, "channelsUnsupported"),
            (4429, "tooManyChannels"),
        ] {
            let (left, right) = tokio::io::duplex(4096);
            let mut client = WebSocketStream::from_raw_socket(
                Box::new(left) as Box<dyn transport::Connection>,
                Role::Client,
                None,
            )
            .await;
            let mut relay = WebSocketStream::from_raw_socket(
                Box::new(right) as Box<dyn transport::Connection>,
                Role::Server,
                None,
            )
            .await;
            relay
                .send(Message::Close(Some(CloseFrame {
                    code: CloseCode::from(code),
                    reason: "private-body-canary".into(),
                })))
                .await
                .unwrap();
            let error = read(&mut client).await.unwrap_err();
            assert!(error.contains(reason));
            assert!(error.contains(&code.to_string()));
            assert!(!error.contains("private-body-canary"));
        }
    }
}
