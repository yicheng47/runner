use super::*;

#[derive(Default)]
struct CountingStore {
    memory: MemoryStore,
    reads: AtomicU32,
    unavailable: AtomicBool,
}
impl SecretStore for CountingStore {
    fn get(&self, name: &str) -> super::super::super::store::StoreResult<Option<Vec<u8>>> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        if self.unavailable.load(Ordering::Acquire) {
            return Err("secret store unavailable");
        }
        self.memory.get(name)
    }
    fn put(&self, name: &str, value: &[u8]) -> super::super::super::store::StoreResult<()> {
        self.memory.put(name, value)
    }
    fn delete(&self, name: &str) -> super::super::super::store::StoreResult<()> {
        self.memory.delete(name)
    }
}

#[tokio::test]
async fn live_route_needs_no_secrets_or_directory_even_when_both_fail() {
    let store = Arc::new(CountingStore::default());
    let fixture = Fixture::with_store(store.clone()).await;
    fixture.pair().await;
    fixture.client.overview("2".into()).await.unwrap();
    let reads = store.reads.load(Ordering::Relaxed);
    let opens = fixture.relay.opens.load(Ordering::Relaxed);
    store.unavailable.store(true, Ordering::Release);
    fixture.relay.directory_error.store(true, Ordering::Release);
    *fixture.client.0.directory.lock().await = None;
    assert!(fixture
        .client
        .computers()
        .await
        .unwrap_err()
        .contains("resource_exhausted"));
    let listings = fixture.relay.listings.load(Ordering::Relaxed);
    fixture.client.overview("2".into()).await.unwrap();
    let terminal = fixture.client.attach(terminal_config()).await.unwrap();
    fixture
        .client
        .viewport("2".into(), "session".into(), 90, 30, true)
        .await
        .unwrap();
    let request = c::ClientMessage::decode(
        br#"{"type":"rename","id":1,"target":{"kind":"session","id":"session"},"name":"new"}"#,
    )
    .unwrap();
    fixture.client.request("2".into(), request).await.unwrap();
    assert_eq!(store.reads.load(Ordering::Relaxed), reads);
    assert_eq!(fixture.relay.listings.load(Ordering::Relaxed), listings);
    assert_eq!(fixture.relay.opens.load(Ordering::Relaxed), opens);
    drop(terminal);
}

#[tokio::test]
async fn credentials_drop_on_each_identity_change_and_keys_follow_writes() {
    for change in ["origin", "host", "signout"] {
        let fixture = Fixture::new().await;
        fixture.pair().await;
        fixture.client.overview("2".into()).await.unwrap();
        let route = fixture.client.route("2").await.unwrap();
        assert!(fixture.client.0.cached.lock().await.context.is_some());
        fixture.status.send_modify(|status| match change {
            "origin" => status.origin = "https://other.invalid".into(),
            "host" => status.host_id = Some("other".into()),
            _ => status.state = RemoteState::Off,
        });
        tokio::time::timeout(Duration::from_secs(2), route.cancel.cancelled())
            .await
            .unwrap();
        let cached = fixture.client.0.cached.lock().await;
        assert!(cached.context.is_none());
        assert!(cached.keys.is_none());
    }
    let fixture = Fixture::new().await;
    fixture.pair().await;
    assert_eq!(fixture.client.keys().await.unwrap().0.len(), 1);
    fixture.client.forget("2".into()).await.unwrap();
    assert!(fixture.client.keys().await.unwrap().0.is_empty());
    fixture.pair().await;
    fixture.client.reset().await.unwrap();
    assert!(fixture.client.keys().await.unwrap().0.is_empty());
}

