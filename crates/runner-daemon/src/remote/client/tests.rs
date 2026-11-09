use super::super::{
    channels::{Channels, TrustWrites},
    machine::{Command as HostCommand, Input, Machine},
    store::{Identity, MemoryStore},
    Clock,
};
use super::*;
use crate::companion::core::Core;
use futures_util::{SinkExt, StreamExt};
use runner_core::{companion::channel, protocol::terminal::*};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Mutex as StdMutex,
};
use tokio_tungstenite::{
    tungstenite::{protocol::Role, Message},
    WebSocketStream,
};

enum RelayCommand {
    Open(Box<transport::Socket>),
    Frame(u32, Option<Vec<u8>>),
    Host(HostCommand, oneshot::Sender<()>),
    Drop(oneshot::Sender<()>),
}
struct Relay {
    tx: mpsc::UnboundedSender<RelayCommand>,
    hosts: StdMutex<Vec<Computer>>,
    listings: AtomicU32,
    opens: AtomicU32,
    writer: Arc<WriteGate>,
}
#[derive(Default)]
struct WriteGate {
    blocked: AtomicBool,
    entered: tokio::sync::Notify,
    waker: futures_util::task::AtomicWaker,
}
struct GatedConnection(tokio::io::DuplexStream, Arc<WriteGate>);
impl tokio::io::AsyncRead for GatedConnection {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_read(cx, buffer)
    }
}
impl tokio::io::AsyncWrite for GatedConnection {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.1.waker.register(cx.waker());
        if self.1.blocked.load(Ordering::Acquire) {
            self.1.entered.notify_one();
            return std::task::Poll::Pending;
        }
        std::pin::Pin::new(&mut self.0).poll_write(cx, bytes)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_shutdown(cx)
    }
}
impl Service for Relay {
    fn list<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<Vec<Computer>>> {
        Box::pin(async {
            self.listings.fetch_add(1, Ordering::Relaxed);
            Ok(self.hosts.lock().unwrap().clone())
        })
    }
    fn open<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
    ) -> BoxFuture<'a, std::result::Result<transport::Socket, transport::Failure>> {
        Box::pin(async move {
            self.opens.fetch_add(1, Ordering::Relaxed);
            let (client, host) = tokio::io::duplex(2 * 1024 * 1024);
            let client = WebSocketStream::from_raw_socket(
                Box::new(GatedConnection(client, self.writer.clone()))
                    as Box<dyn transport::Connection>,
                Role::Client,
                None,
            )
            .await;
            let host = WebSocketStream::from_raw_socket(
                Box::new(host) as Box<dyn transport::Connection>,
                Role::Server,
                None,
            )
            .await;
            self.tx.send(RelayCommand::Open(Box::new(host))).unwrap();
            Ok(client)
        })
    }
}
struct Fixture {
    client: Client,
    relay: Arc<Relay>,
    status: watch::Sender<RemoteStatus>,
    core: Arc<TestCore>,
    root: tempfile::TempDir,
    cancel: CancellationToken,
    store: Arc<dyn SecretStore>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl Fixture {
    async fn new() -> Self {
        Self::with_store(Arc::new(MemoryStore::default())).await
    }
    async fn with_store(store: Arc<dyn SecretStore>) -> Self {
        Self::with_host(store, crate::companion::FEATURES).await
    }
    async fn without_ping() -> Self {
        let features: Vec<_> = crate::companion::FEATURES
            .iter()
            .copied()
            .filter(|feature| *feature != "ping")
            .collect();
        Self::with_host(Arc::new(MemoryStore::default()), &features).await
    }
    async fn with_host(store: Arc<dyn SecretStore>, features: &[&'static str]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut identity = Identity::generate().unwrap();
        identity.pin = Some(Secret("012345".into()));
        let fingerprint = remote::fingerprint(&identity.host_key().unwrap().public);
        let saved = Saved {
            version: 1,
            origin: "https://relay.invalid".into(),
            host_id: "1".into(),
            relay_url: "wss://relay.invalid/relay/host".into(),
            account: Default::default(),
            host_name: "client".into(),
            fingerprint: "local".into(),
            activated: true,
            claim_expires_at: chrono::Utc::now(),
            paused: true,
            stopped: None,
        };
        saved.save(root.path()).unwrap();
        store
            .put(
                &saved.origin,
                &serde_json::to_vec(&Secrets {
                    v: 2,
                    key: None,
                    credential: Secret("credential-canary".into()),
                })
                .unwrap(),
            )
            .unwrap();
        let (status, status_rx) = watch::channel(RemoteStatus {
            state: RemoteState::Paused,
            origin: saved.origin.clone(),
            host_id: Some(saved.host_id.clone()),
            ..Default::default()
        });
        let cancel = CancellationToken::new();
        let events = crate::events::EventChannel::new();
        let client = Client::new(
            root.path().into(),
            store.clone(),
            status_rx,
            events.clone(),
            cancel.clone(),
        );
        let (tx, mut rx) = mpsc::unbounded_channel();
        let relay = Arc::new(Relay {
            tx: tx.clone(),
            hosts: StdMutex::new(vec![Computer {
                id: "2".into(),
                name: "host".into(),
                fingerprint,
                online: true,
                ..Default::default()
            }]),
            listings: AtomicU32::new(0),
            opens: AtomicU32::new(0),
            writer: Default::default(),
        });
        assert!(client.0.service.set(relay.clone()).is_ok());
        let core = Arc::new(TestCore::default());
        {
            let mut overview = core.fake.overview.lock().unwrap();
            let session = overview.sessions.values().next().unwrap().clone();
            overview.sessions.insert("session".into(), session);
        }
        let companion = Arc::new(std::sync::OnceLock::new());
        assert!(companion
            .set(core.clone() as Arc<dyn crate::companion::core::Core>)
            .is_ok());
        let mut machine = Machine::default();
        machine.identity = Some(identity);
        machine.saved = Some(saved);
        machine.status.state = RemoteState::Connected {
            since: chrono::Utc::now(),
        };
        let writes = Arc::new(TrustWrites::default());
        let mut channels = Channels::new(writes.clone(), companion);
        channels.features = features.to_vec();
        let path = root.path().to_path_buf();
        let stop = cancel.clone();
        tokio::spawn(async move {
            let mut sockets = HashMap::new();
            let mut next = 1u32;
            loop {
                let output = tokio::select! {
                    _ = stop.cancelled() => break,
                    command = rx.recv() => {
                        match command.unwrap() {
                            RelayCommand::Open(socket) => {
                                let id = next; next += 1;
                                let (sink, mut stream) = (*socket).split(); sockets.insert(id, sink);
                                match channels.open(id, &machine) { Ok(()) => {}, Err(reason) => { sockets.get_mut(&id).unwrap().send(Message::Binary(reason.frame().into())).await.unwrap(); } }
                                let tx = tx.clone(); tokio::spawn(async move {
                                    while let Some(frame) = stream.next().await {
                                        match frame { Ok(Message::Binary(bytes)) => { let _ = tx.send(RelayCommand::Frame(id, Some(bytes.to_vec()))); }, _ => break }
                                    }
                                    let _ = tx.send(RelayCommand::Frame(id, None));
                                });
                            }
                            RelayCommand::Frame(id, Some(frame)) => { if !channels.feed(id, &frame) { sockets.remove(&id); } }
                            RelayCommand::Frame(id, None) => { channels.closed(id); sockets.remove(&id); }
                            RelayCommand::Host(command, reply) => {
                                let revoke = if let HostCommand::RevokeDevice(f) = &command { Some(f.clone()) } else { None };
                                let (effects, _) = machine.step(Input::Command(command), chrono::Utc::now(), &mut || 0).unwrap();
                                let frames = if effects.iter().any(|e| matches!(e, super::super::machine::Effect::Close)) {
                                    for socket in sockets.values_mut() { let _ = socket.send(Message::Close(None)).await; }
                                    sockets.clear(); for id in 1..next { channels.closed(id); } Vec::new()
                                } else if let Some(f) = revoke { channels.revoke(&f) } else { Vec::new() };
                                for frame in frames { deliver(frame, &mut sockets).await; }
                                let _ = reply.send(());
                            }
                            RelayCommand::Drop(reply) => { for socket in sockets.values_mut() { let _ = socket.send(Message::Close(None)).await; }
                                    sockets.clear(); for id in 1..next { channels.closed(id); } let _ = reply.send(()); }
                        }
                        None
                    }
                    event = channels.next() => channels.handle(event, &mut machine, &path, &Clock::default(), &events),
                    completion = writes.next() => { writes.complete(completion, Some(&mut channels), &mut machine, &path, &events); None }
                };
                if let Some(frame) = output {
                    deliver(frame, &mut sockets).await;
                }
            }
        });
        Self {
            client,
            relay,
            status,
            core,
            root,
            cancel,
            store,
        }
    }
    async fn pair(&self) {
        self.client
            .pair("2".into(), Secret("012345".into()))
            .await
            .unwrap();
    }
    async fn host(&self, command: HostCommand) {
        let (tx, rx) = oneshot::channel();
        self.relay.tx.send(RelayCommand::Host(command, tx)).unwrap();
        rx.await.unwrap();
    }
    async fn drop_route(&self) {
        let (tx, rx) = oneshot::channel();
        self.relay.tx.send(RelayCommand::Drop(tx)).unwrap();
        rx.await.unwrap();
    }
}
async fn deliver(
    frame: Message,
    sockets: &mut HashMap<u32, futures_util::stream::SplitSink<transport::Socket, Message>>,
) {
    match frame {
        Message::Binary(bytes) => {
            let (id, frame) = channel::unwrap(&bytes).unwrap();
            if let Some(socket) = sockets.get_mut(&id) {
                let _ = socket.send(Message::Binary(frame.to_vec().into())).await;
            }
        }
        Message::Text(text) => {
            if let Ok(remote::Control::CloseChannel { channel }) = serde_json::from_str(&text) {
                if let Some(mut socket) = sockets.remove(&channel) {
                    let _ = socket.send(Message::Close(None)).await;
                }
            }
        }
        _ => {}
    }
}
#[derive(Default)]
struct TestCore {
    backend: std::sync::OnceLock<Arc<dyn Core>>,
    fake: crate::companion::tests::Fake,
    streams: StdMutex<Vec<std::sync::mpsc::Sender<TerminalFrame>>>,
    sizes: StdMutex<Vec<(u16, u16, &'static str)>>,
    attaches: AtomicU32,
    stopped: AtomicBool,
    closed: tokio::sync::Notify,
    action_gate: StdMutex<Option<ActionGate>>,
    feed_gate: StdMutex<Option<ActionGate>>,
    fork_calls: AtomicU32,
}
struct ActionGate {
    fork_only: bool,
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    release: std::sync::mpsc::Receiver<()>,
}
struct TestFrames(
    StdMutex<std::sync::mpsc::Receiver<TerminalFrame>>,
    std::sync::mpsc::Sender<TerminalFrame>,
    bool,
);
impl TerminalSubscription for TestFrames {
    fn recv(&mut self) -> std::result::Result<TerminalFrame, runner_core::protocol::ClientError> {
        if self.2 {
            return Err(runner_core::protocol::ClientError::msg("closed"));
        }
        self.0
            .lock()
            .unwrap()
            .recv()
            .map_err(|_| runner_core::protocol::ClientError::msg("closed"))
    }
    fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
        let tx = self.1.clone();
        Arc::new(move || {
            let _ = tx.send(TerminalFrame::Resync);
        })
    }
}
impl TestCore {
    fn terminal(
        self: Arc<Self>,
        target: &str,
        history: usize,
    ) -> crate::companion::core::Result<TerminalAttachment> {
        let fake = Arc::new(crate::companion::tests::Fake::default());
        *fake.bytes.lock().unwrap() = self.fake.bytes.lock().unwrap().clone();
        let mut terminal = fake.attach(target, history, 1)?.terminal;
        let (tx, rx) = std::sync::mpsc::channel();
        self.streams.lock().unwrap().push(tx.clone());
        terminal.frames = Box::new(TestFrames(
            StdMutex::new(rx),
            tx,
            self.stopped.load(Ordering::Relaxed),
        ));
        Ok(terminal)
    }
}
impl crate::companion::core::Core for TestCore {
    fn statuses(
        &self,
    ) -> crate::companion::core::Result<std::collections::BTreeMap<String, c::SessionStatus>> {
        if let Some(core) = self.backend.get() {
            core.statuses()
        } else {
            self.fake.statuses()
        }
    }
    fn action(
        &self,
        request: c::ActionRequest,
    ) -> crate::companion::core::Result<serde_json::Value> {
        let fork = matches!(request, c::ActionRequest::Fork { .. });
        if fork {
            self.fork_calls.fetch_add(1, Ordering::Relaxed);
        }
        let gate = {
            let mut gate = self.action_gate.lock().unwrap();
            if gate.as_ref().is_some_and(|gate| !gate.fork_only || fork) {
                gate.take()
            } else {
                None
            }
        };
        if let Some(gate) = gate {
            gate.entered.send(()).unwrap();
            gate.release.recv().unwrap();
            return Ok(serde_json::json!({"session":"forked"}));
        }
        match self.backend.get() {
            Some(core) => core.action(request),
            None => Ok(serde_json::json!({"ok":true})),
        }
    }
    fn feed(&self, request: c::FeedRequest) -> crate::companion::core::Result<c::FeedPage> {
        let gate = self.feed_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.entered.send(()).unwrap();
            gate.release.recv().unwrap();
        }
        match self.backend.get() {
            Some(core) => core.feed(request),
            None => Ok(c::FeedPage {
                entries: vec![],
                before: None,
            }),
        }
    }
    fn feed_cursor(&self, mission: &str) -> crate::companion::core::Result<u64> {
        match self.backend.get() {
            Some(core) => core.feed_cursor(mission),
            None => Ok(0),
        }
    }
    fn feed_since(
        &self,
        mission: &str,
        cursor: u64,
    ) -> crate::companion::core::Result<crate::companion::feed::Updates> {
        match self.backend.get() {
            Some(core) => core.feed_since(mission, cursor),
            None => Ok(crate::companion::feed::Updates {
                cursor,
                entries: vec![],
                more: false,
            }),
        }
    }
    fn answer(&self, request: c::AnswerRequest) -> crate::companion::core::Result<()> {
        self.backend.get().unwrap().answer(request)
    }
    fn post(&self, request: c::PostRequest) -> crate::companion::core::Result<()> {
        self.backend.get().unwrap().post(request)
    }
    fn overview(&self) -> crate::companion::core::Result<c::Overview> {
        if let Some(core) = self.backend.get() {
            return core.overview();
        }
        self.fake.overview()
    }
    fn subscribe(
        &self,
    ) -> crate::companion::core::Result<tokio::sync::broadcast::Receiver<crate::events::AppEvent>>
    {
        if let Some(core) = self.backend.get() {
            return core.subscribe();
        }
        self.fake.subscribe()
    }
    fn attach(
        self: Arc<Self>,
        target: &str,
        history: usize,
        channel: u64,
    ) -> crate::companion::core::Result<crate::companion::core::Attachment> {
        if let Some(core) = self.backend.get() {
            return core.clone().attach(target, history, channel);
        }
        self.attaches.fetch_add(1, Ordering::Relaxed);
        if self.stopped.load(Ordering::Relaxed) {
            return Err(crate::companion::core::Failure(
                c::ErrorCode::NotFound,
                "Terminal has stopped.",
            ));
        }
        let terminal = self.clone().terminal(target, history)?;
        let target = target.to_owned();
        Ok(crate::companion::core::Attachment {
            terminal,
            resync: Arc::new(move || self.clone().terminal(&target, history)),
        })
    }
    fn resize(
        &self,
        session: &str,
        channel: u64,
        name: &str,
        size: (u16, u16),
        control: crate::companion::core::Control,
    ) -> crate::companion::core::Result<()> {
        if let Some(core) = self.backend.get() {
            return core.resize(session, channel, name, size, control);
        }
        let mode = match control {
            crate::companion::core::Control::Take => "take",
            crate::companion::core::Control::IfFree => "ifFree",
            crate::companion::core::Control::Viewport => "viewport",
        };
        self.sizes.lock().unwrap().push((size.0, size.1, mode));
        Ok(())
    }
    fn input(
        &self,
        target: &str,
        channel: u64,
        bytes: &[u8],
    ) -> crate::companion::core::Result<()> {
        if let Some(core) = self.backend.get() {
            return core.input(target, channel, bytes);
        }
        self.fake.input(target, channel, bytes)
    }
    fn detach(&self, session: &str, subscriber: u64, channel: u64, release: bool) {
        if let Some(core) = self.backend.get() {
            core.detach(session, subscriber, channel, release);
        }
    }
    fn closed_reason(&self, _: &str) -> c::ClosedReason {
        self.closed.notify_one();
        c::ClosedReason::Exited
    }
    fn release_channel(&self, channel: u64) {
        if let Some(core) = self.backend.get() {
            core.release_channel(channel);
        }
    }
}

#[tokio::test]
async fn lists_pairs_uses_overview_while_paused_and_keeps_keys_on_signout() {
    let fixture = Fixture::new().await;
    assert!(!fixture.client.computers().await.unwrap()[0].paired);
    fixture.pair().await;
    assert!(fixture.client.computers().await.unwrap()[0].paired);
    assert_eq!(fixture.relay.listings.load(Ordering::Relaxed), 1);
    assert!(!fixture
        .client
        .overview("2".into())
        .await
        .unwrap()
        .sessions
        .is_empty());
    fixture.status.send_modify(|s| s.state = RemoteState::Off);
    assert!(fixture.client.computers().await.is_err());
    assert_eq!(ClientKeys::load(fixture.store.as_ref()).unwrap().0.len(), 1);
    assert!(
        !format!("{:?}", ClientKeys::load(fixture.store.as_ref()).unwrap())
            .contains("credential-canary")
    );
}

#[tokio::test]
async fn wrong_pin_is_one_attempt_and_forget_removes_only_the_target() {
    let fixture = Fixture::new().await;
    let error = fixture
        .client
        .pair("2".into(), Secret("999999".into()))
        .await
        .unwrap_err();
    assert_eq!(error, "Remote channel: wrongPin.");
    assert_eq!(fixture.relay.opens.load(Ordering::Relaxed), 1);
    assert!(fixture.client.keys().await.unwrap().0.is_empty());
    fixture.pair().await;
    fixture.client.forget("2".into()).await.unwrap();
    assert!(fixture.client.keys().await.unwrap().0.is_empty());
    assert!(fixture
        .store
        .get("https://relay.invalid")
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn terminal_attachment_maps_snapshot_output_resize_input_and_control() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let config = RemoteTerminal {
        computer: "2".into(),
        session: "session".into(),
        cols: 100,
        rows: 40,
        take_control: true,
    };
    let mut attachment = fixture.client.attach(config).await.unwrap();
    assert_eq!(attachment.snapshot.bytes, b"screen");
    assert!(
        matches!(attachment.snapshot.controller, TerminalController::Browser { ref name, .. } if name == "Host desktop")
    );
    let tx = fixture.core.streams.lock().unwrap()[0].clone();
    tx.send(TerminalFrame::Output {
        seq: 1,
        bytes: b"output-canary".to_vec(),
    })
    .unwrap();
    let mut frames = tokio::task::spawn_blocking(move || { assert!(matches!(attachment.frames.recv().unwrap(), TerminalFrame::Output { seq: 11, ref bytes } if bytes == b"output-canary")); attachment }).await.unwrap();
    tx.send(TerminalFrame::Resized {
        seq: 2,
        cols: 120,
        rows: 50,
        controller: TerminalController::None,
    })
    .unwrap();
    let mut frames = tokio::task::spawn_blocking(move || {
        assert!(matches!(
            frames.frames.recv().unwrap(),
            TerminalFrame::Resized {
                seq: 12,
                cols: 120,
                rows: 50,
                ..
            }
        ));
        frames
    })
    .await
    .unwrap();
    tx.send(TerminalFrame::Output {
        seq: 3,
        bytes: b"after-resize".to_vec(),
    })
    .unwrap();
    let frames = tokio::task::spawn_blocking(move || {
        assert!(matches!(
            frames.frames.recv().unwrap(),
            TerminalFrame::Output { seq: 13, .. }
        ));
        frames
    })
    .await
    .unwrap();
    fixture
        .client
        .input("2".into(), "session".into(), b"input-canary".to_vec())
        .await
        .unwrap();
    fixture
        .client
        .viewport("2".into(), "session".into(), 110, 45, true)
        .await
        .unwrap();
    fixture
        .client
        .viewport("2".into(), "session".into(), 90, 30, false)
        .await
        .unwrap();
    assert_eq!(
        fixture.core.fake.input.lock().unwrap()[0].1,
        b"input-canary"
    );
    assert_eq!(
        *fixture.core.sizes.lock().unwrap(),
        [(100, 40, "take"), (110, 45, "take"), (90, 30, "viewport")]
    );
    drop(frames);
}

#[tokio::test]
async fn only_explicit_reset_can_discard_corrupt_pairing_metadata() {
    for paired in [true, false] {
        let fixture = Fixture::new().await;
        Identity::generate()
            .unwrap()
            .save(fixture.store.as_ref())
            .unwrap();
        let identity = fixture
            .store
            .get(super::super::store::IDENTITY_NAME)
            .unwrap();
        let enrollment = fixture.store.get("https://relay.invalid").unwrap();
        if paired {
            fixture.pair().await;
        }
        let primary = fixture
            .store
            .get(super::super::store::CLIENT_KEYS_NAME)
            .unwrap();
        fixture.store.put("client-keys-cleanup", b"{").unwrap();
        assert!(fixture.client.forget("2".into()).await.is_err());
        assert!(
            fixture
                .store
                .get(super::super::store::CLIENT_KEYS_NAME)
                .unwrap()
                == primary
        );
        assert_eq!(
            fixture.store.get("client-keys-cleanup").unwrap().unwrap(),
            b"{"
        );
        assert!(ClientKeys::default().save(fixture.store.as_ref()).is_err());
        fixture.client.reset().await.unwrap();
        fixture.pair().await;
        fixture.client.overview("2".into()).await.unwrap();
        fixture.client.reset().await.unwrap();
        let corrupt = b"RUNNER-CLIENT-KEYS:{";
        fixture
            .store
            .put(super::super::store::CLIENT_KEYS_NAME, corrupt)
            .unwrap();
        assert!(ClientKeys::default().save(fixture.store.as_ref()).is_err());
        assert!(fixture.client.forget("2".into()).await.is_err());
        assert_eq!(
            fixture
                .store
                .get(super::super::store::CLIENT_KEYS_NAME)
                .unwrap()
                .unwrap(),
            corrupt
        );
        fixture.client.reset().await.unwrap();
        fixture.pair().await;
        fixture.client.overview("2".into()).await.unwrap();
        fixture.client.reset().await.unwrap();
        assert!(
            fixture
                .store
                .get(super::super::store::IDENTITY_NAME)
                .unwrap()
                == identity
        );
        assert!(fixture.store.get("https://relay.invalid").unwrap() == enrollment);
        assert!(fixture
            .store
            .get(super::super::store::CLIENT_KEYS_NAME)
            .unwrap()
            .is_none());
        assert!(fixture.store.get("client-keys-cleanup").unwrap().is_none());
    }
}

#[tokio::test]
async fn committed_pair_and_forget_succeed_with_deferred_cleanup_and_reset_recovers_corruption() {
    #[derive(Default)]
    struct DeferredStore {
        memory: MemoryStore,
        names: StdMutex<std::collections::HashSet<String>>,
        fail_delete: AtomicBool,
    }
    impl SecretStore for DeferredStore {
        fn get(&self, name: &str) -> super::super::store::StoreResult<Option<Vec<u8>>> {
            self.memory.get(name)
        }
        fn put(&self, name: &str, bytes: &[u8]) -> super::super::store::StoreResult<()> {
            assert!(bytes.len() <= 2560);
            self.memory.put(name, bytes)?;
            self.names.lock().unwrap().insert(name.into());
            Ok(())
        }
        fn delete(&self, name: &str) -> super::super::store::StoreResult<()> {
            if name.starts_with("client-keys/") && self.fail_delete.load(Ordering::Relaxed) {
                return Err("test cleanup failure");
            }
            self.memory.delete(name)?;
            self.names.lock().unwrap().remove(name);
            Ok(())
        }
    }
    let store = Arc::new(DeferredStore::default());
    let fixture = Fixture::with_store(store.clone()).await;
    Identity::generate().unwrap().save(store.as_ref()).unwrap();
    let identity = store.get(super::super::store::IDENTITY_NAME).unwrap();
    let origin = "https://relay.invalid";
    let enrollment = store.get(origin).unwrap();
    let mut keys = ClientKeys::default();
    for index in 0..20 {
        let key = remote::HostKey::generate().unwrap();
        keys.0.insert(
            remote::fingerprint(&key.public),
            ClientKey {
                private: Secret(remote::encode_key(&key.private)),
                public: key.public.to_vec(),
                name: format!("host-{index}"),
                paired_at: chrono::Utc::now(),
            },
        );
    }
    keys.save(store.as_ref()).unwrap();
    store.fail_delete.store(true, Ordering::Relaxed);
    fixture.pair().await;
    assert_eq!(ClientKeys::load(store.as_ref()).unwrap().0.len(), 21);
    assert!(store.get("client-keys-cleanup").unwrap().is_some());
    assert!(!fixture.client.0.routes.lock().await.is_empty());
    fixture.client.overview("2".into()).await.unwrap();
    store.fail_delete.store(false, Ordering::Relaxed);
    ClientKeys::load(store.as_ref())
        .unwrap()
        .save(store.as_ref())
        .unwrap();
    assert!(store.get("client-keys-cleanup").unwrap().is_none());
    store.fail_delete.store(true, Ordering::Relaxed);
    fixture.client.forget("2".into()).await.unwrap();
    assert_eq!(ClientKeys::load(store.as_ref()).unwrap().0.len(), 20);
    assert!(!fixture.client.computers().await.unwrap()[0].paired);
    assert!(store.get("client-keys-cleanup").unwrap().is_some());
    assert!(fixture.client.reset().await.is_err());
    store.fail_delete.store(false, Ordering::Relaxed);
    fixture.client.reset().await.unwrap();
    assert_eq!(
        *store.names.lock().unwrap(),
        [super::super::store::IDENTITY_NAME.into(), origin.into()]
            .into_iter()
            .collect()
    );
    store.put("client-keys-cleanup", b"{").unwrap();
    assert!(keys.save(store.as_ref()).is_err());
    fixture.client.reset().await.unwrap();
    assert!(store.get("client-keys-cleanup").unwrap().is_none());
    fixture.pair().await;
    fixture.client.overview("2".into()).await.unwrap();
    fixture.client.reset().await.unwrap();
    assert_eq!(
        *store.names.lock().unwrap(),
        [super::super::store::IDENTITY_NAME.into(), origin.into()]
            .into_iter()
            .collect()
    );
    assert!(store.get(super::super::store::IDENTITY_NAME).unwrap() == identity);
    assert!(store.get(origin).unwrap() == enrollment);
}

#[tokio::test]
async fn interrupted_key_generation_writes_and_cleanup_recover_before_retry_or_reset() {
    #[derive(Default)]
    struct InterruptedStore {
        memory: MemoryStore,
        names: StdMutex<std::collections::HashSet<String>>,
        remaining: StdMutex<Option<usize>>,
        interrupted: AtomicBool,
    }
    impl InterruptedStore {
        fn mutation(&self) -> super::super::store::StoreResult<()> {
            if let Some(remaining) = self.remaining.lock().unwrap().as_mut() {
                if *remaining == 0 {
                    self.interrupted.store(true, Ordering::Relaxed);
                    return Err("test interruption");
                }
                *remaining -= 1;
            }
            Ok(())
        }
    }
    impl SecretStore for InterruptedStore {
        fn get(&self, name: &str) -> super::super::store::StoreResult<Option<Vec<u8>>> {
            self.memory.get(name)
        }
        fn put(&self, name: &str, bytes: &[u8]) -> super::super::store::StoreResult<()> {
            assert!(bytes.len() <= 2560);
            self.mutation()?;
            self.memory.put(name, bytes)?;
            self.names.lock().unwrap().insert(name.into());
            Ok(())
        }
        fn delete(&self, name: &str) -> super::super::store::StoreResult<()> {
            self.mutation()?;
            self.memory.delete(name)?;
            self.names.lock().unwrap().remove(name);
            Ok(())
        }
    }
    let mut many = ClientKeys::default();
    for index in 0..20 {
        let key = remote::HostKey::generate().unwrap();
        many.0.insert(
            remote::fingerprint(&key.public),
            ClientKey {
                private: Secret(remote::encode_key(&key.private)),
                public: key.public.to_vec(),
                name: format!("host-{index}"),
                paired_at: chrono::Utc::now(),
            },
        );
    }
    let mut fewer = many.clone();
    fewer.0.pop_first();
    let one = ClientKeys(
        many.0
            .iter()
            .take(1)
            .map(|(id, key)| (id.clone(), key.clone()))
            .collect(),
    );
    for (old, new) in [
        (ClientKeys::default(), many.clone()),
        (many.clone(), fewer),
        (many, one),
    ] {
        for retry in [false, true] {
            let mut completed = false;
            for completed_mutations in 0..100 {
                let root = tempfile::tempdir().unwrap();
                let store = Arc::new(InterruptedStore::default());
                Identity::generate().unwrap().save(store.as_ref()).unwrap();
                let identity = store.get(super::super::store::IDENTITY_NAME).unwrap();
                let origin = "https://cleanup-test.invalid";
                store.put(origin, b"isolated-enrollment").unwrap();
                old.save(store.as_ref()).unwrap();
                let client = Client::new(
                    root.path().into(),
                    store.clone(),
                    watch::channel(RemoteStatus::default()).1,
                    Default::default(),
                    CancellationToken::new(),
                );
                *store.remaining.lock().unwrap() = Some(completed_mutations);
                let saved = new.save(store.as_ref());
                let count = ClientKeys::load(store.as_ref()).unwrap().0.len();
                assert!(count == old.0.len() || count == new.0.len());
                if store.interrupted.load(Ordering::Relaxed) {
                    assert_eq!(
                        count,
                        if saved.is_ok() {
                            new.0.len()
                        } else {
                            old.0.len()
                        }
                    );
                    let names = store.names.lock().unwrap().clone();
                    assert!(new.save(store.as_ref()).is_err());
                    assert!(client.reset().await.is_err());
                    assert_eq!(*store.names.lock().unwrap(), names);
                } else {
                    saved.unwrap();
                    completed = true;
                }
                *store.remaining.lock().unwrap() = None;
                if retry {
                    new.save(store.as_ref()).unwrap();
                    assert_eq!(
                        ClientKeys::load(store.as_ref()).unwrap().0.len(),
                        new.0.len()
                    );
                    assert!(store.get("client-keys-cleanup").unwrap().is_none());
                }
                client.reset().await.unwrap();
                assert!(ClientKeys::load(store.as_ref()).unwrap().0.is_empty());
                assert_eq!(
                    *store.names.lock().unwrap(),
                    [super::super::store::IDENTITY_NAME.into(), origin.into()]
                        .into_iter()
                        .collect()
                );
                assert!(store.get(super::super::store::IDENTITY_NAME).unwrap() == identity);
                assert_eq!(store.get(origin).unwrap().unwrap(), b"isolated-enrollment");
                if completed {
                    break;
                }
            }
            assert!(completed);
        }
    }
}

#[tokio::test]
async fn reset_deletes_every_private_key_shard_and_keeps_identity_and_enrollment() {
    #[derive(Default)]
    struct AuditedStore {
        memory: MemoryStore,
        names: StdMutex<std::collections::HashSet<String>>,
        fail_delete: AtomicBool,
    }
    impl SecretStore for AuditedStore {
        fn get(&self, name: &str) -> super::super::store::StoreResult<Option<Vec<u8>>> {
            self.memory.get(name)
        }
        fn put(&self, name: &str, bytes: &[u8]) -> super::super::store::StoreResult<()> {
            self.memory.put(name, bytes)?;
            self.names.lock().unwrap().insert(name.into());
            Ok(())
        }
        fn delete(&self, name: &str) -> super::super::store::StoreResult<()> {
            if name.starts_with("client-keys/") && self.fail_delete.load(Ordering::Relaxed) {
                return Err("test deletion failure");
            }
            self.memory.delete(name)?;
            self.names.lock().unwrap().remove(name);
            Ok(())
        }
    }
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(AuditedStore::default());
    Identity::generate().unwrap().save(store.as_ref()).unwrap();
    let identity = store.get(super::super::store::IDENTITY_NAME).unwrap();
    let origin = "https://reset-test.invalid";
    store.put(origin, b"isolated-enrollment").unwrap();
    let mut keys = ClientKeys::default();
    for index in 0..20 {
        let key = remote::HostKey::generate().unwrap();
        keys.0.insert(
            remote::fingerprint(&key.public),
            ClientKey {
                private: Secret(remote::encode_key(&key.private)),
                public: key.public.to_vec(),
                name: format!("host-{index}"),
                paired_at: chrono::Utc::now(),
            },
        );
    }
    keys.save(store.as_ref()).unwrap();
    assert!(store.names.lock().unwrap().len() > 3);
    let client = Client::new(
        root.path().into(),
        store.clone(),
        watch::channel(RemoteStatus::default()).1,
        Default::default(),
        CancellationToken::new(),
    );
    store.fail_delete.store(true, Ordering::Relaxed);
    assert!(client.reset().await.is_err());
    assert!(store
        .get(super::super::store::CLIENT_KEYS_NAME)
        .unwrap()
        .is_some());
    store.fail_delete.store(false, Ordering::Relaxed);
    client.reset().await.unwrap();
    assert!(ClientKeys::load(store.as_ref()).unwrap().0.is_empty());
    assert_eq!(
        *store.names.lock().unwrap(),
        [super::super::store::IDENTITY_NAME.into(), origin.into()]
            .into_iter()
            .collect()
    );
    assert_eq!(
        store.get(super::super::store::IDENTITY_NAME).unwrap(),
        identity
    );
    assert_eq!(store.get(origin).unwrap().unwrap(), b"isolated-enrollment");
    store
        .put(
            super::super::store::CLIENT_KEYS_NAME,
            b"RUNNER-CLIENT-KEYS:{",
        )
        .unwrap();
    assert!(ClientKeys::load(store.as_ref()).is_err());
    client.reset().await.unwrap();
    assert!(store
        .get(super::super::store::CLIENT_KEYS_NAME)
        .unwrap()
        .is_none());
    assert_eq!(
        store.get(super::super::store::IDENTITY_NAME).unwrap(),
        identity
    );
    assert_eq!(store.get(origin).unwrap().unwrap(), b"isolated-enrollment");
}

#[tokio::test]
async fn changed_listing_is_not_paired_and_reset_deletes_client_keys() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture.relay.hosts.lock().unwrap()[0].fingerprint = "changed".into();
    *fixture.client.0.directory.lock().await = None;
    assert!(!fixture.client.computers().await.unwrap()[0].paired);
    assert!(fixture
        .client
        .overview("2".into())
        .await
        .unwrap_err()
        .contains("Pair"));
    fixture.client.reset().await.unwrap();
    assert!(fixture
        .store
        .get(super::super::store::CLIENT_KEYS_NAME)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn follows_overview_and_status_pushes_and_releases_connection_owner() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let mut events = fixture.client.0.events.subscribe();
    fixture.client.follow("2".into(), 77, true).await.unwrap();
    loop {
        if events.recv().await.unwrap().name == "remote/overview" {
            break;
        }
    }
    fixture.core.fake.overview.lock().unwrap().tabs[0].name = "changed".into();
    fixture.core.fake.events.emit("chat/changed", &());
    loop {
        let event = events.recv().await.unwrap();
        if event.name == "remote/overview" {
            assert!(event.payload.to_string().contains("changed"));
            break;
        }
    }
    fixture
        .core
        .fake
        .overview
        .lock()
        .unwrap()
        .sessions
        .values_mut()
        .next()
        .unwrap()
        .status
        .unread = true;
    fixture.core.fake.events.emit("session/status", &());
    loop {
        let event = events.recv().await.unwrap();
        if event.name == "remote/session-status" {
            assert_eq!(event.payload["status"]["unread"], true);
            break;
        }
    }
    fixture.client.owner_gone(77).await;
}

#[tokio::test]
async fn reconnects_with_saved_key_and_keeps_it_after_host_revoke_until_explicit_forget() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let key = fixture
        .client
        .keys()
        .await
        .unwrap()
        .0
        .values()
        .next()
        .unwrap()
        .clone();
    let public = remote::HostKey::from_private(&key.private.0)
        .unwrap()
        .public;
    let mut events = fixture.client.0.events.subscribe();
    fixture.drop_route().await;
    loop {
        if events.recv().await.unwrap().name == "remote/connection" {
            break;
        }
    }
    // A completed disconnect leaves no subscriber to trigger an automatic reconnect.
    loop {
        let routes = fixture.client.0.routes.lock().await;
        if routes.values().all(|r| r.cancel.is_cancelled()) {
            break;
        }
        drop(routes);
        tokio::task::yield_now().await;
    }
    assert!(fixture.client.overview("2".into()).await.is_ok());
    assert_eq!(fixture.relay.opens.load(Ordering::Relaxed), 2);
    fixture
        .host(HostCommand::RevokeDevice(remote::fingerprint(&public)))
        .await;
    loop {
        let routes = fixture.client.0.routes.lock().await;
        if routes.values().all(|r| r.cancel.is_cancelled()) {
            break;
        }
        drop(routes);
        tokio::task::yield_now().await;
    }
    assert!(fixture.client.overview("2".into()).await.is_err());
    assert_eq!(fixture.client.keys().await.unwrap().0.len(), 1);
    fixture.client.forget("2".into()).await.unwrap();
    assert!(fixture.client.keys().await.unwrap().0.is_empty());
}

#[tokio::test]
async fn no_pin_locked_pin_and_bad_handshake_are_clear_and_never_retried() {
    let fixture = Fixture::new().await;
    for _ in 0..5 {
        fixture.host(HostCommand::RecordFailedPin).await;
    }
    assert_eq!(
        fixture
            .client
            .pair("2".into(), Secret("012345".into()))
            .await
            .unwrap_err(),
        "Remote channel: pinLocked."
    );
    fixture.host(HostCommand::UnlockPin).await;
    let mut saved = Saved::load(fixture.root.path()).unwrap().unwrap();
    saved.host_name = String::new();
    saved.save(fixture.root.path()).unwrap();
    assert_eq!(
        fixture
            .client
            .pair("2".into(), Secret("012345".into()))
            .await
            .unwrap_err(),
        "Remote channel: badHandshake."
    );
    fixture
        .host(HostCommand::ResetIdentity(Identity::generate().unwrap()))
        .await;
    assert_eq!(
        fixture
            .client
            .pair("2".into(), Secret("012345".into()))
            .await
            .unwrap_err(),
        "Remote channel: noPin."
    );
    assert_eq!(fixture.relay.opens.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn a_client_without_credit_does_not_stall_a_second_client() {
    let fixture = Fixture::new().await;
    let context = fixture.client.context().await.unwrap();
    let socket = fixture.relay.open(&context, "2").await.unwrap();
    let key = remote::HostKey::generate().unwrap();
    let (mut socket, mut records, _, _) =
        route::pair(socket, &key, &Secret("012345".into()), "stalled")
            .await
            .unwrap();
    *fixture.core.fake.bytes.lock().unwrap() = vec![b'x'; 2 * 1024 * 1024];
    records
        .queue(
            serde_json::to_vec(&c::ClientMessage::Attach(c::AttachRequest {
                id: 1,
                session: "session".into(),
                cols: 80,
                rows: 24,
                control: c::ControlMode::IfFree,
                history: None,
            }))
            .unwrap(),
        )
        .unwrap();
    while let Some(frame) = records.next_frame().unwrap() {
        socket.send(Message::Binary(frame.into())).await.unwrap();
    }
    fixture.pair().await;
    assert!(fixture.client.overview("2".into()).await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn unused_channel_closes_after_a_minute() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture.client.overview("2".into()).await.unwrap();
    let route = fixture.client.route("2").await.unwrap();
    tokio::time::advance(Duration::from_secs(61)).await;
    route.cancel.cancelled().await;
    assert_eq!(fixture.relay.opens.load(Ordering::Relaxed), 1);
}

#[tokio::test(start_paused = true)]
async fn dropped_terminal_reconnects_with_snapshot_and_does_not_replay_input() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let config = RemoteTerminal {
        computer: "2".into(),
        session: "session".into(),
        cols: 80,
        rows: 24,
        take_control: true,
    };
    let mut attachment = fixture.client.attach(config.clone()).await.unwrap();
    fixture
        .client
        .input("2".into(), "session".into(), b"once".to_vec())
        .await
        .unwrap();
    fixture
        .client
        .viewport("2".into(), "session".into(), 80, 24, false)
        .await
        .unwrap();
    let mut events = fixture.client.0.events.subscribe();
    fixture.drop_route().await;
    loop {
        let event = events.recv().await.unwrap();
        if event.name == "remote/connection" && event.payload["state"] == "reconnecting" {
            break;
        }
    }
    assert!(fixture
        .client
        .input("2".into(), "session".into(), b"lost".to_vec())
        .await
        .is_err());
    *fixture.core.fake.bytes.lock().unwrap() = b"fresh screen".to_vec();
    tokio::time::advance(Duration::from_secs(2)).await;
    loop {
        let event = events.recv().await.unwrap();
        if event.name == "remote/connection" && event.payload["state"] == "connected" {
            break;
        }
    }
    let attachment = tokio::task::spawn_blocking(move || {
        assert!(matches!(
            attachment.frames.recv().unwrap(),
            TerminalFrame::Resync
        ));
        attachment
    })
    .await
    .unwrap();
    let fresh = fixture.client.attach(config).await.unwrap();
    assert_eq!(fresh.snapshot.bytes, b"fresh screen");
    assert_eq!(fixture.core.fake.input.lock().unwrap().len(), 1);
    drop((attachment, fresh));
}

#[tokio::test]
async fn host_channel_limit_surfaces_busy_without_retrying_pairing() {
    let fixture = Fixture::new().await;
    let context = fixture.client.context().await.unwrap();
    let mut sockets = Vec::new();
    for _ in 0..16 {
        sockets.push(fixture.relay.open(&context, "2").await.unwrap());
    }
    assert_eq!(
        fixture
            .client
            .pair("2".into(), Secret("012345".into()))
            .await
            .unwrap_err(),
        "Remote channel: busy."
    );
    assert_eq!(fixture.relay.opens.load(Ordering::Relaxed), 17);
}

struct DirectoryError(AtomicU32, &'static str);
impl Service for DirectoryError {
    fn list<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<Vec<Computer>>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(format!("Could not list computers: {}.", self.1))
        })
    }
    fn open<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
    ) -> BoxFuture<'a, std::result::Result<transport::Socket, transport::Failure>> {
        panic!("a failed directory must not open a route")
    }
}
#[tokio::test(start_paused = true)]
async fn directory_errors_are_clear_and_rate_limits_are_cached() {
    let fixture = Fixture::new().await;
    for code in ["unauthenticated", "resource_exhausted"] {
        let client = Client::new(
            fixture.root.path().into(),
            fixture.store.clone(),
            fixture.status.subscribe(),
            Default::default(),
            fixture.cancel.child_token(),
        );
        let service = Arc::new(DirectoryError(AtomicU32::new(0), code));
        assert!(client.0.service.set(service.clone()).is_ok());
        assert!(client.computers().await.unwrap_err().contains(code));
        assert!(client.computers().await.unwrap_err().contains(code));
        assert_eq!(service.0.load(Ordering::Relaxed), 1);
        tokio::time::advance(Duration::from_secs(61)).await;
        assert!(client.computers().await.unwrap_err().contains(code));
        assert_eq!(service.0.load(Ordering::Relaxed), 2);
    }
}

