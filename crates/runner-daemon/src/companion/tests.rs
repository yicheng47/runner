use super::*;
use runner_core::protocol::{terminal::TerminalSubscription, ClientError};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub struct Fake {
    pub overview: Mutex<Overview>,
    pub events: crate::events::EventChannel,
    pub input: Mutex<Vec<(String, Vec<u8>, InputEvent)>>,
    pub history: Mutex<Vec<usize>>,
    pub released: Mutex<Vec<u64>>,
    pub snapshots: AtomicU64,
    overview_calls: AtomicU64,
    overview_gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    overview_started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    capture_overview_before_gate: AtomicBool,
    fail_overview: AtomicBool,
    feed_enabled: AtomicBool,
    feed_calls: AtomicU64,
    feed_gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    feed_started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    feed_since_gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    feed_since_started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    fail_feed_since: AtomicBool,
    requests_enabled: AtomicBool,
    request_gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    request_started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    request_order: Mutex<Vec<String>>,
    appended: Mutex<Vec<FeedEntry>>,
    pub bytes: Mutex<Vec<u8>>,
    continuation: Mutex<(Option<char>, usize)>,
    frames: Mutex<Option<Box<dyn TerminalSubscription>>>,
    closed_reason: Mutex<ClosedReason>,
}
impl Default for Fake {
    fn default() -> Self {
        let HostMessage::Result(result) = serde_json::from_slice::<HostMessage>(include_bytes!(
            "../../../runner-core/src/companion/fixtures/result_overview.json"
        ))
        .unwrap() else {
            panic!()
        };
        Self {
            overview: Mutex::new(serde_json::from_value(result.result).unwrap()),
            events: Default::default(),
            input: Default::default(),
            history: Default::default(),
            released: Default::default(),
            snapshots: AtomicU64::new(0),
            overview_calls: AtomicU64::new(0),
            overview_gate: Default::default(),
            overview_started: Default::default(),
            capture_overview_before_gate: Default::default(),
            fail_overview: Default::default(),
            feed_enabled: AtomicBool::new(false),
            feed_calls: AtomicU64::new(0),
            feed_gate: Default::default(),
            feed_started: Default::default(),
            feed_since_gate: Default::default(),
            feed_since_started: Default::default(),
            fail_feed_since: Default::default(),
            requests_enabled: AtomicBool::new(false),
            request_gate: Default::default(),
            request_started: Default::default(),
            request_order: Default::default(),
            appended: Default::default(),
            bytes: Mutex::new(b"screen".to_vec()),
            continuation: Mutex::new((None, 0)),
            frames: Default::default(),
            closed_reason: Mutex::new(ClosedReason::Exited),
        }
    }
}
impl Fake {
    fn request_gate(&self, name: String) -> core::Result<()> {
        if !self.requests_enabled.load(Ordering::Relaxed) {
            return Err(Failure(
                ErrorCode::UnknownRequest,
                "The host does not serve that request.",
            ));
        }
        self.request_order.lock().unwrap().push(name);
        let gate = self.request_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            if let Some(started) = self.request_started.lock().unwrap().take() {
                let _ = started.send(());
            }
            gate.recv().unwrap();
        }
        Ok(())
    }
}
#[derive(Default)]
struct TestQueue {
    state: Mutex<(VecDeque<TerminalFrame>, bool)>,
    ready: std::sync::Condvar,
}
struct Frames(Arc<TestQueue>);
impl TerminalSubscription for Frames {
    fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
        let mut state = self.0.state.lock().unwrap();
        loop {
            if let Some(frame) = state.0.pop_front() {
                return Ok(frame);
            }
            if state.1 {
                return Err(ClientError::msg("closed"));
            }
            state = self.0.ready.wait(state).unwrap();
        }
    }
    fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
        let queue = self.0.clone();
        Arc::new(move || {
            queue.state.lock().unwrap().1 = true;
            queue.ready.notify_one();
        })
    }
}
impl Core for Fake {
    fn feed_since(&self, _: &str, cursor: u64) -> core::Result<feed::Updates> {
        if !self.requests_enabled.load(Ordering::Relaxed)
            && !self.feed_enabled.load(Ordering::Relaxed)
        {
            return Err(Failure(
                ErrorCode::UnknownRequest,
                "The host does not serve that request.",
            ));
        }
        let entries = std::mem::take(&mut *self.appended.lock().unwrap());
        let gate = self.feed_since_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            if let Some(started) = self.feed_since_started.lock().unwrap().take() {
                let _ = started.send(());
            }
            gate.recv().unwrap();
        }
        if self.fail_feed_since.load(Ordering::Relaxed) {
            return Err(Failure(ErrorCode::Failed, "test feed failure"));
        }
        Ok(feed::Updates {
            cursor: cursor + entries.len() as u64,
            entries,
            more: false,
        })
    }
    fn action(&self, request: ActionRequest) -> core::Result<serde_json::Value> {
        self.request_gate(format!("action-{}", request.id()))?;
        if let ActionRequest::Layout { name, .. } = request {
            self.overview.lock().unwrap().tabs[0].name = name;
        }
        Ok(serde_json::json!({"session":"action-result"}))
    }
    fn feed_cursor(&self, mission: &str) -> core::Result<u64> {
        self.request_gate(format!("follow-{mission}"))?;
        Ok(17)
    }
    fn feed(&self, request: FeedRequest) -> core::Result<FeedPage> {
        if !self.feed_enabled.load(Ordering::Relaxed) {
            return Err(Failure(
                ErrorCode::UnknownRequest,
                "The host does not serve that request.",
            ));
        }
        self.feed_calls.fetch_add(1, Ordering::Relaxed);
        let gate = self.feed_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            if let Some(started) = self.feed_started.lock().unwrap().take() {
                let _ = started.send(());
            }
            gate.recv().unwrap();
        }
        Ok(FeedPage {
            entries: vec![],
            before: Some(request.id.to_string()),
        })
    }
    fn statuses(&self) -> core::Result<BTreeMap<String, SessionStatus>> {
        Ok(self
            .overview
            .lock()
            .unwrap()
            .sessions
            .iter()
            .map(|(id, session)| (id.clone(), session.status.clone()))
            .collect())
    }
    fn overview(&self) -> core::Result<Overview> {
        self.overview_calls.fetch_add(1, Ordering::Relaxed);
        let fail = self.fail_overview.swap(false, Ordering::Relaxed);
        let captured = self
            .capture_overview_before_gate
            .load(Ordering::Relaxed)
            .then(|| self.overview.lock().unwrap().clone());
        let gate = self.overview_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            if let Some(started) = self.overview_started.lock().unwrap().take() {
                let _ = started.send(());
            }
            gate.recv().unwrap();
        }
        if fail {
            return Err(Failure(ErrorCode::Failed, "test overview failure"));
        }
        Ok(captured.unwrap_or_else(|| self.overview.lock().unwrap().clone()))
    }
    fn subscribe(&self) -> core::Result<broadcast::Receiver<AppEvent>> {
        Ok(self.events.subscribe())
    }
    fn attach(
        self: Arc<Self>,
        session: &str,
        history: usize,
        _channel: u64,
    ) -> core::Result<core::Attachment> {
        self.history.lock().unwrap().push(history);
        let subscriber_id = self.snapshots.fetch_add(1, Ordering::Relaxed) + 1;
        let (preceding_char, unfinished_len) = *self.continuation.lock().unwrap();
        let terminal = TerminalAttachment {
            subscriber_id,
            snapshot: TerminalSnapshot {
                seq: subscriber_id * 10,
                cols: 80,
                rows: 24,
                controller: TerminalController::Desktop,
                bytes: self.bytes.lock().unwrap().clone(),
                unfinished_len,
                preceding_char,
            },
            frames: self
                .frames
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Box::new(Frames(Arc::new(TestQueue::default())))),
        };
        let session = session.to_owned();
        Ok(core::Attachment {
            terminal,
            resync: Arc::new(move || {
                self.clone()
                    .attach(&session, history, 0)
                    .map(|a| a.terminal)
            }),
        })
    }
    fn resize(&self, _: &str, _: u64, _: &str, _: (u16, u16), _: Control) -> core::Result<()> {
        Ok(())
    }
    fn input(&self, session: &str, _: u64, bytes: &[u8]) -> core::Result<()> {
        self.input
            .lock()
            .unwrap()
            .push((session.into(), bytes.to_vec(), classify_input(bytes)));
        Ok(())
    }
    fn detach(&self, _: &str, _: u64, _: u64, _: bool) {}
    fn closed_reason(&self, _: &str) -> ClosedReason {
        self.closed_reason.lock().unwrap().clone()
    }
    fn release_channel(&self, channel: u64) {
        self.released.lock().unwrap().push(channel);
    }
}
fn request(value: serde_json::Value) -> ClientMessage {
    ClientMessage::decode(&serde_json::to_vec(&value).unwrap()).unwrap()
}
async fn next(adapter: &mut Adapter) -> serde_json::Value {
    loop {
        if let Some(bytes) = adapter.next(true).await.unwrap() {
            return serde_json::from_slice(&bytes).unwrap();
        }
        assert!(
            adapter.overview_work.is_some()
                || adapter.request_work.is_some()
                || adapter.status_work.is_some()
                || adapter.feed_read.is_some()
                || adapter.feed_ready.is_some()
                || adapter.output_due.is_some()
                || adapter.overview_due.is_some()
                || adapter.action_work.is_some()
                || !adapter.forks.is_empty(),
            "no pending message"
        );
        adapter.changed().await.unwrap();
    }
}
async fn next_non_overview(adapter: &mut Adapter) -> serde_json::Value {
    loop {
        let message = next(adapter).await;
        if message["type"] != "overview" {
            return message;
        }
    }
}
// An overview that finishes before the status scan carries the change and supersedes its status push.
async fn next_stopped(adapter: &mut Adapter, session: &str) {
    loop {
        let message = next(adapter).await;
        let status = match message["type"].as_str() {
            Some("status") => &message["status"],
            Some("overview") => &message["sessions"][session]["status"],
            _ => panic!("expected the stopped status, got {message}"),
        };
        if status == "stopped" {
            return;
        }
    }
}
async fn attach(adapter: &mut Adapter, session: &str) {
    adapter.request(request(serde_json::json!({"type":"attach","id":1,"session":session,"cols":80,"rows":24,"control":"take","history":2}))).await.unwrap();
    assert_eq!(next(adapter).await["type"], "result");
}
fn quiet(adapter: &mut Adapter) {
    for stream in &adapter.streams {
        stream.queue.lock().unwrap().ended = false;
    }
}
#[tokio::test]
async fn overview_matches_the_golden_and_unserved_requests_remain_unknown() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 7, "browser".into()).unwrap();
    adapter
        .request(request(serde_json::json!({"type":"overview","id":11})))
        .await
        .unwrap();
    let golden: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../runner-core/src/companion/fixtures/result_overview.json"
    ))
    .unwrap();
    assert_eq!(next(&mut adapter).await["result"], golden["result"]);
    for (i, fixture) in [
        include_str!("../../../runner-core/src/companion/fixtures/feed_request.json"),
        include_str!("../../../runner-core/src/companion/fixtures/follow.json"),
        r#"{"type":"settings","id":1}"#,
        include_str!("../../../runner-core/src/companion/fixtures/answer.json"),
        include_str!("../../../runner-core/src/companion/fixtures/post.json"),
        include_str!("../../../runner-core/src/companion/fixtures/role.json"),
        include_str!("../../../runner-core/src/companion/fixtures/crew.json"),
        include_str!("../../../runner-core/src/companion/fixtures/create_role.json"),
        include_str!("../../../runner-core/src/companion/fixtures/update_role.json"),
        include_str!("../../../runner-core/src/companion/fixtures/delete_role.json"),
        include_str!("../../../runner-core/src/companion/fixtures/create_crew.json"),
        include_str!("../../../runner-core/src/companion/fixtures/update_crew.json"),
        include_str!("../../../runner-core/src/companion/fixtures/delete_crew.json"),
        include_str!("../../../runner-core/src/companion/fixtures/add_slot.json"),
        include_str!("../../../runner-core/src/companion/fixtures/update_slot.json"),
        include_str!("../../../runner-core/src/companion/fixtures/remove_slot.json"),
        include_str!("../../../runner-core/src/companion/fixtures/reorder_slots.json"),
        include_str!("../../../runner-core/src/companion/fixtures/set_lead.json"),
    ]
    .iter()
    .enumerate()
    {
        let mut value: serde_json::Value = serde_json::from_str(fixture).unwrap();
        value["id"] = i.into();
        adapter.request(request(value)).await.unwrap();
        let reply = next(&mut adapter).await;
        assert_eq!(reply["code"], "unknownRequest");
        assert_eq!(reply["id"], i);
    }
    drop(adapter);
    assert_eq!(*fake.released.lock().unwrap(), [7]);
}
#[tokio::test(start_paused = true)]
async fn overview_coalesces_and_delivers_the_final_change_and_status() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    adapter
        .request(request(serde_json::json!({"type":"overview","id":1})))
        .await
        .unwrap();
    next(&mut adapter).await;
    fake.events.emit("chat/layout-changed", &());
    adapter.changed().await.unwrap();
    adapter.changed().await.unwrap();
    assert_eq!(next(&mut adapter).await["type"], "overview");
    let id = fake
        .overview
        .lock()
        .unwrap()
        .sessions
        .keys()
        .next()
        .unwrap()
        .clone();
    for name in ["second", "last"] {
        {
            let mut overview = fake.overview.lock().unwrap();
            overview.tabs[0].name = name.into();
            overview.sessions.get_mut(&id).unwrap().status.unread = true;
        }
        fake.events.emit("chat/tab-attention-changed", &());
        adapter.changed().await.unwrap();
    }
    assert_eq!(next(&mut adapter).await["type"], "status");
    assert!(adapter.next(false).await.unwrap().is_none());
    tokio::time::advance(Duration::from_millis(250)).await;
    adapter.changed().await.unwrap();
    assert_eq!(next(&mut adapter).await["tabs"][0]["name"], "last");
}

