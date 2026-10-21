use super::super::trust::Trust;
use super::super::{
    machine::{Command, Effect, Input},
    store::Identity,
};
use super::*;
use channel::{BrowserConnect, BrowserPair};
use runner_core::remote::Secret;
use serde_json::Value;

struct Relay {
    routes: Option<Channels>,
    writes: Arc<TrustWrites>,
    companion: Arc<std::sync::OnceLock<Arc<dyn crate::companion::core::Core>>>,
    machine: Machine,
    root: tempfile::TempDir,
    events: crate::events::EventChannel,
}
impl Relay {
    fn new() -> Self {
        Self::with_core(Arc::new(crate::companion::tests::Fake::default()))
    }
    fn with_core(core: Arc<dyn crate::companion::core::Core>) -> Self {
        let companion = Arc::new(std::sync::OnceLock::new());
        assert!(companion.set(core).is_ok());
        let mut identity = Identity::generate().unwrap();
        identity.pin = Some(Secret("012345".into()));
        let mut machine = Machine::default();
        machine.identity = Some(identity);
        machine.status.state = remote::RemoteState::Connected {
            since: chrono::Utc::now(),
        };
        machine.saved = Some(super::super::persistence::Saved {
            version: 1,
            origin: "https://relay.invalid".into(),
            host_id: "test".into(),
            relay_url: "wss://relay.invalid/relay/host".into(),
            account: remote::RemoteAccount::default(),
            host_name: "test-host".into(),
            fingerprint: String::new(),
            activated: true,
            claim_expires_at: chrono::Utc::now(),
            paused: false,
            stopped: None,
        });
        machine.host = remote::HostInfo::normalized("test-host", "macos", "arm64", "test", "test");
        let writes = Arc::new(TrustWrites::default());
        Self {
            routes: Some(Channels::new(writes.clone(), companion.clone())),
            writes,
            companion,
            machine,
            root: tempfile::tempdir().unwrap(),
            events: crate::events::EventChannel::new(),
        }
    }
    fn open(&mut self, id: u32) {
        self.routes
            .as_mut()
            .unwrap()
            .open(id, &self.machine)
            .unwrap();
    }
    fn feed(&mut self, id: u32, bytes: &[u8]) {
        assert!(self.routes.as_mut().unwrap().feed(id, bytes));
    }
    async fn output(&mut self) -> Message {
        loop {
            let writes = self.writes.clone();
            let routes = self.routes.as_mut().unwrap();
            let out = tokio::select! {
                event = routes.next() => routes.handle(event, &mut self.machine, self.root.path(), &Clock::default(), &self.events),
                completion = writes.next() => {
                    writes.complete(completion, Some(routes), &mut self.machine, self.root.path(), &self.events);
                    None
                }
            };
            routes.sync(&mut self.machine);
            if let Some(out) = out {
                return out;
            }
        }
    }
    async fn frame(&mut self, id: u32) -> Vec<u8> {
        match self.output().await {
            Message::Binary(bytes) => {
                let (channel, frame) = channel::unwrap(&bytes).unwrap();
                assert_eq!(channel, id);
                frame.to_vec()
            }
            _ => panic!("expected binary frame"),
        }
    }
    async fn pair_start(&mut self, id: u32, pin: &str) -> BrowserPair {
        self.open(id);
        let (browser, pair) = BrowserPair::start(
            NoiseKeys {
                private: &[id as u8; 32],
                fixed_ephemeral: None,
            },
            pin,
            [id as u8; 16],
            [9; 32],
        )
        .unwrap();
        self.feed(id, &pair);
        browser
    }
    async fn pair_finish(&mut self, id: u32, pin: &str) -> (Records, Vec<u8>) {
        let browser = self.pair_start(id, pin).await;
        let reply = self.frame(id).await;
        let (transport, finish, host) = browser.finish(&reply, "test browser").unwrap();
        self.feed(id, &finish);
        (Records::new(transport), host)
    }
    async fn hello(&mut self, id: u32, browser: &mut Records) -> Value {
        browser
            .queue(br#"{"type":"hello","versions":[1],"client":"test","build":"test"}"#.to_vec())
            .unwrap();
        self.flush(id, browser);
        self.message(id, browser).await
    }
    fn flush(&mut self, id: u32, browser: &mut Records) {
        while let Some(frame) = browser.next_frame().unwrap() {
            self.feed(id, &frame);
        }
    }
    async fn message(&mut self, id: u32, browser: &mut Records) -> Value {
        loop {
            let frame = self.frame(id).await;
            if let Some(bytes) = browser.receive(&frame).unwrap() {
                browser.handled();
                return serde_json::from_slice(&bytes).unwrap();
            }
        }
    }
    fn command(&mut self, command: Command) -> Vec<Message> {
        let revoke = if let Command::RevokeDevice(f) = &command {
            Some(f.clone())
        } else {
            None
        };
        let (effects, _) = self
            .machine
            .step(Input::Command(command), chrono::Utc::now(), &mut || 0)
            .unwrap();
        transition(
            &mut self.routes,
            &mut self.machine,
            &effects,
            revoke.as_deref(),
        )
    }
}

#[tokio::test]
async fn pairs_connects_welcomes_and_refuses_requests_over_the_relay() {
    let mut relay = Relay::new();
    let mut events = relay.events.subscribe();
    let (mut browser, host) = relay.pair_finish(1, "012345").await;
    let welcome = relay.hello(1, &mut browser).await;
    assert_eq!(welcome["type"], "welcome");
    assert_eq!(
        welcome["features"],
        serde_json::json!(crate::companion::FEATURES)
    );
    assert_eq!(relay.machine.trust.devices.len(), 1);
    assert!(relay.machine.trust.devices[0].connected);
    assert_eq!(events.try_recv().unwrap().name, "remote/paired");
    assert!(events.try_recv().is_err());
    assert!(!Trust::load(relay.root.path()).unwrap().devices[0].connected);
    browser
        .queue(br#"{"type":"overview","id":7}"#.to_vec())
        .unwrap();
    relay.flush(1, &mut browser);
    let error = relay.message(1, &mut browser).await;
    assert_eq!(error["type"], "result");
    assert_eq!(error["id"], 7);
    let (connecting, connect) = BrowserConnect::start(
        NoiseKeys {
            private: &[1; 32],
            fixed_ephemeral: None,
        },
        &host,
    )
    .unwrap();
    relay.open(2);
    relay.feed(2, &connect);
    let accept = relay.frame(2).await;
    let mut browser2 = Records::new(connecting.accept(&accept).unwrap());
    assert_eq!(relay.hello(2, &mut browser2).await["type"], "welcome");
    let fingerprint = relay.machine.trust.devices[0].fingerprint.clone();
    let closed = relay.command(Command::RevokeDevice(fingerprint));
    assert_eq!(closed.len(), 2);
    assert!(relay.routes.as_ref().unwrap().entries.is_empty());
    relay.open(3);
    relay.feed(3, &connect);
    assert_eq!(relay.frame(3).await, Reject::UnknownDevice.frame());
}

#[tokio::test]
async fn parallel_confirmations_lock_at_five_and_unlock_allows_pairing() {
    let mut relay = Relay::new();
    let mut finishes = Vec::new();
    for id in 1..=6 {
        let browser = relay.pair_start(id, "999999").await;
        let reply = relay.frame(id).await;
        let (_, finish, _) = browser.finish(&reply, "wrong").unwrap();
        finishes.push((id, finish));
    }
    for (id, finish) in finishes {
        relay.feed(id, &finish);
    }
    let mut wrong = 0;
    let mut locked = 0;
    let mut closed = 0;
    while wrong + locked < 6 || closed < 6 {
        match relay.output().await {
            Message::Binary(bytes) => {
                let (_, frame) = channel::unwrap(&bytes).unwrap();
                if frame == Reject::WrongPin.frame() {
                    wrong += 1;
                } else {
                    assert_eq!(frame, Reject::PinLocked.frame());
                    locked += 1;
                }
            }
            Message::Text(_) => closed += 1,
            _ => panic!("unexpected relay output"),
        }
    }
    assert_eq!((wrong, locked), (4, 2));
    assert_eq!(relay.machine.trust.failed_pins, 5);
    assert!(relay.machine.trust.pin_locked);
    assert_eq!(Trust::load(relay.root.path()).unwrap().failed_pins, 5);
    relay.command(Command::UnlockPin);
    let (mut browser, _) = relay.pair_finish(7, "012345").await;
    assert_eq!(relay.hello(7, &mut browser).await["type"], "welcome");
    assert_eq!(relay.machine.trust.failed_pins, 0);
}

#[tokio::test]
async fn reject_reasons_malformed_frames_and_tampering() {
    for reason in [
        Reject::NoPin,
        Reject::PinLocked,
        Reject::BadHandshake,
        Reject::WrongPin,
    ] {
        let mut relay = Relay::new();
        match reason {
            Reject::NoPin => relay.machine.identity.as_mut().unwrap().pin = None,
            Reject::PinLocked => relay.machine.trust.pin_locked = true,
            _ => {}
        }
        let b = relay.pair_start(1, "012345").await;
        if matches!(reason, Reject::NoPin | Reject::PinLocked) {
            assert_eq!(relay.frame(1).await, reason.frame());
            continue;
        }
        let reply = relay.frame(1).await;
        let (_, mut finish, _) = b.finish(&reply, "test").unwrap();
        if reason == Reject::BadHandshake {
            finish.truncate(60);
        } else {
            *finish.last_mut().unwrap() ^= 1;
        }
        relay.feed(1, &finish);
        assert_eq!(relay.frame(1).await, reason.frame());
        assert_eq!(
            relay.machine.trust.failed_pins,
            u8::from(reason == Reject::WrongPin)
        );
    }
    let mut relay = Relay::new();
    for id in 1..=16 {
        relay.open(id);
    }
    assert_eq!(
        relay.routes.as_mut().unwrap().open(17, &relay.machine),
        Err(Reject::Busy)
    );
    assert!(relay.routes.as_mut().unwrap().feed(999, &[6; 20]));
}

#[tokio::test(start_paused = true)]
async fn deadline_uses_a_fake_clock_and_channel_close_is_isolated() {
    let mut relay = Relay::new();
    relay.open(1);
    relay.open(2);
    tokio::time::advance(Duration::from_secs(30)).await;
    let mut count = 0;
    while !relay.routes.as_ref().unwrap().entries.is_empty() {
        match relay.output().await {
            Message::Binary(bytes) => {
                let (_, frame) = channel::unwrap(&bytes).unwrap();
                assert_eq!(frame, Reject::BadHandshake.frame());
                count += 1;
            }
            Message::Text(_) => {}
            _ => panic!("unexpected"),
        }
    }
    assert_eq!(count, 2);
    assert!(relay.machine.identity.is_some());
}

#[tokio::test]
async fn pause_disconnect_reset_and_route_loss_close_all_channels() {
    for command in [
        Command::Pause(true),
        Command::Logout,
        Command::ResetIdentity(Identity::generate().unwrap()),
    ] {
        let mut relay = Relay::new();
        let (mut b, _) = relay.pair_finish(1, "012345").await;
        relay.hello(1, &mut b).await;
        relay.command(command);
        assert!(relay.routes.is_none());
        assert!(relay.machine.trust.devices.iter().all(|d| !d.connected));
    }
    let mut relay = Relay::new();
    relay.open(1);
    relay.open(2);
    transition(
        &mut relay.routes,
        &mut relay.machine,
        &[Effect::Close],
        None,
    );
    assert!(relay.routes.is_none());
}

fn raw_frame(transport: &mut Transport, record: &[u8]) -> Vec<u8> {
    let mut out = vec![0; record.len() + 16];
    transport.write_message(record, &mut out).unwrap();
    channel::frame(channel::Kind::Data, &out)
}

#[tokio::test]
async fn invalid_records_and_failed_persistence_end_only_their_channel() {
    let mut relay = Relay::new();
    let (mut browser, _) = relay.pair_finish(1, "012345").await;
    relay.hello(1, &mut browser).await;
    relay.feed(1, &[6; 20]);
    assert!(matches!(relay.output().await, Message::Text(_)));
    assert!(relay.machine.identity.is_some());
    assert_eq!(relay.machine.trust.failed_pins, 0);
    let (mut browser, _) = relay.pair_finish(2, "012345").await;
    assert_eq!(relay.hello(2, &mut browser).await["type"], "welcome");
    let mut relay = Relay::new();
    let mut events = relay.events.subscribe();
    std::fs::create_dir(relay.root.path().join("remote-trust.json")).unwrap();
    let browser = relay.pair_start(1, "012345").await;
    let reply = relay.frame(1).await;
    let (_, finish, _) = browser.finish(&reply, "test").unwrap();
    relay.feed(1, &finish);
    assert_eq!(relay.frame(1).await, Reject::BadHandshake.frame());
    assert!(relay.machine.trust.devices.is_empty());
    assert!(!relay.machine.trust.pin_locked);
    assert_eq!(relay.machine.trust.failed_pins, 0);
    assert!(relay.machine.identity.is_some());
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn failed_pairing_persistence_survives_route_loss_and_blocks_provisional_ik() {
    let mut relay = Relay::new();
    let mut events = relay.events.subscribe();
    std::fs::create_dir(relay.root.path().join("remote-trust.json")).unwrap();
    let (_, host) = relay.pair_finish(1, "012345").await;
    let routes = relay.routes.as_mut().unwrap();
    let event = routes.next().await;
    assert!(matches!(event, Event::Handshake { .. }));
    routes.handle(
        event,
        &mut relay.machine,
        relay.root.path(),
        &Clock::default(),
        &relay.events,
    );
    transition(
        &mut relay.routes,
        &mut relay.machine,
        &[Effect::Close],
        None,
    );
    assert!(relay.routes.is_none());
    let completion = relay.writes.next().await;
    assert!(!completion.saved);
    std::fs::remove_dir(relay.root.path().join("remote-trust.json")).unwrap();
    relay.routes = Some(Channels::new(relay.writes.clone(), relay.companion.clone()));
    relay.open(1);
    let (_, connect) = BrowserConnect::start(
        NoiseKeys {
            private: &[1; 32],
            fixed_ephemeral: None,
        },
        &host,
    )
    .unwrap();
    relay.feed(1, &connect);
    assert_eq!(relay.frame(1).await, Reject::UnknownDevice.frame());
    relay.writes.complete(
        completion,
        relay.routes.as_mut(),
        &mut relay.machine,
        relay.root.path(),
        &relay.events,
    );
    assert!(relay.machine.trust.devices.is_empty());
    assert!(!relay.routes.as_ref().unwrap().entries[&1].ready);
    assert!(events.try_recv().is_err());
    relay
        .writes
        .save(relay.machine.trust.clone(), relay.root.path().to_path_buf())
        .await
        .unwrap();
    assert!(Trust::load(relay.root.path()).unwrap().devices.is_empty());
}

#[tokio::test]
async fn saturated_trust_writer_rejects_pairing_without_a_success_event() {
    let mut relay = Relay::new();
    let mut events = relay.events.subscribe();
    let _occupied = relay
        .writes
        .slots
        .clone()
        .acquire_many_owned(16)
        .await
        .unwrap();
    relay.pair_finish(1, "012345").await;
    assert_eq!(relay.frame(1).await, Reject::BadHandshake.frame());
    assert!(relay.machine.trust.devices.is_empty());
    assert!(events.try_recv().is_err());
    relay
        .writes
        .save(relay.machine.trust.clone(), relay.root.path().to_path_buf())
        .await
        .unwrap();
    assert!(Trust::load(relay.root.path()).unwrap().devices.is_empty());
}

#[tokio::test]
async fn persisted_pairing_after_route_loss_cannot_open_a_reused_channel_number() {
    let mut relay = Relay::new();
    let mut events = relay.events.subscribe();
    relay.pair_finish(1, "012345").await;
    let routes = relay.routes.as_mut().unwrap();
    let event = routes.next().await;
    routes.handle(
        event,
        &mut relay.machine,
        relay.root.path(),
        &Clock::default(),
        &relay.events,
    );
    let completion = relay.writes.next().await;
    assert!(completion.saved);
    let old_instance = completion.instance;
    transition(
        &mut relay.routes,
        &mut relay.machine,
        &[Effect::Close],
        None,
    );
    relay.routes = Some(Channels::new(relay.writes.clone(), relay.companion.clone()));
    relay.open(1);
    relay.writes.complete(
        completion,
        relay.routes.as_mut(),
        &mut relay.machine,
        relay.root.path(),
        &relay.events,
    );
    assert_eq!(events.try_recv().unwrap().name, "remote/paired");
    assert!(events.try_recv().is_err());
    let routes = relay.routes.as_mut().unwrap();
    assert!(!routes.entries[&1].ready);
    for event in [
        Event::Send {
            channel: 1,
            instance: old_instance,
            frame: vec![6; 20],
        },
        Event::Finished {
            channel: 1,
            instance: old_instance,
            reason: "closed",
        },
    ] {
        assert!(routes
            .handle(
                event,
                &mut relay.machine,
                relay.root.path(),
                &Clock::default(),
                &relay.events
            )
            .is_none());
    }
    assert!(routes.entries.contains_key(&1));
}

#[tokio::test]
async fn overlapping_pairings_of_one_device_preserve_the_successful_save_in_either_order() {
    for success_first in [false, true] {
        let mut relay = Relay::new();
        let mut events = relay.events.subscribe();
        std::fs::create_dir(relay.root.path().join("remote-trust.json")).unwrap();
        relay.pair_finish(1, "012345").await;
        let routes = relay.routes.as_mut().unwrap();
        let event = routes.next().await;
        routes.handle(
            event,
            &mut relay.machine,
            relay.root.path(),
            &Clock::default(),
            &relay.events,
        );
        let failed = relay.writes.next().await;
        assert!(!failed.saved);
        std::fs::remove_dir(relay.root.path().join("remote-trust.json")).unwrap();
        relay.open(2);
        let (browser, pair) = BrowserPair::start(
            NoiseKeys {
                private: &[1; 32],
                fixed_ephemeral: None,
            },
            "012345",
            [2; 16],
            [9; 32],
        )
        .unwrap();
        relay.feed(2, &pair);
        let reply = relay.frame(2).await;
        let (_, finish, _) = browser.finish(&reply, "same browser").unwrap();
        relay.feed(2, &finish);
        let routes = relay.routes.as_mut().unwrap();
        let event = routes.next().await;
        routes.handle(
            event,
            &mut relay.machine,
            relay.root.path(),
            &Clock::default(),
            &relay.events,
        );
        let successful = relay.writes.next().await;
        assert!(successful.saved);
        let completions = if success_first {
            [successful, failed]
        } else {
            [failed, successful]
        };
        for completion in completions {
            relay.writes.complete(
                completion,
                relay.routes.as_mut(),
                &mut relay.machine,
                relay.root.path(),
                &relay.events,
            );
        }
        assert_eq!(relay.machine.trust.devices.len(), 1);
        assert!(relay.writes.unconfirmed.lock().unwrap().is_empty());
        assert_eq!(events.try_recv().unwrap().name, "remote/paired");
        assert!(events.try_recv().is_err());
        assert_eq!(Trust::load(relay.root.path()).unwrap().devices.len(), 1);
    }
}

#[tokio::test]
async fn companion_requires_hello_and_negotiates_its_version() {
    let mut relay = Relay::new();
    let (mut b, _) = relay.pair_finish(1, "012345").await;
    b.queue(br#"{"type":"overview","id":1}"#.to_vec()).unwrap();
    relay.flush(1, &mut b);
    assert!(matches!(relay.output().await, Message::Text(_)));
    let (mut b, _) = relay.pair_finish(2, "012345").await;
    b.queue(br#"{"type":"hello","versions":[2],"client":"test","build":"test"}"#.to_vec())
        .unwrap();
    relay.flush(2, &mut b);
    assert_eq!(relay.message(2, &mut b).await["code"], "unsupportedVersion");
    assert!(matches!(relay.output().await, Message::Text(_)));
}

#[tokio::test]
async fn queue_overflow_and_an_empty_frame_close_only_the_addressed_channel() {
    let mut relay = Relay::new();
    relay.open(1);
    relay.open(2);
    let frame = vec![0; channel::MAX_FRAME];
    let mut overflow = false;
    for _ in 0..17 {
        if !relay.routes.as_mut().unwrap().feed(1, &frame) {
            overflow = true;
            break;
        }
    }
    assert!(overflow);
    assert!(!relay.routes.as_ref().unwrap().entries.contains_key(&1));
    assert!(relay.routes.as_ref().unwrap().entries.contains_key(&2));
    relay.feed(2, b"");
    assert_eq!(relay.frame(2).await, Reject::BadHandshake.frame());
    assert_eq!(relay.machine.trust.failed_pins, 0);
}

#[tokio::test]
async fn a_trust_write_does_not_hold_up_an_open_channel() {
    let mut relay = Relay::new();
    let (mut first, _) = relay.pair_finish(1, "012345").await;
    relay.hello(1, &mut first).await;
    let writer = relay.routes.as_ref().unwrap().writes.clone();
    let (held, ready) = oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let _guard = writer.saved.lock().unwrap();
        let _ = held.send(());
        let _ = wait.recv();
    });
    ready.await.unwrap();
    let (mut second, _) = relay.pair_finish(2, "012345").await;
    first
        .queue(br#"{"type":"overview","id":9}"#.to_vec())
        .unwrap();
    relay.flush(1, &mut first);
    assert_eq!(relay.message(1, &mut first).await["type"], "result");
    release.send(()).unwrap();
    thread.join().unwrap();
    assert_eq!(relay.hello(2, &mut second).await["type"], "welcome");
}

#[test]
fn a_delayed_trust_snapshot_cannot_restore_revoked_devices() {
    let root = tempfile::tempdir().unwrap();
    let writer = TrustWrites::default();
    let mut old = Trust::default();
    old.record_paired(&[1; 32], "test".into(), chrono::Utc::now())
        .unwrap();
    writer.persist(2, &Trust::default(), root.path()).unwrap();
    writer.persist(1, &old, root.path()).unwrap();
    assert!(Trust::load(root.path()).unwrap().devices.is_empty());
}

#[tokio::test]
async fn socket_loop_pairs_over_an_in_process_websocket_and_answers_heartbeats() {
    use super::super::{spawn_with_store, store::MemoryStore, transport};
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{tungstenite::protocol::Role, WebSocketStream};
    let root = tempfile::tempdir().unwrap();
    let handle = spawn_with_store(
        root.path().into(),
        Arc::new(MemoryStore::default()),
        crate::events::EventChannel::new(),
    );
    handle.set_companion_core(Arc::new(crate::companion::tests::Fake::default()));
    handle.set_pin(Secret("012345".into())).await.unwrap();
    let (host_io, peer_io) = tokio::io::duplex(remote::MAX_RELAY_SIZE * 2);
    let mut peer = WebSocketStream::from_raw_socket(peer_io, Role::Server, None).await;
    let host = transport::socket(
        Box::new(host_io),
        &remote::HostInfo::normalized("test", "macos", "arm64", "test", "test"),
    )
    .await
    .unwrap();
    let hello = peer.next().await.unwrap().unwrap();
    let hello: Value = serde_json::from_str(hello.to_text().unwrap()).unwrap();
    assert_eq!(hello["features"], serde_json::json!(["channels"]));
    handle
        .request(Command::TestSocket(transport::TestSocket(Box::new(host))))
        .await
        .unwrap();
    peer.send(Message::Text(
        r#"{"type":"channelOpen","channel":17}"#.into(),
    ))
    .await
    .unwrap();
    let (browser, pair) = BrowserPair::start(
        NoiseKeys {
            private: &[1; 32],
            fixed_ephemeral: None,
        },
        "012345",
        [2; 16],
        [3; 32],
    )
    .unwrap();
    peer.send(Message::Binary(channel::wrap(17, &pair).into()))
        .await
        .unwrap();
    let Message::Binary(reply) = peer.next().await.unwrap().unwrap() else {
        panic!("expected pair reply")
    };
    let (id, reply) = channel::unwrap(&reply).unwrap();
    assert_eq!(id, 17);
    let (browser, finish, _) = browser.finish(reply, "in-process browser").unwrap();
    let mut browser = Records::new(browser);
    peer.send(Message::Binary(channel::wrap(17, &finish).into()))
        .await
        .unwrap();
    browser
        .queue(br#"{"type":"hello","versions":[1],"client":"test","build":"test"}"#.to_vec())
        .unwrap();
    while let Some(frame) = browser.next_frame().unwrap() {
        peer.send(Message::Binary(channel::wrap(17, &frame).into()))
            .await
            .unwrap();
    }
    loop {
        let Message::Binary(bytes) = peer.next().await.unwrap().unwrap() else {
            panic!("expected encrypted record")
        };
        let (id, frame) = channel::unwrap(&bytes).unwrap();
        assert_eq!(id, 17);
        if let Some(bytes) = browser.receive(frame).unwrap() {
            let welcome: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(welcome["type"], "welcome");
            assert_eq!(
                welcome["features"],
                serde_json::json!(crate::companion::FEATURES)
            );
            break;
        }
    }
    assert!(handle.trust().devices[0].connected);
    peer.send(Message::Ping(vec![1, 2, 3].into()))
        .await
        .unwrap();
    assert_eq!(
        peer.next().await.unwrap().unwrap(),
        Message::Pong(vec![1, 2, 3].into())
    );
    peer.send(Message::Binary(channel::wrap(999, &[6; 20]).into()))
        .await
        .unwrap();
    let fingerprint = handle.trust().devices[0].fingerprint.clone();
    handle.revoke_device(fingerprint).await.unwrap();
    let close = peer.next().await.unwrap().unwrap();
    assert_eq!(
        close.to_text().unwrap(),
        r#"{"type":"closeChannel","channel":17}"#
    );
    assert!(handle.trust().devices.is_empty());
    handle.shutdown().await;
}
async fn raw_record(relay: &mut Relay, id: u32, browser: &mut Transport) -> Vec<u8> {
    let frame = relay.frame(id).await;
    let mut out = vec![0; frame.len()];
    let n = browser.read_message(&frame[1..], &mut out).unwrap();
    out.truncate(n);
    out
}

#[tokio::test]
async fn a_browser_without_credit_does_not_block_other_channels() {
    let mut relay = Relay::new();
    let browser = relay.pair_start(1, "012345").await;
    let reply = relay.frame(1).await;
    let (mut browser, finish, _) = browser.finish(&reply, "slow").unwrap();
    relay.feed(1, &finish);
    let hello = [
        &[1],
        br#"{"type":"hello","versions":[1],"client":"test","build":"test"}"#.as_slice(),
    ]
    .concat();
    relay.feed(1, &raw_frame(&mut browser, &hello));
    let mut spent = 0;
    loop {
        let record = raw_record(&mut relay, 1, &mut browser).await;
        if record[0] == 1 {
            spent += record.len() - 1;
            break;
        }
    }
    while spent < channel::INITIAL_CREDIT {
        relay.feed(
            1,
            &raw_frame(
                &mut browser,
                &[&[1], br#"{"type":"x","id":0}"#.as_slice()].concat(),
            ),
        );
        loop {
            let record = raw_record(&mut relay, 1, &mut browser).await;
            if record[0] != 2 {
                spent += record.len() - 1;
            }
            if record[0] == 1 || spent == channel::INITIAL_CREDIT {
                break;
            }
        }
    }
    let (mut second, _) = relay.pair_finish(2, "012345").await;
    assert_eq!(relay.hello(2, &mut second).await["type"], "welcome");
    let mut credit = vec![2];
    credit.extend_from_slice(&(spent as u32).to_be_bytes());
    relay.feed(1, &raw_frame(&mut browser, &credit));
    let record = raw_record(&mut relay, 1, &mut browser).await;
    assert_eq!(record[0], 1);
    assert!(record.len() > 1);
}

#[tokio::test]
async fn adapter_serves_overview_terminal_and_input_over_the_encrypted_channel() {
    let fake = Arc::new(crate::companion::tests::Fake::default());
    let mut relay = Relay::with_core(fake.clone());
    let (mut browser, _) = relay.pair_finish(1, "012345").await;
    let welcome = relay.hello(1, &mut browser).await;
    assert_eq!(
        welcome["features"],
        serde_json::json!(crate::companion::FEATURES)
    );
    browser
        .queue(br#"{"type":"overview","id":1}"#.to_vec())
        .unwrap();
    relay.flush(1, &mut browser);
    let overview = relay.message(1, &mut browser).await;
    let golden: Value = serde_json::from_slice(include_bytes!(
        "../../../../runner-core/src/companion/fixtures/result_overview.json"
    ))
    .unwrap();
    assert_eq!(overview["result"], golden["result"]);
    browser.queue(br#"{"type":"attach","id":2,"session":"terminal","cols":80,"rows":24,"control":"take","history":3}"#.to_vec()).unwrap();
    relay.flush(1, &mut browser);
    assert_eq!(
        relay.message(1, &mut browser).await,
        serde_json::json!({"type":"result","id":2,"result":null})
    );
    let snapshot = relay.message(1, &mut browser).await;
    assert_eq!(snapshot["type"], "snapshot");
    assert_eq!(snapshot["more"], false);
    assert_eq!(relay.message(1, &mut browser).await["type"], "resized");
    browser
        .queue(br#"{"type":"input","id":3,"session":"terminal","data":"aGVsbG8="}"#.to_vec())
        .unwrap();
    relay.flush(1, &mut browser);
    assert_eq!(relay.message(1, &mut browser).await["id"], 3);
    {
        let input = fake.input.lock().unwrap();
        assert_eq!(input[0].1, b"hello");
        assert!(
            matches!(&input[0].2, runner_core::protocol::terminal::InputEvent::Key { kind: runner_core::protocol::terminal::InputKind::Content { text } } if text == "hello")
        );
    }
    browser
        .queue(br#"{"type":"post","id":4,"mission":"ignored","to":[],"text":"ignored"}"#.to_vec())
        .unwrap();
    relay.flush(1, &mut browser);
    assert_eq!(
        relay.message(1, &mut browser).await["code"],
        "unknownRequest"
    );
    assert_eq!(*fake.history.lock().unwrap(), [3]);
    browser
        .queue(br#"{"type":"detach","id":5,"session":"terminal"}"#.to_vec())
        .unwrap();
    relay.flush(1, &mut browser);
    assert_eq!(relay.message(1, &mut browser).await["id"], 5);
}