#[tokio::test]
async fn stored_pin_mismatch_fails_before_connect_and_debug_redacts_private_keys() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let mut keys = fixture.client.keys().await.unwrap();
    let key = keys.0.values_mut().next().unwrap();
    let private = key.private.0.clone();
    key.public = vec![0; 32];
    assert!(!format!("{keys:?}").contains(&private));
    keys.save(fixture.store.as_ref()).unwrap();
    assert!(fixture
        .client
        .computers()
        .await
        .unwrap_err()
        .contains("pinned"));
    assert_eq!(fixture.relay.opens.load(Ordering::Relaxed), 1);
}

struct RefuseRoute(Vec<Computer>, u16);

struct ForgedReject(Vec<Computer>, channel::Reject);
impl Service for ForgedReject {
    fn list<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<Vec<Computer>>> {
        Box::pin(async { Ok(self.0.clone()) })
    }
    fn open<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
    ) -> BoxFuture<'a, std::result::Result<transport::Socket, transport::Failure>> {
        let reject = self.1.frame();
        Box::pin(async move {
            let (client, host) = tokio::io::duplex(4096);
            let client = WebSocketStream::from_raw_socket(
                Box::new(client) as Box<dyn transport::Connection>,
                Role::Client,
                None,
            )
            .await;
            let mut host = WebSocketStream::from_raw_socket(
                Box::new(host) as Box<dyn transport::Connection>,
                Role::Server,
                None,
            )
            .await;
            tokio::spawn(async move {
                let _ = host.next().await;
                let _ = host.send(Message::Binary(reject.into())).await;
            });
            Ok(client)
        })
    }
}