#[tokio::test(start_paused = true)]
async fn a_host_palette_change_pushes_the_overview() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    adapter
        .request(request(serde_json::json!({"type":"overview","id":1})))
        .await
        .unwrap();
    next(&mut adapter).await;
    fake.events.emit("terminal/palette", &());
    adapter.changed().await.unwrap();
    adapter.changed().await.unwrap();
    assert_eq!(next(&mut adapter).await["type"], "overview");
}

#[tokio::test]
async fn an_old_status_worker_cannot_undo_a_newer_explicit_overview_snapshot() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    adapter.initialize().await.unwrap();
    let old = adapter.statuses.clone();
    let session = old.keys().next().unwrap().clone();
    let (release, held) = tokio::sync::oneshot::channel();
    adapter.status_work = Some(tokio::spawn(async move {
        held.await.unwrap();
        Ok(old)
    }));
    adapter.status_started_revision = adapter.status_revision;
    fake.overview
        .lock()
        .unwrap()
        .sessions
        .get_mut(&session)
        .unwrap()
        .status
        .unread = true;
    adapter
        .request(request(serde_json::json!({"type":"overview","id":99})))
        .await
        .unwrap();
    loop {
        let message = next(&mut adapter).await;
        if message["id"] == 99 {
            break;
        }
    }
    assert!(adapter.statuses[&session].unread);
    release.send(()).unwrap();
    while adapter.status_work.is_some() {
        adapter.changed().await.unwrap();
    }
    assert!(adapter.statuses[&session].unread);
    while let Some(bytes) = adapter.next(false).await.unwrap() {
        let message: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        if message["type"] == "status" {
            assert_eq!(message["unread"], true);
        }
    }
}

