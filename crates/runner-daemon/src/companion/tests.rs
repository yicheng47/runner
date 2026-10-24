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
    overview_gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    overview_started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
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
            overview_gate: Default::default(),
            overview_started: Default::default(),
            bytes: Mutex::new(b"screen".to_vec()),
            continuation: Mutex::new((None, 0)),
            frames: Default::default(),
            closed_reason: Mutex::new(ClosedReason::Exited),
        }
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
    fn overview(&self) -> core::Result<Overview> {
        let gate = self.overview_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            if let Some(started) = self.overview_started.lock().unwrap().take() {
                let _ = started.send(());
            }
            gate.recv().unwrap();
        }
        Ok(self.overview.lock().unwrap().clone())
    }
    fn subscribe(&self) -> core::Result<broadcast::Receiver<AppEvent>> {
        Ok(self.events.subscribe())
    }
    fn attach(self: Arc<Self>, session: &str, history: usize) -> core::Result<core::Attachment> {
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
            resync: Arc::new(move || self.clone().attach(&session, history).map(|a| a.terminal)),
        })
    }
    fn resize(&self, _: &str, _: u64, _: &str, _: (u16, u16), _: Control) -> core::Result<()> {
        Ok(())
    }
    fn input(&self, session: &str, bytes: &[u8]) -> core::Result<()> {
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
    serde_json::from_slice(&adapter.next(true).await.unwrap().unwrap()).unwrap()
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
    assert_eq!(next(&mut adapter).await["type"], "status");
    wait_for_end(&adapter).await;
    assert_eq!(next(&mut adapter).await["type"], "closed");
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
            assert_eq!(next(&mut adapter).await["type"], "status");
        }
        assert!(cancelled.load(Ordering::SeqCst));
        let premature = adapter.next(true).await.unwrap();
        release.send(()).unwrap();
        assert!(
            premature.is_none(),
            "{change} closed before its held output was forwarded"
        );
        wait_for_end(&adapter).await;
        let output = next(&mut adapter).await;
        assert_eq!(output["type"], "output");
        let data: TerminalBytes = serde_json::from_value(output["data"].clone()).unwrap();
        assert_eq!(data.0, b"final output");
        let closed = next(&mut adapter).await;
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

#[test]
fn every_action_and_served_feed_request_is_advertised() {
    for name in ActionRequest::NAMES
        .iter()
        .copied()
        .chain(["feed", "follow", "unfollow", "answer", "post"])
    {
        assert!(FEATURES.contains(&name), "{name}");
    }
}