#[tokio::test]
async fn unauthenticated_rejects_preserve_saved_pairing_keys_until_explicit_forget() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let before = serde_json::to_value(fixture.client.keys().await.unwrap()).unwrap();
    for reason in [
        channel::Reject::UnknownDevice,
        channel::Reject::BadHandshake,
        channel::Reject::Busy,
        channel::Reject::NoPin,
        channel::Reject::PinLocked,
        channel::Reject::WrongPin,
    ] {
        let client = Client::new(
            fixture.root.path().into(),
            fixture.store.clone(),
            fixture.status.subscribe(),
            Default::default(),
            fixture.cancel.child_token(),
        );
        let hosts = fixture.relay.hosts.lock().unwrap().clone();
        assert!(client
            .0
            .service
            .set(Arc::new(ForgedReject(hosts, reason)))
            .is_ok());
        let result = client.overview("2".into()).await;
        assert!(result.is_err());
        assert!(result.err().unwrap().contains(reason.reason()));
        assert!(serde_json::to_value(client.keys().await.unwrap()).unwrap() == before);
        assert!(client.computers().await.unwrap()[0].paired);
    }
    fixture.client.forget("2".into()).await.unwrap();
    assert!(fixture.client.keys().await.unwrap().0.is_empty());
}

impl Service for RefuseRoute {
    fn list<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<Vec<Computer>>> {
        Box::pin(async { Ok(self.0.clone()) })
    }
    fn open<'a>(
        &'a self,
        _: &'a Context,
        _: &'a str,
    ) -> BoxFuture<'a, std::result::Result<transport::Socket, transport::Failure>> {
        Box::pin(async move {
            Err(transport::Failure {
                code: "unavailable".into(),
                message: format!("Remote route refused (HTTP {}).", self.1),
                retry: remote::Retry::Backoff,
            })
        })
    }
}
#[tokio::test]
async fn initial_route_refusal_reaches_the_request_and_old_generation_input_is_discarded() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let route = fixture.client.route("2").await.unwrap();
    let generation = route.generation().wrapping_sub(1);
    assert!(route
        .input(generation, "session".into(), b"stale".to_vec())
        .await
        .unwrap_err()
        .contains("discarded"));
    assert!(fixture.core.fake.input.lock().unwrap().is_empty());
    for status in [401, 404, 429, 503] {
        let client = Client::new(
            fixture.root.path().into(),
            fixture.store.clone(),
            fixture.status.subscribe(),
            Default::default(),
            fixture.cancel.child_token(),
        );
        let hosts = fixture.relay.hosts.lock().unwrap().clone();
        assert!(client
            .0
            .service
            .set(Arc::new(RefuseRoute(hosts, status)))
            .is_ok());
        assert!(client
            .overview("2".into())
            .await
            .unwrap_err()
            .contains(&status.to_string()));
    }
}