#[tokio::test]
async fn debounced_overview_work_survives_cancellation_without_duplicate_scans() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    adapter.initialize().await.unwrap();
    tokio::time::pause();
    adapter.last_overview = Some(Instant::now());
    for _ in 0..32 {
        fake.events.emit("event/appended", &());
        adapter.changed().await.unwrap();
    }
    assert_eq!(fake.overview_calls.load(Ordering::Relaxed), 0);
    let (release, gate) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    *fake.overview_gate.lock().unwrap() = Some(gate);
    *fake.overview_started.lock().unwrap() = Some(started);
    tokio::time::advance(Duration::from_millis(250)).await;
    while adapter.overview_work.is_none() {
        adapter.changed().await.unwrap();
    }
    ready.await.unwrap();
    while adapter.status_work.is_some() || !adapter.events.is_empty() {
        adapter.changed().await.unwrap();
    }
    tokio::time::resume();
    for _ in 0..16 {
        assert!(
            tokio::time::timeout(Duration::from_millis(1), adapter.changed())
                .await
                .is_err()
        );
        assert_eq!(fake.overview_calls.load(Ordering::Relaxed), 1);
    }
    fake.overview.lock().unwrap().tabs[0].name = "Final tab".into();
    let id = fake
        .overview
        .lock()
        .unwrap()
        .sessions
        .keys()
        .next()
        .unwrap()
        .clone();
    fake.overview
        .lock()
        .unwrap()
        .sessions
        .get_mut(&id)
        .unwrap()
        .status
        .unread = true;
    fake.events.emit("chat/tab-attention-changed", &());
    adapter.changed().await.unwrap();
    assert_eq!(next(&mut adapter).await["type"], "status");
    assert_eq!(fake.overview_calls.load(Ordering::Relaxed), 1);
    release.send(()).unwrap();
    assert_eq!(next(&mut adapter).await["tabs"][0]["name"], "Final tab");
    assert_eq!(fake.overview_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn slow_actions_and_follows_keep_terminals_live_and_preserve_request_order() {
    for slow in ["action", "follow"] {
        let fake = Arc::new(Fake::default());
        fake.requests_enabled.store(true, Ordering::Relaxed);
        let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
        attach(&mut adapter, "a").await;
        next(&mut adapter).await;
        next(&mut adapter).await;
        let (release, gate) = std::sync::mpsc::channel();
        let (started, ready) = tokio::sync::oneshot::channel();
        *fake.request_gate.lock().unwrap() = Some(gate);
        *fake.request_started.lock().unwrap() = Some(started);
        let first = if slow == "action" {
            serde_json::json!({"type":"rename","id":100,"target":{"kind":"session","id":"a"},"name":"first"})
        } else {
            serde_json::json!({"type":"follow","id":100,"mission":"m"})
        };
        adapter.request(request(first)).await.unwrap();
        ready.await.unwrap();
        for queued in [
            serde_json::json!({"type":"follow","id":101,"mission":"m"}),
            serde_json::json!({"type":"unfollow","id":102,"mission":"m"}),
            serde_json::json!({"type":"rename","id":103,"target":{"kind":"session","id":"a"},"name":"last"}),
        ] {
            adapter.request(request(queued)).await.unwrap();
        }
        assert!(adapter.requests.len() <= 3 && adapter.actions.len() <= 1);
        adapter
            .request(request(
                serde_json::json!({"type":"input","id":999,"session":"a","data":"eA=="}),
            ))
            .await
            .unwrap();
        assert_eq!(next(&mut adapter).await["id"], 999);
        assert_eq!(fake.input.lock().unwrap()[0].1, b"x");
        adapter.streams[0]
            .queue
            .lock()
            .unwrap()
            .push(TerminalFrame::Output {
                seq: 11,
                bytes: b"live output".to_vec(),
            });
        assert_eq!(next(&mut adapter).await["type"], "output");
        release.send(()).unwrap();
        let mut ids = std::collections::BTreeSet::new();
        for _ in 100..104 {
            let reply = next(&mut adapter).await;
            assert_eq!(reply["type"], "result");
            ids.insert(reply["id"].as_u64().unwrap());
        }
        assert_eq!(ids, (100..104).collect());
        assert!(!adapter.feeds.contains_key("m"));
        let order = fake.request_order.lock().unwrap();
        assert_eq!(order.len(), 3);
        if slow == "action" {
            assert!(
                order.iter().position(|s| s == "action-100").unwrap()
                    < order.iter().position(|s| s == "action-103").unwrap()
            );
        } else {
            assert_eq!(order.iter().filter(|s| s.as_str() == "follow-m").count(), 2);
        }
        assert!(adapter.requests.is_empty() && adapter.request_work.is_none());
    }
}

#[tokio::test]
async fn a_slow_fork_does_not_block_actions_feed_or_follow() {
    let fake = Arc::new(Fake::default());
    fake.requests_enabled.store(true, Ordering::Relaxed);
    fake.feed_enabled.store(true, Ordering::Relaxed);
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    let (release, gate) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    *fake.request_gate.lock().unwrap() = Some(gate);
    *fake.request_started.lock().unwrap() = Some(started);
    adapter
        .request(request(
            serde_json::json!({"type":"fork","id":100,"session":"a","cols":80,"rows":24}),
        ))
        .await
        .unwrap();
    ready.await.unwrap();
    for queued in [
        serde_json::json!({"type":"rename","id":101,"target":{"kind":"session","id":"a"},"name":"new"}),
        serde_json::json!({"type":"feed","id":102,"mission":"m","limit":20}),
        serde_json::json!({"type":"follow","id":103,"mission":"m"}),
    ] {
        adapter.request(request(queued)).await.unwrap();
    }
    let mut replies = std::collections::BTreeSet::new();
    for _ in 0..3 {
        let reply = next(&mut adapter).await;
        assert_eq!(reply["type"], "result");
        replies.insert(reply["id"].as_u64().unwrap());
    }
    assert_eq!(replies, (101..104).collect());
    assert_eq!(adapter.forks.len(), 1);
    assert!(adapter.feeds.contains_key("m"));
    for id in 104..112 {
        adapter
            .request(request(
                serde_json::json!({"type":"fork","id":id,"session":"a","cols":80,"rows":24}),
            ))
            .await
            .unwrap();
    }
    assert_eq!(adapter.forks.len(), 8);
    let refused = next(&mut adapter).await;
    assert_eq!(refused["type"], "error");
    assert_eq!(refused["id"], 111);
    release.send(()).unwrap();
    let mut replies = std::collections::BTreeSet::new();
    for _ in 0..8 {
        let reply = next(&mut adapter).await;
        assert_eq!(reply["type"], "result");
        replies.insert(reply["id"].as_u64().unwrap());
    }
    assert_eq!(replies, std::iter::once(100).chain(104..111).collect());
    assert!(adapter.forks.is_empty());
}

#[tokio::test]
async fn unfollow_cleanup_ids_share_the_existing_encoded_byte_budget() {
    let fake = Arc::new(Fake::default());
    fake.requests_enabled.store(true, Ordering::Relaxed);
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    let (release, gate) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    *fake.request_gate.lock().unwrap() = Some(gate);
    *fake.request_started.lock().unwrap() = Some(started);
    adapter
        .request(request(
            serde_json::json!({"type":"follow","id":1,"mission":"m"}),
        ))
        .await
        .unwrap();
    ready.await.unwrap();
    adapter
        .request(request(
            serde_json::json!({"type":"unfollow","id":2,"mission":"m"}),
        ))
        .await
        .unwrap();
    let reserved = adapter.cleanup_bytes;
    adapter.in_flight(MAX_MESSAGE_SIZE - 2 * reserved);
    adapter
        .request(request(
            serde_json::json!({"type":"unfollow","id":3,"mission":"m"}),
        ))
        .await
        .unwrap();
    assert_eq!(adapter.cleanup_bytes, 2 * reserved);
    assert_eq!(adapter.requests.len(), 1);
    assert_eq!(
        adapter
            .request(request(
                serde_json::json!({"type":"unfollow","id":4,"mission":"m"})
            ))
            .await,
        Err("reply queue full")
    );
    assert_eq!(adapter.cleanup_bytes, 2 * reserved);
    adapter.in_flight(0);
    release.send(()).unwrap();
    for id in 1..4 {
        assert_eq!(next(&mut adapter).await["id"], id);
    }
    assert!(adapter.feeds.is_empty());
    assert_eq!(adapter.cleanup_bytes, 0);
}

#[tokio::test]
async fn saturated_feed_work_reserves_and_coalesces_unfollow_cleanup() {
    let fake = Arc::new(Fake::default());
    fake.requests_enabled.store(true, Ordering::Relaxed);
    fake.feed_enabled.store(true, Ordering::Relaxed);
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    adapter.feeds.insert(
        "active".into(),
        FeedSubscription {
            cursor: 0,
            dirty: false,
        },
    );
    let (release, gate) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    *fake.request_gate.lock().unwrap() = Some(gate);
    *fake.request_started.lock().unwrap() = Some(started);
    adapter
        .request(request(
            serde_json::json!({"type":"follow","id":100,"mission":"pending"}),
        ))
        .await
        .unwrap();
    ready.await.unwrap();
    for id in 101..108 {
        adapter
            .request(request(
                serde_json::json!({"type":"feed","id":id,"mission":"m","limit":20}),
            ))
            .await
            .unwrap();
    }
    for (id, mission) in [
        (200, "active"),
        (201, "pending"),
        (202, "pending"),
        (203, "unrelated"),
    ] {
        adapter
            .request(request(
                serde_json::json!({"type":"unfollow","id":id,"mission":mission}),
            ))
            .await
            .unwrap();
    }
    assert!(!adapter.feeds.contains_key("active"));
    assert_eq!(adapter.feed_outstanding, 8);
    assert_eq!(adapter.requests.len(), 8);
    assert!(
        matches!(adapter.requests.back(), Some(FeedJob::Unfollow {ids, ..}) if ids == &[201,202])
    );
    assert!(adapter.cleanup_bytes > 0 && adapter.cleanup_bytes < MAX_MESSAGE_SIZE);
    for id in [200, 203] {
        assert_eq!(next(&mut adapter).await["id"], id);
    }
    release.send(()).unwrap();
    for id in (100..108).chain([201, 202]) {
        let reply = next(&mut adapter).await;
        assert_eq!(reply["type"], "result");
        assert_eq!(reply["id"], id);
    }
    assert!(adapter.feeds.is_empty());
    assert_eq!(adapter.feed_outstanding, 0);
    assert_eq!(adapter.cleanup_bytes, 0);
    assert!(adapter.request_work.is_none() && adapter.requests.is_empty());
}

#[tokio::test]
async fn explicit_overview_after_mutation_waits_for_a_fresh_bounded_scan() {
    for mutation in ["layout", "external"] {
        let fake = Arc::new(Fake::default());
        fake.requests_enabled.store(true, Ordering::Relaxed);
        fake.capture_overview_before_gate
            .store(true, Ordering::Relaxed);
        {
            let mut overview = fake.overview.lock().unwrap();
            let session = overview.sessions.values().next().unwrap().clone();
            overview.sessions.insert("a".into(), session);
        }
        let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
        attach(&mut adapter, "a").await;
        next(&mut adapter).await;
        next(&mut adapter).await;
        quiet(&mut adapter);
        let (release, gate) = std::sync::mpsc::channel();
        let (started, ready) = tokio::sync::oneshot::channel();
        *fake.overview_gate.lock().unwrap() = Some(gate);
        *fake.overview_started.lock().unwrap() = Some(started);
        fake.events.emit("chat/layout-changed", &());
        while adapter.overview_work.is_none() {
            adapter.changed().await.unwrap();
        }
        ready.await.unwrap();
        if mutation == "layout" {
            adapter.request(request(serde_json::json!({"type":"layout","id":50,"target":{"kind":"tab","id":"tab"},"name":"Updated","panes":{"id":"p","session":"a"},"shells":[]}))).await.unwrap();
            assert_eq!(next(&mut adapter).await["id"], 50);
        } else {
            fake.overview.lock().unwrap().tabs[0].name = "Updated".into();
        }
        for id in 100..109 {
            adapter
                .request(request(serde_json::json!({"type":"overview","id":id})))
                .await
                .unwrap();
        }
        let refused = next(&mut adapter).await;
        assert_eq!(refused["type"], "error");
        assert_eq!(refused["id"], 108);
        adapter
            .request(request(
                serde_json::json!({"type":"input","id":999,"session":"a","data":"eA=="}),
            ))
            .await
            .unwrap();
        assert_eq!(next(&mut adapter).await["id"], 999);
        assert_eq!(fake.input.lock().unwrap()[0].1, b"x");
        adapter.streams[0]
            .queue
            .lock()
            .unwrap()
            .push(TerminalFrame::Output {
                seq: 11,
                bytes: b"live output".to_vec(),
            });
        assert_eq!(next(&mut adapter).await["type"], "output");
        assert_eq!(fake.overview_calls.load(Ordering::Relaxed), 1);
        let (release_next, gate) = std::sync::mpsc::channel();
        let (started, ready_next) = tokio::sync::oneshot::channel();
        *fake.overview_gate.lock().unwrap() = Some(gate);
        *fake.overview_started.lock().unwrap() = Some(started);
        release.send(()).unwrap();
        let old = next(&mut adapter).await;
        assert_eq!(old["type"], "overview");
        assert_ne!(old["tabs"][0]["name"], "Updated");
        ready_next.await.unwrap();
        assert_eq!(adapter.overview_waiters.len(), 8);
        assert!(adapter.overview_next.is_empty());
        adapter
            .request(request(
                serde_json::json!({"type":"input","id":998,"session":"a","data":"eQ=="}),
            ))
            .await
            .unwrap();
        assert_eq!(next(&mut adapter).await["id"], 998);
        assert_eq!(fake.input.lock().unwrap()[1].1, b"y");
        adapter.streams[0]
            .queue
            .lock()
            .unwrap()
            .push(TerminalFrame::Output {
                seq: 12,
                bytes: b"still live".to_vec(),
            });
        assert_eq!(next(&mut adapter).await["type"], "output");
        assert_eq!(fake.overview_calls.load(Ordering::Relaxed), 2);
        release_next.send(()).unwrap();
        let mut ids = std::collections::BTreeSet::new();
        while ids.len() < 8 {
            let reply = next(&mut adapter).await;
            if reply["type"] != "result" {
                continue;
            }
            assert_eq!(reply["result"]["tabs"][0]["name"], "Updated", "{mutation}");
            assert!(ids.insert(reply["id"].as_u64().unwrap()));
        }
        assert_eq!(ids, (100..108).collect());
        assert!(
            adapter.overview_waiters.is_empty()
                && adapter.overview_next.is_empty()
                && adapter.overview_replies.is_empty()
        );
        assert_eq!(fake.overview_calls.load(Ordering::Relaxed), 2);
    }
}

#[tokio::test]
async fn explicit_overview_waiters_form_bounded_batches_without_stalling_terminal_io() {
    let fake = Arc::new(Fake::default());
    {
        let mut overview = fake.overview.lock().unwrap();
        let session = overview.sessions.values().next().unwrap().clone();
        overview.sessions.insert("a".into(), session);
    }
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    attach(&mut adapter, "a").await;
    next(&mut adapter).await;
    next(&mut adapter).await;
    quiet(&mut adapter);
    let (release, gate) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    *fake.overview_gate.lock().unwrap() = Some(gate);
    *fake.overview_started.lock().unwrap() = Some(started);
    adapter
        .request(request(serde_json::json!({"type":"overview","id":100})))
        .await
        .unwrap();
    ready.await.unwrap();
    for id in 101..109 {
        adapter
            .request(request(serde_json::json!({"type":"overview","id":id})))
            .await
            .unwrap();
    }
    let refused = next(&mut adapter).await;
    assert_eq!(refused["type"], "error");
    assert_eq!(refused["id"], 108);
    adapter
        .request(request(
            serde_json::json!({"type":"input","id":999,"session":"a","data":"eA=="}),
        ))
        .await
        .unwrap();
    assert_eq!(next(&mut adapter).await["id"], 999);
    assert_eq!(fake.input.lock().unwrap()[0].1, b"x");
    adapter.streams[0]
        .queue
        .lock()
        .unwrap()
        .push(TerminalFrame::Output {
            seq: 11,
            bytes: b"live output".to_vec(),
        });
    assert_eq!(next(&mut adapter).await["type"], "output");
    assert_eq!(fake.overview_calls.load(Ordering::Relaxed), 1);
    release.send(()).unwrap();
    for id in 100..108 {
        assert_eq!(next(&mut adapter).await["id"], id);
    }
    assert!(adapter.overview_waiters.is_empty() && adapter.overview_replies.is_empty());
    assert_eq!(fake.overview_calls.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn background_overview_errors_do_not_fail_later_admitted_requests() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    let (release, gate) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    *fake.overview_gate.lock().unwrap() = Some(gate);
    *fake.overview_started.lock().unwrap() = Some(started);
    fake.fail_overview.store(true, Ordering::Relaxed);
    adapter.overview_broadcast = true;
    adapter.start_overview();
    ready.await.unwrap();
    fake.overview.lock().unwrap().tabs[0].name = "Recovered".into();
    for id in 1..4 {
        adapter
            .request(request(serde_json::json!({"type":"overview","id":id})))
            .await
            .unwrap();
    }
    release.send(()).unwrap();
    for id in 1..4 {
        let reply = next(&mut adapter).await;
        assert_eq!(reply["type"], "result");
        assert_eq!(reply["id"], id);
        assert_eq!(reply["result"]["tabs"][0]["name"], "Recovered");
    }
    assert_eq!(fake.overview_calls.load(Ordering::Relaxed), 2);
    assert!(
        adapter.overview_waiters.is_empty()
            && adapter.overview_next.is_empty()
            && adapter.overview_replies.is_empty()
    );
}

#[tokio::test]
async fn overview_errors_only_fail_their_admitted_batch_and_later_requests_start_fresh() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    let (release, gate) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    *fake.overview_gate.lock().unwrap() = Some(gate);
    *fake.overview_started.lock().unwrap() = Some(started);
    adapter
        .request(request(serde_json::json!({"type":"overview","id":1})))
        .await
        .unwrap();
    ready.await.unwrap();
    for id in [2, 3] {
        adapter
            .request(request(serde_json::json!({"type":"overview","id":id})))
            .await
            .unwrap();
    }
    let (release_next, gate) = std::sync::mpsc::channel();
    let (started, ready_next) = tokio::sync::oneshot::channel();
    *fake.overview_gate.lock().unwrap() = Some(gate);
    *fake.overview_started.lock().unwrap() = Some(started);
    fake.fail_overview.store(true, Ordering::Relaxed);
    release.send(()).unwrap();
    let first = next(&mut adapter).await;
    assert_eq!(first["type"], "result");
    assert_eq!(first["id"], 1);
    ready_next.await.unwrap();
    fake.overview.lock().unwrap().tabs[0].name = "After failure".into();
    adapter
        .request(request(serde_json::json!({"type":"overview","id":4})))
        .await
        .unwrap();
    assert_eq!(adapter.overview_next, [4]);
    release_next.send(()).unwrap();
    for id in [2, 3] {
        let failed = next(&mut adapter).await;
        assert_eq!(failed["type"], "error");
        assert_eq!(failed["id"], id);
        assert_eq!(failed["code"], "failed");
    }
    let last = next(&mut adapter).await;
    assert_eq!(last["type"], "result");
    assert_eq!(last["id"], 4);
    assert_eq!(last["result"]["tabs"][0]["name"], "After failure");
    assert_eq!(fake.overview_calls.load(Ordering::Relaxed), 3);
    assert!(
        adapter.overview_waiters.is_empty()
            && adapter.overview_next.is_empty()
            && adapter.overview_replies.is_empty()
    );
}

#[tokio::test]
async fn follow_catches_an_append_during_cursor_work_without_another_event() {
    let fake = Arc::new(Fake::default());
    fake.requests_enabled.store(true, Ordering::Relaxed);
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    let (release, gate) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    *fake.request_gate.lock().unwrap() = Some(gate);
    *fake.request_started.lock().unwrap() = Some(started);
    adapter
        .request(request(
            serde_json::json!({"type":"follow","id":1,"mission":"m"}),
        ))
        .await
        .unwrap();
    ready.await.unwrap();
    let entry = FeedEntry {
        id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
        ts: chrono::Utc::now().to_rfc3339(),
        from: "coder".into(),
        to: vec![],
        content: FeedContent::Message {
            text: "during Follow".into(),
        },
    };
    fake.appended.lock().unwrap().push(entry.clone());
    fake.events
        .emit("event/appended", &serde_json::json!({"mission_id":"m"}));
    adapter.changed().await.unwrap();
    assert!(!adapter.feeds.contains_key("m"));
    release.send(()).unwrap();
    // The append also schedules an overview broadcast, which may land before the follow reply.
    assert_eq!(next_non_overview(&mut adapter).await["id"], 1);
    let pushed = next_non_overview(&mut adapter).await;
    assert_eq!(pushed["type"], "feedAppended");
    assert_eq!(pushed["entries"][0]["id"], entry.id);
}

#[tokio::test]
async fn slow_feed_pages_keep_input_output_live_with_bounded_requests_and_matching_ids() {
    let fake = Arc::new(Fake::default());
    fake.feed_enabled.store(true, Ordering::Relaxed);
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    attach(&mut adapter, "a").await;
    next(&mut adapter).await;
    next(&mut adapter).await;
    let (release, gate) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    *fake.feed_gate.lock().unwrap() = Some(gate);
    *fake.feed_started.lock().unwrap() = Some(started);
    for id in 100..109 {
        adapter
            .request(request(
                serde_json::json!({"type":"feed","id":id,"mission":"m","limit":20}),
            ))
            .await
            .unwrap();
    }
    ready.await.unwrap();
    assert_eq!(adapter.requests.len(), 7);
    assert_eq!(fake.feed_calls.load(Ordering::Relaxed), 1);
    let refused = next(&mut adapter).await;
    assert_eq!(refused["type"], "error");
    assert_eq!(refused["id"], 108);
    adapter
        .request(request(
            serde_json::json!({"type":"input","id":999,"session":"a","data":"eA=="}),
        ))
        .await
        .unwrap();
    assert_eq!(next(&mut adapter).await["id"], 999);
    assert_eq!(fake.input.lock().unwrap()[0].1, b"x");
    adapter.streams[0]
        .queue
        .lock()
        .unwrap()
        .push(TerminalFrame::Output {
            seq: 11,
            bytes: b"live output".to_vec(),
        });
    assert_eq!(next(&mut adapter).await["type"], "output");
    assert_eq!(fake.feed_calls.load(Ordering::Relaxed), 1);
    release.send(()).unwrap();
    for id in 100..108 {
        let reply = next(&mut adapter).await;
        assert_eq!(reply["id"], id);
        assert_eq!(reply["result"]["before"], id.to_string());
    }
    assert_eq!(fake.feed_calls.load(Ordering::Relaxed), 8);
    assert!(adapter.requests.is_empty() && adapter.request_work.is_none());
}

#[tokio::test]
async fn credit_stalled_state_pushes_coalesce_at_the_tail_without_dropping_replies() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    let mut overview = fake.overview.lock().unwrap().clone();
    overview.tabs[0].name = "x".repeat(300 * 1024);
    adapter
        .push(HostMessage::Overview(overview.clone()))
        .unwrap();
    let in_flight = adapter.next(false).await.unwrap().unwrap();
    adapter.in_flight(in_flight.len());
    adapter
        .push(HostMessage::Result(ResultMessage {
            id: 7,
            result: serde_json::Value::Null,
        }))
        .unwrap();
    for index in 0..20 {
        overview.tabs[0].name = format!("{index}{}", "x".repeat(300 * 1024));
        adapter
            .push(HostMessage::Overview(overview.clone()))
            .unwrap();
    }
    let mut status = overview.sessions.values().next().unwrap().status.clone();
    adapter
        .push(HostMessage::Status(StatusPush {
            session: "a".into(),
            status: status.clone(),
        }))
        .unwrap();
    adapter
        .push(HostMessage::FeedAppended(FeedAppended {
            mission: "m".into(),
            entries: vec![],
        }))
        .unwrap();
    status.unread = !status.unread;
    adapter
        .push(HostMessage::Status(StatusPush {
            session: "a".into(),
            status: status.clone(),
        }))
        .unwrap();
    assert_eq!(adapter.in_flight, in_flight.len());
    assert_eq!(next(&mut adapter).await["id"], 7);
    assert!(next(&mut adapter).await["tabs"][0]["name"]
        .as_str()
        .unwrap()
        .starts_with("19"));
    assert_eq!(next(&mut adapter).await["type"], "feedAppended");
    assert_eq!(next(&mut adapter).await["unread"], status.unread);
    assert!(adapter.next(false).await.unwrap().is_none());
    assert_eq!(adapter.queued, 0);
}
#[tokio::test]
async fn terminal_chunks_order_fairness_overflow_and_resync_sequence() {
    let fake = Arc::new(Fake::default());
    *fake.bytes.lock().unwrap() = vec![b'x'; TERMINAL_CHUNK_SIZE + 1];
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    attach(&mut adapter, "a").await;
    attach(&mut adapter, "b").await;
    let a = next(&mut adapter).await;
    let b = next(&mut adapter).await;
    assert_eq!(a["session"], "a");
    assert_eq!(b["session"], "b");
    assert_eq!(a["more"], true);
    let a2 = next(&mut adapter).await;
    let b2 = next(&mut adapter).await;
    assert_eq!(a2["more"], false);
    assert_eq!(a["seq"], a2["seq"]);
    assert_eq!(b["seq"], b2["seq"]);
    assert_eq!(next(&mut adapter).await["type"], "resized");
    assert_eq!(next(&mut adapter).await["type"], "resized");
    quiet(&mut adapter);
    for stream in &adapter.streams {
        stream.queue.lock().unwrap().push(TerminalFrame::Output {
            seq: 50,
            bytes: vec![b'y'; TERMINAL_CHUNK_SIZE + 1],
        });
    }
    let a3 = next(&mut adapter).await;
    let b3 = next(&mut adapter).await;
    assert_eq!(a3["session"], "a");
    assert_eq!(b3["session"], "b");
    let a4 = next(&mut adapter).await;
    let b4 = next(&mut adapter).await;
    assert!(a4["seq"].as_u64().unwrap() > a3["seq"].as_u64().unwrap());
    assert!(b4["seq"].as_u64().unwrap() > b3["seq"].as_u64().unwrap());
    let stream = adapter.streams.front().unwrap();
    assert!(!stream.queue.lock().unwrap().push(TerminalFrame::Output {
        seq: 100,
        bytes: vec![0; MAX_MESSAGE_SIZE + 1]
    }));
    assert!(adapter.next(false).await.unwrap().is_none());
    let fresh = next(&mut adapter).await;
    assert_eq!(fresh["type"], "snapshot");
    assert!(fresh["seq"].as_u64().unwrap() > a4["seq"].as_u64().unwrap());
    assert_eq!(*fake.history.lock().unwrap(), [2, 2, 2]);
}
// Behaviors of the output merge and window, with time paused: a message that arrives from `sent`
// went without time passing, and one from `delivered` needed the timed wake in `changed`.
async fn streaming(fake: &Arc<Fake>, sessions: &[&str]) -> Adapter {
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    for session in sessions {
        attach(&mut adapter, session).await;
    }
    for _ in sessions {
        assert_eq!(next(&mut adapter).await["type"], "snapshot");
    }
    for _ in sessions {
        assert_eq!(next(&mut adapter).await["type"], "resized");
    }
    adapter
}
fn stream_queue(adapter: &Adapter, session: &str) -> Arc<Mutex<Queue>> {
    let stream = adapter.streams.iter().find(|s| s.session == session);
    stream.unwrap().queue.clone()
}
fn push_output(adapter: &Adapter, session: &str, bytes: &[u8]) -> bool {
    stream_queue(adapter, session)
        .lock()
        .unwrap()
        .push(TerminalFrame::Output {
            seq: 0,
            bytes: bytes.to_vec(),
        })
}
fn push_resize(adapter: &Adapter, session: &str) {
    stream_queue(adapter, session)
        .lock()
        .unwrap()
        .push(TerminalFrame::Resized {
            seq: 0,
            cols: 100,
            rows: 40,
            controller: TerminalController::Desktop,
        });
}
async fn sent(adapter: &mut Adapter) -> Option<serde_json::Value> {
    let bytes = adapter.next(true).await.unwrap()?;
    Some(serde_json::from_slice(&bytes).unwrap())
}
async fn delivered(adapter: &mut Adapter) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(1), next(adapter))
        .await
        .expect("held output must come due with no further frame or event")
}
fn seq(message: &serde_json::Value) -> u64 {
    message["seq"].as_u64().unwrap()
}
fn data(message: &serde_json::Value) -> Vec<u8> {
    assert_eq!(message["type"], "output");
    serde_json::from_value::<TerminalBytes>(message["data"].clone())
        .unwrap()
        .0
}
// A first output, so what is pushed next falls inside its window.
async fn open_window(adapter: &mut Adapter, session: &str) -> u64 {
    push_output(adapter, session, b"first");
    let first = sent(adapter).await.expect("a first output goes at once");
    assert_eq!(data(&first), b"first");
    seq(&first)
}