#[tokio::test(start_paused = true)]
async fn silent_routes_time_out_at_fifteen_seconds_with_and_without_host_ping() {
    for ping in [true, false] {
        let fixture = if ping {
            Fixture::new().await
        } else {
            Fixture::without_ping().await
        };
        fixture.pair().await;
        fixture.client.follow("2".into(), 1, true).await.unwrap();
        fixture.client.overview("2".into()).await.unwrap();
        let mut events = fixture.client.0.events.subscribe();
        let route = fixture.client.route("2").await.unwrap();
        let generation = route.generation();
        fixture
            .relay
            .writer
            .block_reads
            .store(true, Ordering::Release);
        // Drain work already read before the network disappears.
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        let start = Instant::now();
        tokio::time::advance(Duration::from_millis(14_999)).await;
        tokio::task::yield_now().await;
        assert!(route.ready.load(Ordering::Acquire));
        tokio::time::advance(Duration::from_millis(1)).await;
        loop {
            let event = events.recv().await.unwrap();
            if event.name == "remote/connection" && event.payload["state"] == "reconnecting" {
                assert_eq!(event.payload["error"], "Remote route timed out");
                break;
            }
        }
        assert_eq!(start.elapsed(), Duration::from_secs(15));
        assert!(!route.ready.load(Ordering::Acquire));
        fixture
            .relay
            .writer
            .block_reads
            .store(false, Ordering::Release);
        fixture.relay.writer.reader.wake();
        loop {
            let event = events.recv().await.unwrap();
            if event.name == "remote/connection" && event.payload["state"] == "connected" {
                break;
            }
        }
        assert!(route.generation() > generation);
        fixture.client.overview("2".into()).await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn older_host_stays_alive_through_relay_pongs_without_companion_latency() {
    let fixture = Fixture::without_ping().await;
    fixture.pair().await;
    fixture.client.follow("2".into(), 1, true).await.unwrap();
    fixture.client.overview("2".into()).await.unwrap();
    let route = fixture.client.route("2").await.unwrap();
    let generation = route.generation();
    for _ in 0..8 {
        tokio::time::advance(Duration::from_secs(5)).await;
        // Give the automatic WebSocket pong a full reader/writer turn.
        for _ in 0..30 {
            tokio::task::yield_now().await;
        }
        assert!(route.ready.load(Ordering::Acquire));
        assert_eq!(route.generation(), generation);
    }
}

#[tokio::test]
async fn stalled_writes_keep_more_than_thirty_two_inputs_in_order_and_report_echo() {
    let fixture = Fixture::new().await;
    let mut events = fixture.client.0.events.subscribe();
    fixture.pair().await;
    let attachment = fixture.client.attach(terminal_config()).await.unwrap();
    let route = fixture.client.route("2").await.unwrap();
    // Settle the initial ping before checking that an echo alone emits no event.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if events.recv().await.unwrap().name == "remote/latency" {
                break;
            }
        }
    })
    .await
    .expect("initial ping was not answered");
    let writer = &fixture.relay.writer;
    writer.blocked.store(true, Ordering::Release);
    for byte in 0..100u8 {
        route
            .input(route.generation(), "session".into(), vec![byte])
            .await
            .unwrap();
    }
    writer.entered.notified().await;
    assert!(fixture.core.fake.input.lock().unwrap().is_empty());
    // Reads continue while the writer is stalled.
    fixture.core.streams.lock().unwrap()[0]
        .send(TerminalFrame::Output {
            seq: 1,
            bytes: b"before".to_vec(),
        })
        .unwrap();
    let attachment = receive_terminal(attachment).await.0;
    assert!(route.ready.load(Ordering::Acquire));
    while let Ok(event) = events.try_recv() {
        if event.name == "remote/latency" {
            assert!(event.payload["echo_ms"].is_null());
        }
    }
    writer.blocked.store(false, Ordering::Release);
    writer.waker.wake();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if fixture.core.fake.input.lock().unwrap().len() == 100 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let bytes: Vec<_> = fixture
        .core
        .fake
        .input
        .lock()
        .unwrap()
        .iter()
        .flat_map(|(_, bytes, _)| bytes.clone())
        .collect();
    assert_eq!(bytes, (0..100u8).collect::<Vec<_>>());
    assert_eq!(fixture.relay.opens.load(Ordering::Relaxed), 1);
    fixture.core.streams.lock().unwrap()[0]
        .send(TerminalFrame::Output {
            seq: 2,
            bytes: b"echo".to_vec(),
        })
        .unwrap();
    let attachment = receive_terminal(attachment).await.0;
    while let Ok(event) = events.try_recv() {
        assert_ne!(event.name, "remote/latency");
    }
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(5)).await;
    loop {
        let event = events.recv().await.unwrap();
        if event.name == "remote/latency" {
            assert!(event.payload["ms"].is_u64());
            assert!(event.payload["echo_ms"].is_u64());
            break;
        }
    }
    tokio::time::resume();
    drop(attachment);
}

#[tokio::test]
async fn buffered_input_at_the_cap_is_discarded_on_reconnect_with_scoped_errors() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let attachment = fixture.client.attach(terminal_config()).await.unwrap();
    let route = fixture.client.route("2").await.unwrap();
    let mut events = fixture.client.0.events.subscribe();
    fixture.relay.writer.blocked.store(true, Ordering::Release);
    for _ in 0..c::MAX_MESSAGE_SIZE / (64 * 1024) {
        route
            .input(route.generation(), "session".into(), vec![b'x'; 64 * 1024])
            .await
            .unwrap();
    }
    fixture.relay.writer.entered.notified().await;
    assert!(route
        .input(route.generation(), "session".into(), vec![b'y'])
        .await
        .unwrap_err()
        .contains("queue is full"));
    // Losing this route discards both the writer's frames and input still waiting in the loop.
    fixture.drop_route().await;
    loop {
        let event = events.recv().await.unwrap();
        if event.name == "session/input-error" {
            assert_eq!(
                event.payload["session_id"],
                remote::terminal_address("2", "session")
            );
            break;
        }
    }
    fixture.relay.writer.blocked.store(false, Ordering::Release);
    fixture.relay.writer.waker.wake();
    fixture.client.overview("2".into()).await.unwrap();
    assert!(fixture.core.fake.input.lock().unwrap().is_empty());
    drop(attachment);
}