#[tokio::test]
async fn host_identity_reset_leaves_the_new_listing_unpaired() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let identity = Identity::generate().unwrap();
    let fingerprint = remote::fingerprint(&identity.host_key().unwrap().public);
    fixture.host(HostCommand::ResetIdentity(identity)).await;
    fixture.relay.hosts.lock().unwrap()[0].fingerprint = fingerprint;
    *fixture.client.0.directory.lock().await = None;
    assert!(!fixture.client.computers().await.unwrap()[0].paired);
    assert!(fixture
        .client
        .overview("2".into())
        .await
        .unwrap_err()
        .contains("Pair"));
    assert_eq!(fixture.relay.opens.load(Ordering::Relaxed), 1);
}

async fn receive_terminal(
    mut attachment: TerminalAttachment,
) -> (TerminalAttachment, TerminalFrame) {
    tokio::task::spawn_blocking(move || {
        let frame = attachment.frames.recv().unwrap();
        (attachment, frame)
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn host_clipboard_writes_reach_the_viewer_whole_under_the_remote_address() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let attachment = fixture.client.attach(terminal_config()).await.unwrap();
    let mut events = fixture.client.0.events.subscribe();
    let cap = runner_terminal::terminal::MAX_CLIPBOARD_BYTES;
    let escaped = "\n\\\"\u{1}é"
        .repeat(cap)
        .chars()
        .take(cap / 2)
        .collect::<String>();
    let escaped = format!("{escaped}{}", "\n".repeat(cap - escaped.len()));
    for text in [escaped, "copied".into()] {
        fixture.core.fake.events.emit(
            "session/clipboard",
            &serde_json::json!({"session_id":"session","target":"selection","text":text}),
        );
        let event = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let event = events.recv().await.unwrap();
                if event.name == "session/clipboard" {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            event.payload["session_id"],
            remote::terminal_address("2", "session")
        );
        assert_eq!(event.payload["target"], "selection");
        assert!(event.payload["text"] == text.as_str());
    }
    drop(attachment);
}
fn terminal_config() -> RemoteTerminal {
    RemoteTerminal {
        computer: "2".into(),
        session: "session".into(),
        cols: 80,
        rows: 24,
        take_control: true,
    }
}
#[tokio::test]
async fn a_joining_subscriber_replays_snapshot_output_and_resize_without_invalidating_others() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let first = fixture.client.attach(terminal_config()).await.unwrap();
    let tx = fixture.core.streams.lock().unwrap()[0].clone();
    tx.send(TerminalFrame::Output {
        seq: 1,
        bytes: b"before join".to_vec(),
    })
    .unwrap();
    let (first, frame) = receive_terminal(first).await;
    assert!(matches!(frame, TerminalFrame::Output { seq: 11, .. }));
    tx.send(TerminalFrame::Resized {
        seq: 2,
        cols: 120,
        rows: 50,
        controller: TerminalController::None,
    })
    .unwrap();
    let (first, frame) = receive_terminal(first).await;
    assert!(matches!(frame, TerminalFrame::Resized { seq: 12, .. }));
    let second = fixture.client.attach(terminal_config()).await.unwrap();
    assert_eq!(second.snapshot.seq, 10);
    assert_eq!((second.snapshot.cols, second.snapshot.rows), (80, 24));
    let (second, frame) = receive_terminal(second).await;
    assert!(
        matches!(frame, TerminalFrame::Output { seq: 11, ref bytes } if bytes == b"before join")
    );
    let (second, frame) = receive_terminal(second).await;
    assert!(matches!(
        frame,
        TerminalFrame::Resized {
            seq: 12,
            cols: 120,
            rows: 50,
            ..
        }
    ));
    assert_eq!(fixture.core.streams.lock().unwrap().len(), 1);
    tx.send(TerminalFrame::Output {
        seq: 3,
        bytes: b"both".to_vec(),
    })
    .unwrap();
    for attachment in [first, second] {
        let (_, frame) = receive_terminal(attachment).await;
        assert!(matches!(frame, TerminalFrame::Output { seq: 13, ref bytes } if bytes == b"both"));
    }
}
#[tokio::test(start_paused = true)]
async fn two_subscribers_recover_repeatedly_and_settle_without_a_resync_loop() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let mut first = fixture.client.attach(terminal_config()).await.unwrap();
    let mut second = fixture.client.attach(terminal_config()).await.unwrap();
    let mut events = fixture.client.0.events.subscribe();
    for round in 0..3 {
        let tx = fixture.core.streams.lock().unwrap().last().unwrap().clone();
        tx.send(TerminalFrame::Output {
            seq: 1,
            bytes: b"shared output".to_vec(),
        })
        .unwrap();
        let (next, frame) = receive_terminal(first).await;
        first = next;
        assert!(
            matches!(frame, TerminalFrame::Output { seq: 11, ref bytes } if bytes == b"shared output")
        );
        let (next, frame) = receive_terminal(second).await;
        second = next;
        assert!(
            matches!(frame, TerminalFrame::Output { seq: 11, ref bytes } if bytes == b"shared output")
        );
        if round == 2 {
            break;
        }
        fixture.drop_route().await;
        loop {
            let event = events.recv().await.unwrap();
            if event.name == "remote/connection" && event.payload["state"] == "reconnecting" {
                break;
            }
        }
        tokio::time::advance(Duration::from_secs(2)).await;
        loop {
            let event = events.recv().await.unwrap();
            if event.name == "remote/connection" && event.payload["state"] == "connected" {
                break;
            }
        }
        let (old, frame) = receive_terminal(first).await;
        assert!(matches!(frame, TerminalFrame::Resync));
        first = fixture.client.attach(terminal_config()).await.unwrap();
        drop(old);
        let (old, frame) = receive_terminal(second).await;
        assert!(matches!(frame, TerminalFrame::Resync));
        second = fixture.client.attach(terminal_config()).await.unwrap();
        drop(old);
        assert_eq!(fixture.core.streams.lock().unwrap().len(), round + 2);
    }
}
#[tokio::test(start_paused = true)]
async fn reconnect_uses_the_latest_viewport_from_ipc_resize_and_take_control() {
    for take in [false, true] {
        let fixture = Fixture::new().await;
        fixture.pair().await;
        let _attachment = fixture.client.attach(terminal_config()).await.unwrap();
        if take {
            fixture
                .client
                .viewport("2".into(), "session".into(), 120, 50, true)
                .await
                .unwrap();
        } else {
            fixture
                .client
                .queue_viewport("2".into(), "session".into(), 120, 50)
                .await
                .unwrap();
        }
        fixture.client.overview("2".into()).await.unwrap();
        let mut events = fixture.client.0.events.subscribe();
        fixture.drop_route().await;
        loop {
            let event = events.recv().await.unwrap();
            if event.name == "remote/connection" && event.payload["state"] == "reconnecting" {
                break;
            }
        }
        tokio::time::advance(Duration::from_secs(2)).await;
        loop {
            let event = events.recv().await.unwrap();
            if event.name == "remote/connection" && event.payload["state"] == "connected" {
                break;
            }
        }
        fixture.client.overview("2".into()).await.unwrap();
        assert_eq!(
            fixture.core.sizes.lock().unwrap().last().copied(),
            Some((120, 50, "ifFree"))
        );
    }
}
#[tokio::test]
async fn emitted_overview_and_ipc_event_debug_redact_bodies_but_serialization_keeps_them() {
    use runner_core::protocol::{wire, ClientEvent};
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture.core.fake.overview.lock().unwrap().missions[0].asks[0].prompt =
        "private-ask-body-canary".into();
    let mut events = fixture.client.0.events.subscribe();
    fixture.client.follow("2".into(), 1, true).await.unwrap();
    let event = loop {
        let event = events.recv().await.unwrap();
        if event.name == "remote/overview" {
            break event;
        }
    };
    assert!(event
        .payload
        .to_string()
        .contains("private-ask-body-canary"));
    assert!(!format!("{event:?}").contains("private-ask-body-canary"));
    let frame = wire::Frame::json(
        wire::EVENT,
        &wire::Event {
            event: Some(ClientEvent {
                name: event.name.into(),
                payload: event.payload,
            }),
            lagged: None,
        },
    )
    .unwrap();
    assert!(String::from_utf8(frame.payload.clone())
        .unwrap()
        .contains("private-ask-body-canary"));
    let event = frame.decode::<wire::Event>().unwrap().event.unwrap();
    assert!(event
        .payload
        .to_string()
        .contains("private-ask-body-canary"));
    assert!(!format!("{event:?}").contains("private-ask-body-canary"));
}
#[tokio::test]
async fn pairs_with_a_utf8_bounded_label_and_keeps_the_full_valid_service_name() {
    let fixture = Fixture::new().await;
    let name = "中".repeat(22);
    let mut saved = Saved::load(fixture.root.path()).unwrap().unwrap();
    saved.host_name = name.clone();
    saved.save(fixture.root.path()).unwrap();
    fixture.pair().await;
    let trust = super::super::trust::Trust::load(fixture.root.path()).unwrap();
    assert_eq!(trust.devices[0].name, "中".repeat(21));
    assert_eq!(trust.devices[0].name.len(), 63);
    assert_eq!(
        Saved::load(fixture.root.path()).unwrap().unwrap().host_name,
        name
    );
    assert!(fixture.client.overview("2".into()).await.is_ok());
}