#[tokio::test(start_paused = true)]
async fn output_after_a_quiet_period_goes_without_time_passing() {
    let fake = Arc::new(Fake::default());
    let mut adapter = streaming(&fake, &["a"]).await;
    let first = open_window(&mut adapter, "a").await;
    tokio::time::advance(OUTPUT_WINDOW).await;
    push_output(&adapter, "a", b"typed");
    let typed = sent(&mut adapter).await.expect("a quiet window has passed");
    assert_eq!(data(&typed), b"typed");
    assert_eq!(seq(&typed), first + 1);
}

#[tokio::test(start_paused = true)]
async fn frames_within_the_window_leave_as_one_output() {
    let fake = Arc::new(Fake::default());
    let mut adapter = streaming(&fake, &["a"]).await;
    let first = open_window(&mut adapter, "a").await;
    let start = Instant::now();
    push_output(&adapter, "a", b"one ");
    assert!(sent(&mut adapter).await.is_none());
    tokio::time::advance(Duration::from_millis(2)).await;
    push_output(&adapter, "a", b"two ");
    push_output(&adapter, "a", b"three");
    assert!(sent(&mut adapter).await.is_none());
    let merged = delivered(&mut adapter).await;
    assert_eq!(data(&merged), b"one two three");
    assert_eq!(seq(&merged), first + 1);
    assert!(start.elapsed() >= OUTPUT_WINDOW);
    assert_eq!(stream_queue(&adapter, "a").lock().unwrap().bytes, 0);
}