#[tokio::test]
async fn a_slow_action_times_out_without_reconnecting_and_its_late_reply_is_ignored() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture.client.follow("2".into(), 1, true).await.unwrap();
    fixture.client.overview("2".into()).await.unwrap();
    let route = fixture.client.route("2").await.unwrap();
    let generation = route.generation();
    let (entered, mut ready) = mpsc::unbounded_channel();
    let (release, gate) = std::sync::mpsc::channel();
    *fixture.core.action_gate.lock().unwrap() = Some(ActionGate {
        fork_only: false,
        entered,
        release: gate,
    });
    let client = fixture.client.clone();
    let action = tokio::spawn(async move {
        let request = c::ClientMessage::decode(
            br#"{"type":"rename","id":1,"target":{"kind":"session","id":"session"},"name":"new"}"#,
        )
        .unwrap();
        client.request("2".into(), request).await
    });
    ready.recv().await.unwrap();
    tokio::time::pause();
    keep_alive(&fixture, 35).await;
    assert_eq!(
        action.await.unwrap().unwrap_err(),
        "Remote request timed out"
    );
    assert!(route.ready.load(Ordering::Acquire));
    assert_eq!(route.generation(), generation);
    let mut events = fixture.client.0.events.subscribe();
    release.send(()).unwrap();
    tokio::time::resume();
    fixture.client.overview("2".into()).await.unwrap();
    while let Ok(event) = events.try_recv() {
        assert_ne!(event.name, "remote/connection");
    }
    assert_eq!(fixture.relay.opens.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn forget_ignores_a_cancelled_old_identity_beside_the_live_route() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture.client.overview("2".into()).await.unwrap();
    let live = fixture.client.route("2").await.unwrap();
    let old_key = remote::HostKey::generate().unwrap();
    let old_fingerprint = remote::fingerprint(&old_key.public);
    let mut key = fixture.client.keys().await.unwrap().0[&live.fingerprint].clone();
    key.public = old_key.public;
    fixture
        .client
        .update_key(old_fingerprint.clone(), Some(key))
        .await
        .unwrap();
    let mut stale = live.clone();
    stale.fingerprint = old_fingerprint.clone();
    stale.cancel = CancellationToken::new();
    stale.cancel.cancel();
    {
        let mut routes = fixture.client.0.routes.lock().await;
        // Exercise the iteration order that used to choose the dead identity.
        for _ in 0..100 {
            *routes = HashMap::from([
                (old_fingerprint.clone(), stale.clone()),
                (live.fingerprint.clone(), live.clone()),
            ]);
            if routes.values().next().unwrap().cancel.is_cancelled() {
                break;
            }
        }
        assert!(routes.values().next().unwrap().cancel.is_cancelled());
    }
    fixture.client.forget("2".into()).await.unwrap();
    assert!(live.cancel.is_cancelled());
    let keys = fixture.client.keys().await.unwrap();
    assert!(!keys.0.contains_key(&live.fingerprint));
    assert!(keys.0.contains_key(&old_fingerprint));
    let routes = fixture.client.0.routes.lock().await;
    assert!(!routes.contains_key(&live.fingerprint));
    assert!(routes.contains_key(&old_fingerprint));
}

#[tokio::test]
async fn a_two_hundred_kib_ipc_paste_arrives_whole_and_in_order_on_a_healthy_route() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let attachment = fixture.client.attach(terminal_config()).await.unwrap();
    let route = fixture.client.route("2").await.unwrap();
    let generation = route.generation();
    let (_root, _server, ipc) = ipc_fixture(&fixture).await;
    let mut events = fixture.client.0.events.subscribe();
    let body: Vec<_> = (0..200 * 1024).map(|i| b'a' + (i % 26) as u8).collect();
    let paste = [b"\x1b[200~".as_slice(), &body, b"\x1b[201~"].concat();
    ipc.input(&remote::terminal_address("2", "session"), &paste)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let received: usize = fixture
                .core
                .fake
                .input
                .lock()
                .unwrap()
                .iter()
                .map(|(_, bytes, _)| bytes.len())
                .sum();
            if received == paste.len() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let inputs = fixture.core.fake.input.lock().unwrap();
    assert!(inputs.len() >= 4);
    let received: Vec<_> = inputs
        .iter()
        .flat_map(|(_, bytes, _)| bytes.iter().copied())
        .collect();
    assert_eq!(received, paste);
    drop(inputs);
    while let Ok(event) = events.try_recv() {
        assert_ne!(event.name, "session/input-error");
    }
    assert!(route.ready.load(Ordering::Acquire));
    assert_eq!(route.generation(), generation);
    assert_eq!(fixture.relay.opens.load(Ordering::Relaxed), 1);
    drop(attachment);
}