#[tokio::test]
async fn replay_rollover_resyncs_once_then_both_subscribers_settle() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let mut first = fixture.client.attach(terminal_config()).await.unwrap();
    let tx = fixture.core.streams.lock().unwrap()[0].clone();
    for seq in 1..=33 {
        tx.send(TerminalFrame::Output {
            seq,
            bytes: vec![b'x'; c::TERMINAL_CHUNK_SIZE],
        })
        .unwrap();
        let (next, frame) = receive_terminal(first).await;
        first = next;
        assert!(matches!(frame, TerminalFrame::Output { .. }));
    }
    *fixture.core.fake.bytes.lock().unwrap() = b"rolled snapshot".to_vec();
    let second = fixture.client.attach(terminal_config()).await.unwrap();
    let (old, frame) = receive_terminal(first).await;
    assert!(matches!(frame, TerminalFrame::Resync));
    first = fixture.client.attach(terminal_config()).await.unwrap();
    drop(old);
    assert_eq!(first.snapshot.bytes, b"rolled snapshot");
    assert_eq!(second.snapshot.bytes, b"rolled snapshot");
    assert_eq!(fixture.core.streams.lock().unwrap().len(), 2);
    let tx = fixture.core.streams.lock().unwrap()[1].clone();
    tx.send(TerminalFrame::Output {
        seq: 1,
        bytes: b"settled".to_vec(),
    })
    .unwrap();
    for attachment in [first, second] {
        let (_, frame) = receive_terminal(attachment).await;
        assert!(
            matches!(frame, TerminalFrame::Output { seq: 11, ref bytes } if bytes == b"settled")
        );
    }
}