#[tokio::test(start_paused = true)]
async fn a_merge_stops_at_the_chunk_size_and_keeps_the_rest_queued() {
    let fake = Arc::new(Fake::default());
    let mut adapter = streaming(&fake, &["a"]).await;
    let first = open_window(&mut adapter, "a").await;
    let frames: Vec<Vec<u8>> = (0..4).map(|i| vec![b'a' + i; 10 * 1024]).collect();
    for frame in &frames {
        push_output(&adapter, "a", frame);
    }
    let full = sent(&mut adapter)
        .await
        .expect("what cannot grow does not wait");
    assert_eq!(data(&full), frames[..3].concat());
    assert_eq!(seq(&full), first + 1);
    assert_eq!(
        stream_queue(&adapter, "a").lock().unwrap().bytes,
        frames[3].len() + 64
    );
    assert!(sent(&mut adapter).await.is_none());
    let fill = vec![b'z'; TERMINAL_CHUNK_SIZE - frames[3].len()];
    push_output(&adapter, "a", &fill);
    let exact = sent(&mut adapter).await.expect("an exact fit is full");
    assert_eq!(data(&exact), [&frames[3][..], &fill].concat());
    assert_eq!(seq(&exact), first + 2);
    assert_eq!(stream_queue(&adapter, "a").lock().unwrap().bytes, 0);
}

#[tokio::test(start_paused = true)]
async fn an_oversized_frame_splits_without_waiting_between_its_chunks() {
    let fake = Arc::new(Fake::default());
    let mut adapter = streaming(&fake, &["a"]).await;
    let mut seen = open_window(&mut adapter, "a").await;
    let big: Vec<u8> = (0..TERMINAL_CHUNK_SIZE * 2 + 1)
        .map(|i| (i % 251) as u8)
        .collect();
    push_output(&adapter, "a", &big);
    push_output(&adapter, "a", b"after");
    let mut received = Vec::new();
    for len in [TERMINAL_CHUNK_SIZE, TERMINAL_CHUNK_SIZE, 1] {
        let piece = sent(&mut adapter).await.expect("a split never waits");
        assert_eq!(data(&piece).len(), len);
        assert_eq!(seq(&piece), seen + 1);
        seen = seq(&piece);
        received.extend(data(&piece));
    }
    assert_eq!(received, big);
    assert!(sent(&mut adapter).await.is_none());
    assert_eq!(data(&delivered(&mut adapter).await), b"after");
}

#[tokio::test(start_paused = true)]
async fn a_resize_ends_the_merge_and_is_not_delayed() {
    let fake = Arc::new(Fake::default());
    let mut adapter = streaming(&fake, &["a"]).await;
    let first = open_window(&mut adapter, "a").await;
    push_output(&adapter, "a", b"held");
    assert!(sent(&mut adapter).await.is_none());
    push_output(&adapter, "a", b" before");
    push_resize(&adapter, "a");
    push_output(&adapter, "a", b"after");
    let before = sent(&mut adapter)
        .await
        .expect("output ahead of a resize goes");
    assert_eq!(data(&before), b"held before");
    assert_eq!(seq(&before), first + 1);
    let resized = sent(&mut adapter).await.expect("a resize never waits");
    assert_eq!(resized["type"], "resized");
    assert_eq!(seq(&resized), first + 2);
    assert!(sent(&mut adapter).await.is_none());
    let after = delivered(&mut adapter).await;
    assert_eq!(data(&after), b"after");
    assert_eq!(seq(&after), first + 3);
}

#[tokio::test(start_paused = true)]
async fn held_output_is_delivered_by_changed_alone() {
    let fake = Arc::new(Fake::default());
    let mut adapter = streaming(&fake, &["a"]).await;
    open_window(&mut adapter, "a").await;
    let start = Instant::now();
    push_output(&adapter, "a", b"held");
    assert!(sent(&mut adapter).await.is_none());
    assert_eq!(adapter.output_due, Some(start + OUTPUT_WINDOW));
    tokio::time::timeout(Duration::from_secs(1), adapter.changed())
        .await
        .expect("the timed wake needs no other cause")
        .unwrap();
    assert!(start.elapsed() >= OUTPUT_WINDOW);
    assert_eq!(adapter.output_due, None);
    assert_eq!(data(&sent(&mut adapter).await.unwrap()), b"held");
}