async fn ipc_fixture(
    fixture: &Fixture,
) -> (
    tempfile::TempDir,
    super::super::tests::SocketHarness,
    runner_core::protocol::DaemonClient,
) {
    let root = super::super::tests::socket_root();
    let mut core = crate::test_support::test_core_in(root.path().into());
    core.events = fixture.client.0.events.clone();
    core.remote = super::super::RemoteHandle {
        client: Some(fixture.client.clone()),
        ..Default::default()
    };
    let server = super::super::tests::SocketHarness::new(root.path(), core).await;
    let client = server.client(Duration::from_secs(5)).await;
    (root, server, client)
}

#[tokio::test]
async fn queued_input_rejections_and_teardown_reach_the_scoped_ipc_error_event() {
    for failure in ["bytes", "generation", "teardown"] {
        // Teardown blocks the writer on the overview's write, which a ping must not reach first.
        let fixture = Fixture::without_ping().await;
        fixture.pair().await;
        let (_root, _server, ipc) = ipc_fixture(&fixture).await;
        let mut events = ipc.subscribe();
        let route = fixture.client.route("2").await.unwrap();
        match failure {
            "bytes" => fixture
                .client
                .input(
                    "2".into(),
                    "session".into(),
                    vec![b'x'; c::MAX_MESSAGE_SIZE],
                )
                .await
                .unwrap(),
            "generation" => route
                .tx
                .try_send(route::Command::Input(
                    route.generation().wrapping_sub(1),
                    "session".into(),
                    b"discarded-generation".to_vec(),
                ))
                .unwrap(),
            _ => {
                let writer = fixture.relay.writer.clone();
                writer.blocked.store(true, Ordering::Release);
                let client = fixture.client.clone();
                let waiting = tokio::spawn(async move { client.overview("2".into()).await });
                writer.entered.notified().await;
                route
                    .input(
                        route.generation(),
                        "session".into(),
                        b"discarded-teardown".to_vec(),
                    )
                    .await
                    .unwrap();
                route.cancel.cancel();
                writer.blocked.store(false, Ordering::Release);
                writer.waker.wake();
                assert!(waiting.await.unwrap().is_err());
            }
        }
        let event = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = events.recv().await.unwrap();
                if event.name == "session/input-error" {
                    break event;
                }
            }
        })
        .await
        .expect(failure);
        assert_eq!(
            event.payload["session_id"],
            remote::terminal_address("2", "session")
        );
        assert!(event.payload["message"]
            .as_str()
            .unwrap()
            .contains(if failure == "bytes" {
                "queue"
            } else {
                "discarded"
            }));
        assert!(fixture.core.fake.input.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn delayed_actions_survive_the_ipc_and_fork_route_deadlines() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let (_root, server, _) = ipc_fixture(&fixture).await;
    let ipc = server.client(Duration::from_millis(40)).await;
    for kind in ["rename", "fork"] {
        let (entered, mut ready) = tokio::sync::mpsc::unbounded_channel();
        let (release, gate) = std::sync::mpsc::channel();
        *fixture.core.action_gate.lock().unwrap() = Some(ActionGate {
            fork_only: false,
            entered,
            release: gate,
        });
        let request = if kind == "fork" {
            serde_json::json!({"type":"fork","id":1,"session":"session","cols":80,"rows":24})
        } else {
            serde_json::json!({"type":"rename","id":1,"target":{"kind":"session","id":"session"},"name":"new"})
        };
        let request = c::ClientMessage::decode(&serde_json::to_vec(&request).unwrap()).unwrap();
        let client = ipc.clone();
        let action =
            tokio::task::spawn_blocking(move || client.remote_request("2".into(), request));
        ready.recv().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !action.is_finished(),
            "generic remote requests must outlive the ordinary IPC budget"
        );
        if kind == "fork" {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(40)).await;
            tokio::task::yield_now().await;
            assert!(!action.is_finished());
            assert!(fixture
                .client
                .route("2")
                .await
                .unwrap()
                .ready
                .load(Ordering::Acquire));
            tokio::time::resume();
        }
        release.send(()).unwrap();
        assert_eq!(action.await.unwrap().unwrap().result["session"], "forked");
        fixture.core.action_gate.lock().unwrap().take();
    }
}