#[tokio::test(start_paused = true)]
async fn an_ended_stream_flushes_held_output_then_closes() {
    let fake = Arc::new(Fake::default());
    let mut adapter = streaming(&fake, &["a"]).await;
    open_window(&mut adapter, "a").await;
    push_output(&adapter, "a", b"last ");
    assert!(sent(&mut adapter).await.is_none());
    push_output(&adapter, "a", b"words");
    stream_queue(&adapter, "a").lock().unwrap().ended = true;
    let last = sent(&mut adapter).await.expect("an end does not wait");
    assert_eq!(data(&last), b"last words");
    let closed = sent(&mut adapter).await.expect("closed follows the output");
    assert_eq!(closed["type"], "closed");
    assert!(adapter.streams.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_flood_never_waits_on_the_window_and_only_a_real_overflow_resyncs() {
    let fake = Arc::new(Fake::default());
    let mut adapter = streaming(&fake, &["a"]).await;
    let mut last = open_window(&mut adapter, "a").await;
    let start = Instant::now();
    let frame = vec![b'x'; 10 * 1024];
    let (mut pushed, mut received) = (0, 0);
    while pushed < MAX_MESSAGE_SIZE * 3 {
        assert!(
            push_output(&adapter, "a", &frame),
            "the flood overflowed the queue"
        );
        pushed += frame.len();
        while let Some(message) = sent(&mut adapter).await {
            assert_eq!(seq(&message), last + 1);
            last = seq(&message);
            received += data(&message).len();
        }
    }
    assert_eq!(start.elapsed(), Duration::ZERO);
    assert!(pushed - received < TERMINAL_CHUNK_SIZE);
    let tail = delivered(&mut adapter).await;
    assert_eq!(seq(&tail), last + 1);
    last = seq(&tail);
    assert_eq!(received + data(&tail).len(), pushed);
    assert_eq!(fake.snapshots.load(Ordering::Relaxed), 1);

    assert!(!push_output(&adapter, "a", &vec![0; MAX_MESSAGE_SIZE + 1]));
    let snapshot = sent(&mut adapter).await.expect("an overflow does not wait");
    assert_eq!(snapshot["type"], "snapshot");
    assert!(seq(&snapshot) > last);
    assert_eq!(sent(&mut adapter).await.unwrap()["type"], "resized");
    push_output(&adapter, "a", b"fresh");
    let fresh = sent(&mut adapter)
        .await
        .expect("a new stream has no window");
    assert_eq!(data(&fresh), b"fresh");
    assert_eq!(seq(&fresh), seq(&snapshot) + 1);
}

#[tokio::test(start_paused = true)]
async fn a_held_stream_does_not_hold_back_other_streams_or_feeds() {
    let fake = Arc::new(Fake::default());
    fake.feed_enabled.store(true, Ordering::Relaxed);
    let mut adapter = streaming(&fake, &["a", "b"]).await;
    open_window(&mut adapter, "a").await;
    push_output(&adapter, "a", b"held");
    push_output(&adapter, "b", b"other");
    let other = sent(&mut adapter).await.expect("b has no window open");
    assert_eq!(other["session"], "b");
    assert_eq!(data(&other), b"other");
    fake.appended.lock().unwrap().push(FeedEntry {
        id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
        ts: chrono::Utc::now().to_rfc3339(),
        from: "coder".into(),
        to: vec![],
        content: FeedContent::Message {
            text: "during a hold".into(),
        },
    });
    adapter.feeds.insert(
        "m".into(),
        FeedSubscription {
            cursor: 0,
            dirty: true,
        },
    );
    let feed = delivered(&mut adapter).await;
    assert_eq!(feed["type"], "feedAppended");
    assert!(sent(&mut adapter).await.is_none());
    let held = delivered(&mut adapter).await;
    assert_eq!(held["session"], "a");
    assert_eq!(data(&held), b"held");
}

#[tokio::test(start_paused = true)]
async fn output_queued_without_credit_merges_when_credit_returns() {
    let fake = Arc::new(Fake::default());
    let mut adapter = streaming(&fake, &["a"]).await;
    let first = open_window(&mut adapter, "a").await;
    for frame in [&b"one "[..], b"two ", b"three"] {
        push_output(&adapter, "a", frame);
        assert!(adapter.next(false).await.unwrap().is_none());
        tokio::time::advance(OUTPUT_WINDOW).await;
    }
    let merged = sent(&mut adapter).await.expect("the window has passed");
    assert_eq!(data(&merged), b"one two three");
    assert_eq!(seq(&merged), first + 1);
}

#[tokio::test(start_paused = true)]
async fn a_due_window_without_credit_wakes_once_and_does_not_spin() {
    let fake = Arc::new(Fake::default());
    let mut adapter = streaming(&fake, &["a"]).await;
    open_window(&mut adapter, "a").await;
    push_output(&adapter, "a", b"held");
    assert!(sent(&mut adapter).await.is_none());
    assert!(adapter.next(false).await.unwrap().is_none());
    tokio::time::timeout(Duration::from_secs(1), adapter.changed())
        .await
        .expect("the window comes due once")
        .unwrap();
    assert!(adapter.next(false).await.unwrap().is_none());
    assert!(
        tokio::time::timeout(OUTPUT_WINDOW * 10, adapter.changed())
            .await
            .is_err(),
        "a wake that cannot send would spin the channel loop"
    );
    assert_eq!(data(&sent(&mut adapter).await.unwrap()), b"held");
}

#[test]
fn input_classification_and_queue_bound() {
    assert!(
        matches!(classify_input(b"\x1b[200~a\nb\x1b[201~"), InputEvent::Paste { text } if text == "a\nb")
    );
    assert!(matches!(
        classify_input(b"\r"),
        InputEvent::Key {
            kind: InputKind::Submit
        }
    ));
    assert!(matches!(
        classify_input("hello 世界".as_bytes()),
        InputEvent::Key {
            kind: InputKind::Content { .. }
        }
    ));
    for bytes in [&b"\x1b[A"[..], &b"\x03"[..], &b"\xff"[..]] {
        assert!(matches!(
            classify_input(bytes),
            InputEvent::Key {
                kind: InputKind::Navigate
            }
        ));
    }
    let mut queue = Queue::default();
    assert!(queue.push(TerminalFrame::Output {
        seq: 1,
        bytes: vec![0; MAX_MESSAGE_SIZE - 64]
    }));
    assert!(!queue.push(TerminalFrame::Resized {
        seq: 2,
        cols: 80,
        rows: 24,
        controller: TerminalController::Desktop
    }));
    assert!(queue.resync);
    assert!(queue.frames.is_empty());
}

#[tokio::test]
async fn empty_snapshot_input_resize_and_close_preserve_per_terminal_order() {
    let fake = Arc::new(Fake::default());
    fake.bytes.lock().unwrap().clear();
    let mut adapter = Adapter::new(fake.clone(), 8, String::new()).unwrap();
    attach(&mut adapter, "a").await;
    let empty = next(&mut adapter).await;
    assert_eq!(empty["data"], "");
    assert_eq!(empty["more"], false);
    assert_eq!(next(&mut adapter).await["controller"]["kind"], "desktop");
    adapter
        .request(request(
            serde_json::json!({"type":"input","id":2,"session":"a","data":"DQ=="}),
        ))
        .await
        .unwrap();
    assert_eq!(next(&mut adapter).await["id"], 2);
    assert!(matches!(
        fake.input.lock().unwrap()[0].2,
        InputEvent::Key {
            kind: InputKind::Submit
        }
    ));
    {
        let stream = adapter.streams.front().unwrap();
        let mut queue = stream.queue.lock().unwrap();
        queue.push(TerminalFrame::Output {
            seq: 11,
            bytes: b"before resize".to_vec(),
        });
        queue.push(TerminalFrame::Resized {
            seq: 12,
            cols: 100,
            rows: 40,
            controller: TerminalController::Browser {
                channel: 8,
                name: "same".into(),
            },
        });
        queue.push(TerminalFrame::Output {
            seq: 13,
            bytes: b"after resize".to_vec(),
        });
        queue.push(TerminalFrame::Resized {
            seq: 14,
            cols: 90,
            rows: 30,
            controller: TerminalController::Browser {
                channel: 9,
                name: "other".into(),
            },
        });
        queue.ended = true;
    }
    let before = next(&mut adapter).await;
    let resized = next(&mut adapter).await;
    let after = next(&mut adapter).await;
    let other = next(&mut adapter).await;
    let closed = next(&mut adapter).await;
    assert_eq!(before["type"], "output");
    assert_eq!(resized["controller"]["kind"], "you");
    assert_eq!(after["type"], "output");
    assert_eq!(other["controller"]["name"], "other");
    assert_eq!(closed["reason"], "exited");
    assert!(adapter.streams.is_empty());
    adapter
        .request(request(
            serde_json::json!({"type":"input","id":3,"session":"a","data":"DQ=="}),
        ))
        .await
        .unwrap();
    assert_eq!(next(&mut adapter).await["code"], "badRequest");
}

#[tokio::test]
async fn results_precede_terminal_chunks_and_resync_keeps_the_requested_history() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    attach(&mut adapter, "a").await;
    adapter
        .request(request(serde_json::json!({"type":"overview","id":2})))
        .await
        .unwrap();
    while adapter.overview_work.is_some() {
        adapter.changed().await.unwrap();
    }
    assert_eq!(next(&mut adapter).await["id"], 2);
    let first = next(&mut adapter).await;
    next(&mut adapter).await;
    adapter
        .streams
        .front()
        .unwrap()
        .queue
        .lock()
        .unwrap()
        .push(TerminalFrame::Resync);
    let fresh = next(&mut adapter).await;
    assert_eq!(fresh["type"], "snapshot");
    assert!(fresh["seq"].as_u64().unwrap() > first["seq"].as_u64().unwrap());
    assert_eq!(*fake.history.lock().unwrap(), [2, 2]);
}

#[tokio::test]
async fn a_slow_core_request_holds_up_only_its_channel() {
    let fake = Arc::new(Fake::default());
    let (release, wait) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    *fake.overview_gate.lock().unwrap() = Some(wait);
    *fake.overview_started.lock().unwrap() = Some(started);
    let mut slow = Adapter::new(fake.clone(), 1, "slow".into()).unwrap();
    let mut other = Adapter::new(fake, 2, "other".into()).unwrap();
    let pending = tokio::spawn(async move {
        slow.request(request(serde_json::json!({"type":"overview","id":1})))
            .await
            .unwrap();
        slow
    });
    ready.await.unwrap();
    other
        .request(request(serde_json::json!({"type":"overview","id":2})))
        .await
        .unwrap();
    assert_eq!(next(&mut other).await["id"], 2);
    release.send(()).unwrap();
    let mut slow = pending.await.unwrap();
    assert_eq!(next(&mut slow).await["id"], 1);
}

#[tokio::test(start_paused = true)]
async fn an_event_append_invalidates_pending_asks_in_the_overview() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    adapter.initialize().await.unwrap();
    fake.overview.lock().unwrap().missions[0].asks.clear();
    fake.events
        .emit("event/appended", &serde_json::json!({"mission_id":"test"}));
    adapter.changed().await.unwrap();
    adapter.changed().await.unwrap();
    assert_eq!(
        next(&mut adapter).await["missions"][0]["asks"],
        serde_json::json!([])
    );
}