async fn ipc_remote_request(
    client: &runner_core::protocol::DaemonClient,
    value: serde_json::Value,
) -> serde_json::Value {
    let request = c::ClientMessage::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
    let client = client.clone();
    tokio::task::spawn_blocking(move || client.remote_request("2".into(), request))
        .await
        .unwrap()
        .unwrap()
        .result
}

#[tokio::test]
async fn a_pending_fork_does_not_block_unrelated_ipc_requests_or_reconnect_the_route() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let (_root, _server, ipc) = ipc_fixture(&fixture).await;
    let route = fixture.client.route("2").await.unwrap();
    let generation = route.generation();
    let mut events = fixture.client.0.events.subscribe();
    let (entered, mut ready) = tokio::sync::mpsc::unbounded_channel();
    let (release, gate) = std::sync::mpsc::channel();
    *fixture.core.action_gate.lock().unwrap() = Some(ActionGate {
        fork_only: true,
        entered,
        release: gate,
    });
    let client = ipc.clone();
    let fork = tokio::spawn(async move {
        ipc_remote_request(
            &client,
            serde_json::json!({"type":"fork","id":1,"session":"session","cols":80,"rows":24}),
        )
        .await
    });
    ready.recv().await.unwrap();
    for request in [
        serde_json::json!({"type":"layout","id":1,"target":{"kind":"tab","id":"tab"},"name":"Layout","panes":{"id":"p","session":"session"},"shells":[]}),
        serde_json::json!({"type":"rename","id":1,"target":{"kind":"session","id":"session"},"name":"new"}),
        serde_json::json!({"type":"catalog","id":1}),
        serde_json::json!({"type":"feed","id":1,"mission":"m","limit":20}),
    ] {
        ipc_remote_request(&ipc, request).await;
    }
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(40)).await;
    tokio::task::yield_now().await;
    assert!(route.ready.load(Ordering::Acquire));
    assert_eq!(route.generation(), generation);
    assert!(!fork.is_finished());
    ipc_remote_request(&ipc, serde_json::json!({"type":"catalog","id":1})).await;
    tokio::time::advance(Duration::from_secs(20)).await;
    tokio::task::yield_now().await;
    release.send(()).unwrap();
    tokio::time::resume();
    assert_eq!(fork.await.unwrap()["session"], "forked");
    assert_eq!(fixture.core.fork_calls.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.relay.opens.load(Ordering::Relaxed), 1);
    assert!(route.ready.load(Ordering::Acquire));
    while let Ok(event) = events.try_recv() {
        assert!(event.name != "remote/connection" || event.payload["state"] == "connected");
    }
}

#[tokio::test]
async fn unfollow_under_feed_saturation_releases_the_client_route_for_idle_close() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let route = fixture.client.route("2").await.unwrap();
    route
        .request(|id| {
            c::ClientMessage::Follow(c::MissionRequest {
                id,
                mission: "m".into(),
            })
        })
        .await
        .unwrap();
    let (entered, mut ready) = tokio::sync::mpsc::unbounded_channel();
    let (release, gate) = std::sync::mpsc::channel();
    *fixture.core.feed_gate.lock().unwrap() = Some(ActionGate {
        fork_only: false,
        entered,
        release: gate,
    });
    let mut replies = Vec::new();
    for _ in 0..8 {
        let (reply, receive) = oneshot::channel();
        route
            .send(route::Command::Request(
                c::ClientMessage::Feed(c::FeedRequest {
                    id: route.next_id(),
                    mission: "m".into(),
                    before: None,
                    limit: 20,
                }),
                reply,
            ))
            .await
            .unwrap();
        replies.push(receive);
    }
    ready.recv().await.unwrap();
    route
        .request(|id| {
            c::ClientMessage::Unfollow(c::MissionRequest {
                id,
                mission: "m".into(),
            })
        })
        .await
        .unwrap();
    release.send(()).unwrap();
    for reply in replies {
        reply.await.unwrap().unwrap();
    }
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    route.cancel.cancelled().await;
    tokio::time::resume();
    assert_eq!(fixture.relay.opens.load(Ordering::Relaxed), 1);
}

async fn ipc_attach(client: &runner_core::protocol::DaemonClient) -> TerminalAttachment {
    let client = client.clone();
    tokio::task::spawn_blocking(move || client.attach_remote(terminal_config()).unwrap())
        .await
        .unwrap()
}

async fn written(
    events: &mut dyn runner_core::protocol::EventSubscription,
    kinds: &[u8],
    count: usize,
) {
    let mut received = 0;
    while received < count {
        let event = events.recv().await.unwrap();
        if event.name == "test/terminal-written"
            && kinds
                .iter()
                .any(|kind| event.payload["kind"] == u64::from(*kind))
        {
            received += 1;
        }
    }
}

#[tokio::test]
async fn ipc_recovery_after_more_than_64_tiny_frames_settles_and_preserves_content() {
    use runner_core::protocol::wire;
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let (_root, _server, ipc) = ipc_fixture(&fixture).await;
    let mut first = ipc_attach(&ipc).await;
    let mut witness = fixture.client.attach(terminal_config()).await.unwrap();
    let mut events = ipc.subscribe();
    let mut screen = b"screen".to_vec();
    for round in 0..2 {
        let tx = fixture.core.streams.lock().unwrap().last().unwrap().clone();
        for index in 0..80 {
            let frame = if index % 2 == 0 {
                screen.push(b'a' + round);
                TerminalFrame::Output {
                    seq: index + 1,
                    bytes: vec![b'a' + round],
                }
            } else {
                TerminalFrame::Resized {
                    seq: index + 1,
                    cols: 80,
                    rows: 24,
                    controller: TerminalController::Desktop,
                }
            };
            tx.send(frame).unwrap();
            (witness, _) = receive_terminal(witness).await;
        }
        written(&mut *events, &[wire::OUTPUT, wire::RESIZED], 80).await;
        let (old, frame) = receive_terminal(first).await;
        assert!(matches!(frame, TerminalFrame::Resync));
        *fixture.core.fake.bytes.lock().unwrap() = screen.clone();
        first = ipc_attach(&ipc).await;
        assert_eq!(first.snapshot.bytes, screen);
        drop(old);
        let (old, frame) = receive_terminal(witness).await;
        assert!(matches!(frame, TerminalFrame::Resync));
        witness = fixture.client.attach(terminal_config()).await.unwrap();
        drop(old);
        assert_eq!(
            fixture.core.attaches.load(Ordering::Relaxed),
            round as u32 + 2
        );
        let tx = fixture.core.streams.lock().unwrap().last().unwrap().clone();
        tx.send(TerminalFrame::Output {
            seq: 1,
            bytes: b"tail".to_vec(),
        })
        .unwrap();
        written(&mut *events, &[wire::OUTPUT], 1).await;
        for attachment in [&mut first, &mut witness] {
            assert!(
                matches!(attachment.frames.recv().unwrap(), TerminalFrame::Output { bytes, .. } if bytes == b"tail")
            );
        }
        screen.extend_from_slice(b"tail");
    }
}

#[tokio::test]
async fn ipc_slow_resync_keeps_the_final_snapshot_after_the_host_terminal_closes() {
    use runner_core::protocol::wire;
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let (_root, _server, ipc) = ipc_fixture(&fixture).await;
    let first = ipc_attach(&ipc).await;
    let mut events = ipc.subscribe();
    *fixture.core.fake.bytes.lock().unwrap() = b"final screen and exit output".to_vec();
    fixture.core.stopped.store(true, Ordering::Relaxed);
    fixture.core.streams.lock().unwrap()[0]
        .send(TerminalFrame::Resync)
        .unwrap();
    fixture.core.closed.notified().await;
    fixture.client.overview("2".into()).await.unwrap();
    written(&mut *events, &[wire::RESYNC], 1).await;
    let (old, frame) = receive_terminal(first).await;
    assert!(matches!(frame, TerminalFrame::Resync));
    let mut final_attachment = ipc_attach(&ipc).await;
    drop(old);
    assert_eq!(
        final_attachment.snapshot.bytes,
        b"final screen and exit output"
    );
    assert!(
        tokio::task::spawn_blocking(move || final_attachment.frames.recv().is_err())
            .await
            .unwrap()
    );
    assert_eq!(fixture.core.attaches.load(Ordering::Relaxed), 1);
}

#[path = "action_tests.rs"]
mod action_tests;

#[tokio::test]
async fn an_open_route_reports_the_round_trip_of_its_pings_and_stays_connected() {
    let fixture = Fixture::new().await;
    let mut events = fixture.client.0.events.subscribe();
    fixture.pair().await;
    loop {
        let event = events.recv().await.unwrap();
        if event.name == "remote/latency" {
            assert_eq!(event.payload["computer"], "2");
            assert!(event.payload["ms"].as_u64().unwrap() < 10_000);
            break;
        }
    }
    fixture.client.overview("2".into()).await.unwrap();
    let routes = fixture.client.0.routes.lock().await;
    assert!(routes
        .values()
        .all(|route| route.ready.load(Ordering::Acquire)));
}

#[tokio::test]
async fn a_host_that_does_not_list_ping_is_sent_none_and_reports_no_latency() {
    let fixture = Fixture::without_ping().await;
    let mut events = fixture.client.0.events.subscribe();
    fixture.pair().await;
    // The first ping leaves within a timer tick of connecting, and its pong would come before
    // the reply to an overview sent after that tick.
    tokio::time::sleep(Duration::from_millis(50)).await;
    fixture.client.overview("2".into()).await.unwrap();
    let mut connected = false;
    while let Ok(event) = events.try_recv() {
        assert_ne!(event.name, "remote/latency");
        connected |= event.name == "remote/connection" && event.payload["state"] == "connected";
    }
    assert!(connected);
    let routes = fixture.client.0.routes.lock().await;
    assert!(routes
        .values()
        .all(|route| route.ready.load(Ordering::Acquire)));
}

#[tokio::test(start_paused = true)]
async fn latency_stops_with_the_route_and_starts_again_once_it_returns() {
    let fixture = Fixture::new().await;
    let mut events = fixture.client.0.events.subscribe();
    fixture.pair().await;
    fixture.client.follow("2".into(), 1, true).await.unwrap();
    loop {
        if events.recv().await.unwrap().name == "remote/latency" {
            break;
        }
    }
    fixture.drop_route().await;
    loop {
        let event = events.recv().await.unwrap();
        if event.name == "remote/connection" && event.payload["state"] == "reconnecting" {
            break;
        }
    }
    tokio::time::advance(Duration::from_secs(2)).await;
    loop {
        let event = events.recv().await.unwrap();
        assert_ne!(
            event.name, "remote/latency",
            "no sample may survive the route"
        );
        if event.name == "remote/connection" && event.payload["state"] == "connected" {
            break;
        }
    }
    loop {
        if events.recv().await.unwrap().name == "remote/latency" {
            break;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn pings_do_not_keep_an_idle_route_open() {
    let fixture = Fixture::new().await;
    let mut events = fixture.client.0.events.subscribe();
    fixture.pair().await;
    let mut pongs = 0;
    loop {
        let event = events.recv().await.unwrap();
        pongs += u32::from(event.name == "remote/latency");
        if event.name == "remote/connection" && event.payload["state"] == "disconnected" {
            break;
        }
    }
    assert!(pongs > 1, "the idle route should have been pinged");
}