#[tokio::test]
async fn a_stopped_session_still_closes_after_its_exit_event_was_lost() {
    let fake = Arc::new(Fake::default());
    let session = fake
        .overview
        .lock()
        .unwrap()
        .sessions
        .keys()
        .next()
        .unwrap()
        .clone();
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    adapter.initialize().await.unwrap();
    attach(&mut adapter, &session).await;
    next(&mut adapter).await;
    next(&mut adapter).await;
    fake.overview
        .lock()
        .unwrap()
        .sessions
        .get_mut(&session)
        .unwrap()
        .status
        .status = Status::Stopped;
    fake.events.emit("chat/tab-attention-changed", &());
    adapter.changed().await.unwrap();
    next_stopped(&mut adapter, &session).await;
    wait_for_end(&adapter).await;
    assert_eq!(next_non_overview(&mut adapter).await["type"], "closed");
}

async fn wait_for_end(adapter: &Adapter) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !adapter.streams.front().unwrap().queue.lock().unwrap().ended {
            adapter.ready.notified().await;
        }
    })
    .await
    .unwrap();
}

struct HeldFrame {
    entered: Option<tokio::sync::oneshot::Sender<()>>,
    release: std::sync::mpsc::Receiver<()>,
    cancelled: Arc<AtomicBool>,
}
impl TerminalSubscription for HeldFrame {
    fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
            self.release.recv().unwrap();
            return Ok(TerminalFrame::Output {
                seq: 11,
                bytes: b"final output".to_vec(),
            });
        }
        Err(ClientError::msg("closed"))
    }
    fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
        let cancelled = self.cancelled.clone();
        Arc::new(move || {
            cancelled.store(true, Ordering::SeqCst);
        })
    }
}

#[tokio::test]
async fn exit_archive_and_status_recovery_drain_a_held_frame_before_closing() {
    for change in ["exit", "archive", "status"] {
        let fake = Arc::new(Fake::default());
        let session = fake
            .overview
            .lock()
            .unwrap()
            .sessions
            .keys()
            .next()
            .unwrap()
            .clone();
        fake.overview
            .lock()
            .unwrap()
            .sessions
            .get_mut(&session)
            .unwrap()
            .status
            .status = Status::Working;
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        *fake.frames.lock().unwrap() = Some(Box::new(HeldFrame {
            entered: Some(entered),
            release: held,
            cancelled: cancelled.clone(),
        }));
        let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
        adapter.initialize().await.unwrap();
        attach(&mut adapter, &session).await;
        assert_eq!(next(&mut adapter).await["type"], "snapshot");
        assert_eq!(next(&mut adapter).await["type"], "resized");
        ready.await.unwrap();
        match change {
            "exit" => fake
                .events
                .emit("session/exit", &serde_json::json!({"session_id":session})),
            "archive" => {
                *fake.closed_reason.lock().unwrap() = ClosedReason::Archived;
                fake.overview.lock().unwrap().sessions.remove(&session);
                fake.events.emit(
                    "session/archived",
                    &serde_json::json!({"session_id":session}),
                );
            }
            _ => {
                fake.overview
                    .lock()
                    .unwrap()
                    .sessions
                    .get_mut(&session)
                    .unwrap()
                    .status
                    .status = Status::Stopped;
                fake.events.emit("chat/tab-attention-changed", &());
            }
        }
        adapter.changed().await.unwrap();
        if change == "status" {
            next_stopped(&mut adapter, &session).await;
        }
        assert!(cancelled.load(Ordering::SeqCst));
        let mut premature = adapter.next(true).await.unwrap();
        while premature.as_ref().is_some_and(|bytes| {
            serde_json::from_slice::<serde_json::Value>(bytes).unwrap()["type"] == "overview"
        }) {
            premature = adapter.next(true).await.unwrap();
        }
        release.send(()).unwrap();
        assert!(
            premature.is_none(),
            "{change} closed before its held output was forwarded"
        );
        wait_for_end(&adapter).await;
        let output = next_non_overview(&mut adapter).await;
        assert_eq!(output["type"], "output");
        let data: TerminalBytes = serde_json::from_value(output["data"].clone()).unwrap();
        assert_eq!(data.0, b"final output");
        let closed = next_non_overview(&mut adapter).await;
        assert_eq!(closed["type"], "closed");
        assert_eq!(
            closed["reason"],
            if change == "archive" {
                "archived"
            } else {
                "exited"
            }
        );
    }
}

pub(crate) fn with_output_held(adapter: &Adapter, produce: impl FnOnce()) {
    let _queue = adapter.streams[0].queue.lock().unwrap();
    produce();
}

pub(crate) async fn wait_for_buffered(adapter: &Adapter, frames: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let buffered = {
                let queue = adapter.streams[0].queue.lock().unwrap();
                queue.resync || queue.frames.len() >= frames
            };
            if buffered {
                break;
            }
            adapter.ready.notified().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn snapshot_continuation_survives_chunking_attach_and_resync() {
    let fake = Arc::new(Fake::default());
    let mut bytes = vec![b'x'; TERMINAL_CHUNK_SIZE - 2];
    bytes.extend_from_slice(b"\x1b[3");
    *fake.bytes.lock().unwrap() = bytes.clone();
    *fake.continuation.lock().unwrap() = (Some('c'), 3);
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    attach(&mut adapter, "rep").await;
    let mut previous = 0;
    for resync in [false, true] {
        if resync {
            adapter.streams[0]
                .queue
                .lock()
                .unwrap()
                .push(TerminalFrame::Resync);
        }
        let mut body = Vec::new();
        let mut seq = None;
        loop {
            let HostMessage::Snapshot(snapshot) =
                serde_json::from_value(next(&mut adapter).await).unwrap()
            else {
                panic!()
            };
            assert_eq!(snapshot.preceding_char, Some('c'));
            assert_eq!(snapshot.unfinished_len, 3);
            assert_eq!(*seq.get_or_insert(snapshot.seq), snapshot.seq);
            body.extend_from_slice(&snapshot.data.0);
            if !snapshot.more {
                break;
            }
        }
        assert_eq!(body, bytes);
        assert!(seq.unwrap() > previous);
        previous = seq.unwrap();
        let resized = next(&mut adapter).await;
        assert_eq!(resized["type"], "resized");
        assert_eq!(resized["seq"], previous);
    }
}

#[tokio::test]
async fn a_ping_is_answered_with_its_id_while_overview_and_feed_work_runs() {
    let fake = Arc::new(Fake::default());
    fake.feed_enabled.store(true, Ordering::Relaxed);
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    let (release_overview, gate) = std::sync::mpsc::channel();
    let (started, overview_ready) = tokio::sync::oneshot::channel();
    *fake.overview_gate.lock().unwrap() = Some(gate);
    *fake.overview_started.lock().unwrap() = Some(started);
    adapter
        .request(request(serde_json::json!({"type":"overview","id":1})))
        .await
        .unwrap();
    overview_ready.await.unwrap();
    let (release_feed, gate) = std::sync::mpsc::channel();
    let (started, feed_ready) = tokio::sync::oneshot::channel();
    *fake.feed_gate.lock().unwrap() = Some(gate);
    *fake.feed_started.lock().unwrap() = Some(started);
    adapter
        .request(request(
            serde_json::json!({"type":"feed","id":2,"mission":"m","before":null,"limit":10}),
        ))
        .await
        .unwrap();
    feed_ready.await.unwrap();

    adapter
        .request(request(serde_json::json!({"type":"ping","id":7})))
        .await
        .unwrap();
    assert_eq!(
        next(&mut adapter).await,
        serde_json::json!({"type":"pong","id":7})
    );

    release_overview.send(()).unwrap();
    release_feed.send(()).unwrap();
    let mut replies = BTreeMap::new();
    while replies.len() < 2 {
        let message = next(&mut adapter).await;
        replies.insert(message["id"].as_u64().unwrap(), message["type"].clone());
    }
    assert_eq!(replies[&1], "result");
    assert_eq!(replies[&2], "result");
}

fn feed_entry(id: &str) -> FeedEntry {
    FeedEntry {
        id: id.into(),
        ts: "2026-11-09T00:00:00Z".into(),
        from: "human".into(),
        to: vec![],
        content: FeedContent::Message { text: id.into() },
    }
}
/// A followed mission whose first feed read has started and is held by the returned gate.
async fn held_feed_read(fake: &Arc<Fake>, adapter: &mut Adapter) -> std::sync::mpsc::Sender<()> {
    fake.requests_enabled.store(true, Ordering::Relaxed);
    let (release, gate) = std::sync::mpsc::channel();
    let (started, read_started) = tokio::sync::oneshot::channel();
    *fake.feed_since_gate.lock().unwrap() = Some(gate);
    *fake.feed_since_started.lock().unwrap() = Some(started);
    fake.appended.lock().unwrap().push(feed_entry("first"));
    adapter
        .request(request(
            serde_json::json!({"type":"follow","id":1,"mission":"m"}),
        ))
        .await
        .unwrap();
    assert_eq!(delivered(adapter).await["id"], 1);
    tokio::time::timeout(Duration::from_secs(1), async {
        while adapter.feed_read.is_none() {
            if adapter.next(true).await.unwrap().is_none() && adapter.feed_read.is_none() {
                adapter.changed().await.unwrap();
            }
        }
    })
    .await
    .expect("the followed mission's first read must start");
    read_started.await.unwrap();
    release
}

#[tokio::test]
async fn a_ping_is_answered_while_a_followed_feed_read_is_held() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    let release = held_feed_read(&fake, &mut adapter).await;

    adapter
        .request(request(serde_json::json!({"type":"ping","id":7})))
        .await
        .unwrap();
    assert_eq!(
        delivered(&mut adapter).await,
        serde_json::json!({"type":"pong","id":7})
    );
    assert!(adapter.feed_read.is_some(), "the read is still held");

    release.send(()).unwrap();
    let batch = delivered(&mut adapter).await;
    assert_eq!(batch["type"], "feedAppended");
    assert_eq!(batch["entries"][0]["id"], "first");
}

#[tokio::test]
async fn an_append_during_a_feed_read_is_read_after_it() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    let release = held_feed_read(&fake, &mut adapter).await;

    fake.appended.lock().unwrap().push(feed_entry("second"));
    fake.events
        .emit("event/appended", &serde_json::json!({"mission_id":"m"}));
    drain(&mut adapter).await;

    release.send(()).unwrap();
    for (expected, cursor) in [("first", 18), ("second", 19)] {
        let batch = loop {
            let message = delivered(&mut adapter).await;
            if message["type"] == "feedAppended" {
                break message;
            }
        };
        assert_eq!(batch["entries"][0]["id"], expected);
        assert_eq!(adapter.feeds["m"].cursor, cursor);
    }
}

#[tokio::test]
async fn unfollowing_during_a_feed_read_delivers_no_later_batch() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    let release = held_feed_read(&fake, &mut adapter).await;

    adapter
        .request(request(
            serde_json::json!({"type":"unfollow","id":2,"mission":"m"}),
        ))
        .await
        .unwrap();
    assert_eq!(delivered(&mut adapter).await["id"], 2);

    release.send(()).unwrap();
    while adapter.feed_read.is_some() {
        adapter.changed().await.unwrap();
    }
    assert!(adapter.feed_ready.is_none());
    assert!(adapter.feeds.is_empty());
    assert!(adapter.next(true).await.unwrap().is_none());
}

#[tokio::test]
async fn a_finished_batch_is_dropped_if_its_mission_is_unfollowed_before_sending() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    let release = held_feed_read(&fake, &mut adapter).await;
    release.send(()).unwrap();
    while adapter.feed_read.is_some() {
        adapter.changed().await.unwrap();
    }
    assert!(adapter.feed_ready.is_some());

    adapter
        .request(request(
            serde_json::json!({"type":"unfollow","id":2,"mission":"m"}),
        ))
        .await
        .unwrap();
    assert_eq!(delivered(&mut adapter).await["id"], 2);
    assert!(adapter.feed_ready.is_none());
    assert!(adapter.next(true).await.unwrap().is_none());
}

#[tokio::test]
async fn a_read_from_before_a_refollow_does_not_reach_the_new_subscription() {
    for cursor in [17, 40] {
        let fake = Arc::new(Fake::default());
        let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
        let release = held_feed_read(&fake, &mut adapter).await;

        adapter
            .request(request(
                serde_json::json!({"type":"unfollow","id":2,"mission":"m"}),
            ))
            .await
            .unwrap();
        assert_eq!(delivered(&mut adapter).await["id"], 2);
        adapter.feeds.insert(
            "m".into(),
            FeedSubscription {
                cursor,
                dirty: true,
            },
        );

        release.send(()).unwrap();
        while adapter.feed_read.is_some() {
            adapter.changed().await.unwrap();
        }
        assert!(adapter.feed_ready.is_none(), "cursor {cursor}");
        assert_eq!(adapter.feeds["m"].cursor, cursor);
        assert!(adapter.feeds["m"].dirty);
    }
}

#[tokio::test]
async fn a_failed_read_from_before_a_same_cursor_refollow_leaves_the_new_subscription_working() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    let release = held_feed_read(&fake, &mut adapter).await;
    let cursor = adapter.feeds["m"].cursor;
    fake.fail_feed_since.store(true, Ordering::Relaxed);

    for (id, kind) in [(2, "unfollow"), (3, "follow")] {
        adapter
            .request(request(
                serde_json::json!({"type":kind,"id":id,"mission":"m"}),
            ))
            .await
            .unwrap();
        let reply = delivered(&mut adapter).await;
        assert_eq!(
            (reply["id"].as_u64(), &reply["type"]),
            (Some(id), &"result".into())
        );
    }
    assert_eq!(adapter.feeds["m"].cursor, cursor);
    assert!(adapter.feed_read.is_some(), "the old read is still running");

    release.send(()).unwrap();
    while adapter.feed_read.is_some() {
        adapter.changed().await.unwrap();
    }
    assert!(
        adapter.feeds.contains_key("m"),
        "the new subscription stays"
    );

    fake.fail_feed_since.store(false, Ordering::Relaxed);
    fake.appended.lock().unwrap().push(feed_entry("after"));
    fake.events
        .emit("event/appended", &serde_json::json!({"mission_id":"m"}));
    let batch = loop {
        let message = delivered(&mut adapter).await;
        if message["type"] == "feedAppended" {
            break message;
        }
    };
    assert_eq!(batch["entries"][0]["id"], "after");
    assert_eq!(batch["mission"], "m");
}

#[tokio::test]
async fn a_feed_read_waits_for_room_in_the_reply_queue() {
    let fake = Arc::new(Fake::default());
    fake.feed_enabled.store(true, Ordering::Relaxed);
    fake.appended.lock().unwrap().push(feed_entry("first"));
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    adapter.feeds.insert(
        "m".into(),
        FeedSubscription {
            cursor: 0,
            dirty: true,
        },
    );
    adapter.in_flight(MAX_MESSAGE_SIZE - 10);
    assert!(adapter.next(true).await.unwrap().is_none());
    assert!(adapter.feed_read.is_none());
    adapter.in_flight(0);
    assert_eq!(delivered(&mut adapter).await["type"], "feedAppended");
}

#[tokio::test]
async fn a_ping_never_fails_the_channel_however_full_its_reply_queue_is() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake, 1, String::new()).unwrap();
    adapter.in_flight(MAX_MESSAGE_SIZE);
    for id in [9, 10] {
        adapter
            .request(request(serde_json::json!({"type":"ping","id":id})))
            .await
            .unwrap();
    }
    assert_eq!(
        next(&mut adapter).await,
        serde_json::json!({"type":"pong","id":10})
    );
}

#[test]
fn every_action_and_served_feed_request_is_advertised() {
    for name in ActionRequest::NAMES
        .iter()
        .copied()
        .chain(["feed", "follow", "unfollow", "answer", "post", "ping"])
    {
        assert!(FEATURES.contains(&name), "{name}");
    }
}

async fn drain(adapter: &mut Adapter) {
    while !adapter.events.is_empty() {
        adapter.changed().await.unwrap();
    }
}
#[tokio::test]
async fn clipboard_writes_reach_attached_channels_in_bounded_pieces_up_to_the_cap() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    attach(&mut adapter, "a").await;
    next(&mut adapter).await;
    next(&mut adapter).await;
    quiet(&mut adapter);
    let write = |session: &str, text: &str| {
        fake.events.emit(
            "session/clipboard",
            &serde_json::json!({"session_id":session,"target":"selection","text":text}),
        );
    };
    let cap = runner_terminal::terminal::MAX_CLIPBOARD_BYTES;
    let escaped = "\n\\\"\u{1}é"
        .repeat(cap)
        .chars()
        .take(cap / 2)
        .collect::<String>();
    let escaped = format!("{escaped}{}", "\n".repeat(cap - escaped.len()));
    for text in ["x".repeat(cap), escaped] {
        assert_eq!(text.len(), cap);
        write("other", "unattached");
        drain(&mut adapter).await;
        write("a", &text);
        drain(&mut adapter).await;
        let mut received = String::new();
        loop {
            let bytes = adapter.next(true).await.unwrap().expect("clipboard piece");
            assert!(bytes.len() <= TERMINAL_CHUNK_SIZE * 7);
            let piece: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(piece["type"], "clipboard");
            assert_eq!(piece["session"], "a");
            assert_eq!(piece["target"], "selection");
            assert_eq!(piece["offset"], received.len() as u64);
            received.push_str(piece["text"].as_str().unwrap());
            if piece["more"] == false {
                break;
            }
        }
        assert!(received == text);
        assert!(adapter.next(true).await.unwrap().is_none());
    }

    write("a", &"y".repeat(TERMINAL_CHUNK_SIZE * 2));
    drain(&mut adapter).await;
    let first: serde_json::Value =
        serde_json::from_slice(&adapter.next(true).await.unwrap().unwrap()).unwrap();
    assert_eq!(first["more"], true);
    write("a", "newer");
    drain(&mut adapter).await;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&adapter.next(true).await.unwrap().unwrap())
            .unwrap(),
        serde_json::json!({"type":"clipboard","session":"a","target":"selection","text":"newer","offset":0,"more":false})
    );
    assert!(adapter.next(true).await.unwrap().is_none());
    assert!(
        adapter.overview_due.is_none(),
        "a write is not an overview change"
    );
}

#[tokio::test]
async fn clipboard_and_selection_writes_both_complete_once() {
    let fake = Arc::new(Fake::default());
    let mut adapter = Adapter::new(fake.clone(), 1, String::new()).unwrap();
    attach(&mut adapter, "a").await;
    next(&mut adapter).await;
    next(&mut adapter).await;
    quiet(&mut adapter);
    let write = |target: &str, text: &str| {
        fake.events.emit(
            "session/clipboard",
            &serde_json::json!({"session_id":"a","target":target,"text":text}),
        );
    };
    let piece = |bytes: Vec<u8>| serde_json::from_slice::<serde_json::Value>(&bytes).unwrap();
    let large = "c".repeat(TERMINAL_CHUNK_SIZE * 3);
    for selection_after_first_piece in [false, true] {
        write("clipboard", &large);
        let mut pieces = Vec::new();
        if selection_after_first_piece {
            drain(&mut adapter).await;
            pieces.push(piece(adapter.next(true).await.unwrap().unwrap()));
            assert_eq!(pieces[0]["more"], true);
        }
        write("selection", "primary");
        drain(&mut adapter).await;
        while let Some(bytes) = adapter.next(true).await.unwrap() {
            pieces.push(piece(bytes));
        }
        let mut whole = Vec::new();
        let mut text = String::new();
        for piece in pieces {
            assert_eq!(piece["offset"], text.len() as u64);
            text.push_str(piece["text"].as_str().unwrap());
            if piece["more"] == false {
                whole.push((
                    piece["target"].as_str().unwrap().to_owned(),
                    std::mem::take(&mut text),
                ));
            }
        }
        assert_eq!(
            whole,
            [
                ("clipboard".to_owned(), large.clone()),
                ("selection".to_owned(), "primary".to_owned())
            ]
        );
    }

    write("clipboard", "one");
    write("selection", "two");
    write("clipboard", "three");
    drain(&mut adapter).await;
    let mut whole = Vec::new();
    while let Some(bytes) = adapter.next(true).await.unwrap() {
        let piece = piece(bytes);
        assert_eq!(piece["more"], false);
        whole.push((
            piece["target"].as_str().unwrap().to_owned(),
            piece["text"].as_str().unwrap().to_owned(),
        ));
    }
    assert_eq!(
        whole,
        [
            ("selection".to_owned(), "two".to_owned()),
            ("clipboard".to_owned(), "three".to_owned())
        ],
        "the latest write lands last, where both targets share one clipboard"
    );
}
