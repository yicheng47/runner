use super::*;
use gpui::{EntityInputHandler, TestAppContext, VisualTestContext};
use runner_app::i18n::{testing::with_locale, Locale};
use runner_core::{
    companion::{Overview, Status},
    protocol::{
        terminal::{
            TerminalAttachment, TerminalController, TerminalFrame, TerminalSnapshot,
            TerminalSubscription,
        },
        ClientError, ClientEvent, EventError, EventSubscription, Request, Response, Transport,
    },
    remote::{PairedDevice, RemoteTerminal},
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Mutex,
    },
};

struct Events(futures::channel::mpsc::UnboundedReceiver<ClientEvent>);
impl EventSubscription for Events {
    fn recv(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<ClientEvent, EventError>> + Send + '_>> {
        Box::pin(async { self.0.next().await.ok_or(EventError::Closed) })
    }
}
struct Frames {
    rx: mpsc::Receiver<Result<TerminalFrame, ClientError>>,
    tx: mpsc::Sender<Result<TerminalFrame, ClientError>>,
    cancels: Arc<AtomicUsize>,
    entered: mpsc::Sender<()>,
}
impl TerminalSubscription for Frames {
    fn recv(&mut self) -> Result<TerminalFrame, ClientError> {
        let _ = self.entered.send(());
        self.rx
            .recv()
            .unwrap_or_else(|_| Err(ClientError::msg("closed")))
    }
    fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
        let tx = self.tx.clone();
        let cancels = self.cancels.clone();
        Arc::new(move || {
            cancels.fetch_add(1, Ordering::SeqCst);
            let _ = tx.send(Err(ClientError::msg("cancelled")));
        })
    }
}
struct Fake {
    base: runner_daemon::daemon::InProcessTransport,
    computers: Mutex<Vec<Computer>>,
    overview: Mutex<Overview>,
    events: Mutex<Vec<futures::channel::mpsc::UnboundedSender<ClientEvent>>>,
    follows: Mutex<Vec<(String, bool)>>,
    pairs: AtomicUsize,
    pair_ops: Mutex<Vec<&'static str>>,
    refusal: Mutex<Option<String>>,
    attaches: Mutex<Vec<RemoteTerminal>>,
    input: Mutex<Vec<(String, Vec<u8>)>>,
    sizes: Mutex<Vec<(String, u16, u16)>>,
    takes: Mutex<Vec<(String, String, u16, u16)>>,
    controller: Mutex<TerminalController>,
    frame_senders: Mutex<Vec<mpsc::Sender<Result<TerminalFrame, ClientError>>>>,
    frame_entries: Mutex<Vec<mpsc::Receiver<()>>>,
    snapshot: Mutex<Vec<u8>>,
    fail_attach: AtomicBool,
    fail_take: AtomicBool,
    overview_gates: Mutex<HashMap<String, mpsc::Receiver<()>>>,
    overview_started: Mutex<Option<mpsc::Sender<String>>>,
    directory_gate: Mutex<Option<mpsc::Receiver<()>>>,
    directory_started: Mutex<Option<mpsc::Sender<()>>>,
    cancels: Arc<AtomicUsize>,
}
impl Fake {
    fn client(self: &Arc<Self>) -> DaemonClient {
        DaemonClient::new(self.clone())
    }
    fn emit(&self, name: &str, payload: serde_json::Value) {
        let event = ClientEvent {
            name: name.into(),
            payload,
        };
        self.events
            .lock()
            .unwrap()
            .retain(|tx| tx.unbounded_send(event.clone()).is_ok());
    }
}
impl Transport for Fake {
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        Ok(match request {
            Request::remote_computers {} => {
                let gate = self.directory_gate.lock().unwrap().take();
                if let Some(gate) = gate {
                    if let Some(started) = self.directory_started.lock().unwrap().as_ref() {
                        started.send(()).unwrap();
                    }
                    gate.recv().unwrap();
                }
                Response::remote_computers(Ok(self.computers.lock().unwrap().clone()))
            }
            Request::remote_follow { computer, follow } => {
                self.follows.lock().unwrap().push((computer, follow));
                Response::remote_follow(Ok(()))
            }
            Request::remote_overview { computer } => {
                let gate = self.overview_gates.lock().unwrap().remove(&computer);
                if let Some(gate) = gate {
                    if let Some(started) = self.overview_started.lock().unwrap().as_ref() {
                        started.send(computer).unwrap();
                    }
                    gate.recv().unwrap();
                }
                Response::remote_overview(Ok(runner_core::remote::RemoteOverview(
                    self.overview.lock().unwrap().clone(),
                )))
            }
            Request::remote_pair { computer, pin } => {
                self.pair_ops.lock().unwrap().push("pair");
                assert!(pin.0.len() == 6);
                self.pairs.fetch_add(1, Ordering::SeqCst);
                let result = self
                    .refusal
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|reason| Err(ClientError::msg(format!("Remote channel: {reason}."))))
                    .unwrap_or(Ok(()));
                if result.is_ok() {
                    self.computers
                        .lock()
                        .unwrap()
                        .iter_mut()
                        .find(|row| row.id == computer)
                        .unwrap()
                        .paired = true;
                }
                Response::remote_pair(result)
            }
            Request::remote_forget { computer } => {
                self.pair_ops.lock().unwrap().push("forget");
                self.computers
                    .lock()
                    .unwrap()
                    .iter_mut()
                    .find(|row| row.id == computer)
                    .unwrap()
                    .paired = false;
                Response::remote_forget(Ok(()))
            }
            Request::remote_take_control {
                computer,
                session,
                cols,
                rows,
            } => {
                self.takes
                    .lock()
                    .unwrap()
                    .push((computer, session, cols, rows));
                Response::remote_take_control(if self.fail_take.load(Ordering::SeqCst) {
                    Err(ClientError::msg("test take control refused"))
                } else {
                    Ok(())
                })
            }
            request => return self.base.call(request),
        })
    }
    fn subscribe(&self) -> Box<dyn EventSubscription> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        self.events.lock().unwrap().push(tx);
        Box::new(Events(rx))
    }
    fn attach_remote(&self, config: RemoteTerminal) -> Result<TerminalAttachment, ClientError> {
        let (cols, rows) = (config.cols, config.rows);
        self.attaches.lock().unwrap().push(config);
        if self.fail_attach.load(Ordering::SeqCst) {
            return Err(ClientError::msg("test attach failed"));
        }
        let (tx, rx) = mpsc::channel();
        let (entered, entries) = mpsc::channel();
        self.frame_senders.lock().unwrap().push(tx.clone());
        self.frame_entries.lock().unwrap().push(entries);
        Ok(TerminalAttachment {
            subscriber_id: 17,
            snapshot: TerminalSnapshot {
                seq: 1,
                cols,
                rows,
                controller: self.controller.lock().unwrap().clone(),
                bytes: self.snapshot.lock().unwrap().clone(),
                unfinished_len: 0,
                preceding_char: None,
            },
            frames: Box::new(Frames {
                rx,
                tx,
                cancels: self.cancels.clone(),
                entered,
            }),
        })
    }
    fn input(&self, address: &str, bytes: &[u8]) -> Result<(), ClientError> {
        self.input
            .lock()
            .unwrap()
            .push((address.into(), bytes.into()));
        Ok(())
    }
    fn resize(&self, address: &str, _: u64, cols: u16, rows: u16) {
        self.sizes
            .lock()
            .unwrap()
            .push((address.into(), cols, rows));
    }
}
fn computers() -> Vec<Computer> {
    vec![
        Computer {
            id: "self".into(),
            name: "Laptop".into(),
            is_self: true,
            online: true,
            fingerprint: "local-key".into(),
            ..Default::default()
        },
        Computer {
            id: "host".into(),
            name: "Studio".into(),
            online: true,
            paired: true,
            fingerprint: "host-key".into(),
            ..Default::default()
        },
    ]
}
fn overview() -> Overview {
    serde_json::from_value(serde_json::json!({"projects":[{"id":"p", "name":"Remote project"}], "tabs":[{"id":"remote-tab","project":"p","name":"Remote tab","pinned":true,"panes":{"session":"s"},"shells":[]}],"missions":[{"id":"remote-mission","title":"Remote mission","project":null,"crew":"Duo","state":"running","pinned":false,"members":[],"asks":[],"shells":[]}],"sessions":{"s":{"kind":"chat","title":"Remote session","runtime":"codex","status":"needsYou","reason":"approval","unread":true}}})).unwrap()
}
fn fixture(
    cx: &mut TestAppContext,
    path: &std::path::Path,
) -> (Entity<AppStore>, Arc<Fake>, Entity<model::ClientState>) {
    let store = crate::app_store::test_lifecycle_store(cx, path);
    cx.update(|cx| {
        cx.set_global(GlobalNativePaths(NativePaths::new(
            path.into(),
            path.join("logs"),
        )))
    });
    let fake = Arc::new(Fake {
        base: runner_daemon::daemon::InProcessTransport(
            store.read_with(cx, |store, _| store.test_core.clone()),
        ),
        computers: Mutex::new(computers()),
        overview: Mutex::new(overview()),
        events: Mutex::default(),
        follows: Mutex::default(),
        pairs: AtomicUsize::new(0),
        pair_ops: Mutex::default(),
        refusal: Mutex::default(),
        attaches: Mutex::default(),
        input: Mutex::default(),
        sizes: Mutex::default(),
        takes: Mutex::default(),
        controller: Mutex::new(TerminalController::Desktop),
        frame_senders: Mutex::default(),
        frame_entries: Mutex::default(),
        snapshot: Mutex::new(b"remote output".to_vec()),
        fail_attach: AtomicBool::new(false),
        fail_take: AtomicBool::new(false),
        overview_gates: Mutex::default(),
        overview_started: Mutex::default(),
        directory_gate: Mutex::default(),
        directory_started: Mutex::default(),
        cancels: Arc::new(AtomicUsize::new(0)),
    });
    let clients = cx.new(|cx| model::ClientState::new(fake.client(), cx));
    store.update(cx, |store, _| {
        store.client = fake.client();
        store.remote_clients = clients.clone();
    });
    clients.update(cx, |clients, cx| clients.refresh(cx));
    cx.run_until_parked();
    (store, fake, clients)
}
fn add_window(
    cx: &mut TestAppContext,
    store: &Entity<AppStore>,
    path: &std::path::Path,
    name: &str,
) -> gpui::WindowHandle<NativeRoot> {
    let window = cx.add_window(|window, cx| {
        NativeRoot::new(
            name.into(),
            path.join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    cx.simulate_window_resize(window.into(), size(px(1440.), px(900.)));
    window
}
fn device(name: &str, fingerprint: &str) -> PairedDevice {
    PairedDevice {
        name: name.into(),
        fingerprint: fingerprint.into(),
        public_key: String::new(),
        paired_at: chrono::Utc::now(),
        last_seen_at: chrono::Utc::now(),
        connected: true,
    }
}

#[test]
fn account_rows_keep_self_all_access_directions_and_unmatched_devices() {
    let mut computers = computers();
    computers.push(Computer {
        id: "other".into(),
        name: "Other".into(),
        ..Default::default()
    });
    let trust = RemoteTrust {
        devices: vec![
            device("Studio", "per-host-key"),
            device("Other", "other-client"),
            device("Outside account", "unmatched"),
        ],
        ..Default::default()
    };
    let rows = model::computer_rows(&computers, &trust);
    assert!(rows[0].access == Access::This);
    assert!(rows[1].access == Access::Both);
    assert!(rows[2].access == Access::Incoming);
    assert!(rows[3].computer.is_none());
    assert_eq!(rows[3].devices[0].fingerprint, "unmatched");
    let trust = RemoteTrust::default();
    let rows = model::computer_rows(&computers, &trust);
    assert!(rows[1].access == Access::Outgoing);
    assert!(rows[2].access == Access::Unpaired);
    let long = "名".repeat(30);
    assert_eq!(model::device_name(&long).len(), 63);
    let long_computer = Computer {
        name: long.clone(),
        ..Default::default()
    };
    let trust = RemoteTrust {
        devices: vec![device(model::device_name(&long), "long")],
        ..Default::default()
    };
    assert!(model::computer_rows(&[long_computer], &trust)[0].access == Access::Incoming);
}

#[test]
fn remote_terminal_uses_remote_address_discards_disconnected_input_and_cancels_on_leave() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (_, fake, _) = fixture(&mut cx, temp.path());
    let config = RemoteTerminal {
        computer: "host".into(),
        session: "s".into(),
        cols: 90,
        rows: 30,
        take_control: true,
    };
    let address = config.address();
    let connected = Arc::new(AtomicBool::new(true));
    let (client, lifetime) = transport::RemoteTransport::client(
        fake.client(),
        config,
        "Remote".into(),
        connected.clone(),
    );
    let first = client.attach(&address).unwrap();
    assert_eq!(first.snapshot.bytes, b"remote output");
    client.input(&address, b"before").unwrap();
    connected.store(false, Ordering::SeqCst);
    client.input(&address, b"dropped").unwrap();
    client.resize(&address, 17, 50, 20);
    connected.store(true, Ordering::SeqCst);
    client.input(&address, b"after").unwrap();
    client.resize(&address, 17, 2, 2);
    let second = client.attach(&address).unwrap();
    client.terminal_take_back(&address, 100, 40).unwrap();
    assert_eq!(
        *fake.input.lock().unwrap(),
        vec![
            (address.clone(), b"before".to_vec()),
            (address.clone(), b"after".to_vec())
        ]
    );
    assert_eq!(*fake.sizes.lock().unwrap(), vec![(address.clone(), 20, 8)]);
    assert_eq!(
        *fake.takes.lock().unwrap(),
        vec![("host".into(), "s".into(), 100, 40)]
    );
    assert!(fake.attaches.lock().unwrap()[0].take_control);
    assert!(!fake.attaches.lock().unwrap()[1].take_control);
    assert_eq!(fake.attaches.lock().unwrap()[1].cols, 20);
    assert!(client.input("s", b"local").is_err());
    drop(lifetime);
    assert_eq!(fake.cancels.load(Ordering::SeqCst), 1);
    drop((first, second));
}

#[test]
fn follow_leases_share_subscription_and_pushes_stay_in_the_remote_model() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    let one = clients.read_with(&cx, |clients, _| clients.follow("host".into()));
    let two = clients.read_with(&cx, |clients, _| clients.follow("host".into()));
    cx.run_until_parked();
    assert_eq!(*fake.follows.lock().unwrap(), vec![("host".into(), true)]);
    let local_sessions = store.read_with(&cx, |store, _| store.sessions.len());
    fake.emit("remote/session-status", serde_json::json!({"computer":"host","status":{"session":"s","status":"working","unread":false}}));
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"reconnecting"}),
    );
    cx.run_until_parked();
    assert!(
        clients.read_with(&cx, |clients, _| clients.connection("host")) == Connection::Reconnecting
    );
    assert!(
        clients.read_with(&cx, |clients, _| clients.overviews["host"].sessions["s"]
            .status
            .status
            .clone())
            == Status::Working
    );
    assert_eq!(
        store.read_with(&cx, |store, _| store.sessions.len()),
        local_sessions
    );
    drop(one);
    cx.run_until_parked();
    assert_eq!(fake.follows.lock().unwrap().len(), 1);
    drop(two);
    cx.run_until_parked();
    assert_eq!(
        *fake.follows.lock().unwrap(),
        vec![("host".into(), true), ("host".into(), false)]
    );
    fake.emit("remote/connection", serde_json::json!({"computer":"host","state":"reconnecting","error":"Remote channel: unknownDevice."}));
    cx.run_until_parked();
    assert!(clients.read_with(&cx, |clients, _| clients.connection("host")) == Connection::Revoked);
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"disconnected"}),
    );
    cx.run_until_parked();
    assert!(clients.read_with(&cx, |clients, _| clients.connection("host")) == Connection::Revoked);
    assert!(
        !clients
            .read_with(&cx, |clients, _| clients.computers.clone())
            .iter()
            .find(|c| c.id == "host")
            .unwrap()
            .paired
    );
    fake.emit(
        "remote/status",
        serde_json::to_value(RemoteStatus::default()).unwrap(),
    );
    cx.run_until_parked();
    assert!(
        clients.read_with(&cx, |clients, _| clients.computers.is_empty()
            && clients.overviews.is_empty())
    );
}

#[test]
fn one_pin_request_per_submit_and_every_refusal_is_rendered_without_exposing_pin() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, _) = fixture(&mut cx, temp.path());
            let window = add_window(&mut cx, &store, temp.path(), "pin");
            for (reason, expected) in [
                ("wrongPin", pairing::PairState::WrongPin),
                ("pinLocked", pairing::PairState::Locked),
                ("noPin", pairing::PairState::NoPin),
                ("busy", pairing::PairState::Failed),
            ] {
                *fake.refusal.lock().unwrap() = Some(reason.into());
                let before = fake.pairs.load(Ordering::SeqCst);
                window
                    .update(&mut cx, |root, window, cx| {
                        root.open_computer_pair(
                            computers()[1].clone(),
                            pairing::Purpose::PairOnly,
                            window,
                            cx,
                        );
                        let field = root.client_ui.pair.as_ref().unwrap().field.clone();
                        field.update(cx, |field, cx| {
                            field.replace_text_in_range(None, "001234", window, cx)
                        });
                        root.submit_computer_pin(window, cx);
                        root.submit_computer_pin(window, cx);
                        assert!(
                            root.client_ui.pair.as_ref().unwrap().state
                                == pairing::PairState::Checking
                        );
                    })
                    .unwrap();
                cx.run_until_parked();
                assert_eq!(fake.pairs.load(Ordering::SeqCst), before + 1);
                window
                    .read_with(&cx, |root, _| {
                        assert!(root.client_ui.pair.as_ref().unwrap().state == expected);
                    })
                    .unwrap();
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                assert!(visual.debug_bounds("COMPUTER_PAIR_DIALOG").is_some());
            }
            *fake.refusal.lock().unwrap() = None;
            let before = fake.pairs.load(Ordering::SeqCst);
            window
                .update(&mut cx, |root, window, cx| {
                    state(&root.app_store, cx).update(cx, |clients, _| {
                        clients.changed_identities.insert("host".into());
                    });
                    root.open_computer_pair(
                        computers()[1].clone(),
                        pairing::Purpose::Open { new_window: false },
                        window,
                        cx,
                    );
                    let field = root.client_ui.pair.as_ref().unwrap().field.clone();
                    field.update(cx, |field, cx| {
                        field.replace_text_in_range(None, "001234", window, cx)
                    });
                    root.submit_computer_pin(window, cx);
                    root.client_ui.pair = None;
                })
                .unwrap();
            cx.run_until_parked();
            assert_eq!(fake.pairs.load(Ordering::SeqCst), before + 1);
            assert!(fake.pair_ops.lock().unwrap().ends_with(&["forget", "pair"]));
            window
                .read_with(&cx, |root, _| assert!(root.remote_window.is_none()))
                .unwrap();
        });
    }
}

#[test]
fn remote_window_and_picker_render_all_connection_states_and_preserve_local_tabs() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, clients) = fixture(&mut cx, temp.path());
            *fake.controller.lock().unwrap() = TerminalController::Browser {
                channel: 0,
                name: "Host desktop".into(),
            };
            let window = add_window(&mut cx, &store, temp.path(), "remote-ui");
            window.update(&mut cx, |root, _, _| {
                let mut layout = PaneLayout::single(Some("local-only"), &["local-only".into()]);
                layout.split("p1", SplitOrientation::Row).unwrap();
                let node: runner_core::protocol::node::NodeRow = serde_json::from_value(serde_json::json!({"id":"local-tab","parent_id":null,"position":0,"type":"tab","name":"Local tab","ref_id":null,"layout":layout.serialize().unwrap(),"pinned_position":null,"last_completed_at":null,"last_viewed_at":null,"created_at":"2026-10-23T00:00:00Z"})).unwrap();
                root.tabs = TabSet::from_rows(&[node]);
            }).unwrap();
            let original = window
                .read_with(&cx, |root, _| {
                    root.tabs
                        .tabs()
                        .iter()
                        .map(|tab| tab.serialize().unwrap())
                        .collect::<Vec<_>>()
                })
                .unwrap();
            let original_route = window.read_with(&cx, |root, _| root.route.clone()).unwrap();
            window
                .update(&mut cx, |root, window, cx| {
                    root.choose_computer(computers()[1].clone(), false, window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            for variant in [
                theme::ThemeVariant::Carbon,
                theme::ThemeVariant::RunnerLight,
            ] {
                theme::set_active_variant(variant);
                window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
                cx.run_until_parked();
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                assert!(visual.debug_bounds("REMOTE_SIDEBAR_ROW").is_some());
                assert!(visual.debug_bounds("REMOTE_DISABLED_CREATE").is_some());
                assert!(visual.debug_bounds("REMOTE_CONNECTION_ROW").is_some());
                assert!(visual.debug_bounds("REMOTE_TAKEN_BACK").is_some());
            }
            for (state, error, expected) in [
                ("reconnecting", "", Connection::Reconnecting),
                ("disconnected", "route closed", Connection::Unreachable),
                (
                    "disconnected",
                    "Remote channel: unknownDevice.",
                    Connection::Revoked,
                ),
            ] {
                fake.emit(
                    "remote/connection",
                    serde_json::json!({"computer":"host","state":state,"error":error}),
                );
                cx.run_until_parked();
                assert!(
                    clients.read_with(&cx, |clients, _| clients.connection("host")) == expected
                );
                window
                    .read_with(&cx, |root, _| assert!(root.current_subjects().is_empty()))
                    .unwrap();
            }
            window
                .update(&mut cx, |root, _, cx| {
                    root.client_ui.picker_open = true;
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            assert!(visual.debug_bounds("COMPUTER_PICKER_ROW").is_some());
            window
                .update(&mut cx, |root, window, cx| {
                    root.return_to_this_computer(window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            window
                .read_with(&cx, |root, _| {
                    assert!(root.remote_window.is_none());
                    assert_eq!(root.route, original_route);
                    assert_eq!(
                        root.tabs
                            .tabs()
                            .iter()
                            .map(|tab| tab.serialize().unwrap())
                            .collect::<Vec<_>>(),
                        original
                    );
                })
                .unwrap();
            assert!(fake.cancels.load(Ordering::SeqCst) > 0);
            assert_eq!(
                fake.follows.lock().unwrap().last(),
                Some(&("host".into(), false))
            );
        });
    }
}

#[test]
fn picker_new_window_changes_only_the_new_window() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, _, _) = fixture(&mut cx, temp.path());
    let first = add_window(&mut cx, &store, temp.path(), "first");
    first
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), true, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    first
        .read_with(&cx, |root, _| assert!(root.remote_window.is_none()))
        .unwrap();
    let mut remote_count = 0;
    cx.update(|cx| {
        for window in cx.windows() {
            if let Some(window) = window.downcast::<NativeRoot>() {
                if window.read(cx).unwrap().remote_window.is_some() {
                    remote_count += 1;
                }
            }
        }
    });
    assert_eq!(remote_count, 1);
}

#[test]
fn daemon_recovery_replaces_closed_remote_stream_without_taking_control_and_retries_failures() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    let window = add_window(&mut cx, &store, temp.path(), "ipc-recovery");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    fake.frame_entries.lock().unwrap()[0].recv().unwrap();
    let original = window
        .read_with(&cx, |root, _| {
            root.remote_window.as_ref().unwrap().terminals["s"]
                .terminal
                .clone()
        })
        .unwrap();
    fake.frame_senders.lock().unwrap()[0]
        .send(Err(ClientError::msg("IPC closed")))
        .unwrap();
    fake.emit("daemon/disconnected", serde_json::Value::Null);
    *fake.snapshot.lock().unwrap() = b"fresh snapshot".to_vec();
    fake.emit("daemon/reconnected", serde_json::Value::Null);
    cx.run_until_parked();
    assert_eq!(fake.attaches.lock().unwrap().len(), 2);
    assert!(fake.attaches.lock().unwrap()[0].take_control);
    assert!(!fake.attaches.lock().unwrap()[1].take_control);
    let recovered = window
        .read_with(&cx, |root, _| {
            root.remote_window.as_ref().unwrap().terminals["s"]
                .terminal
                .clone()
        })
        .unwrap();
    assert!(!Arc::ptr_eq(&original, &recovered));
    fake.frame_entries.lock().unwrap()[1].recv().unwrap();
    fake.frame_senders.lock().unwrap()[1]
        .send(Ok(TerminalFrame::Output {
            seq: 2,
            bytes: b"\r\nrecovered output".to_vec(),
        }))
        .unwrap();
    fake.frame_entries.lock().unwrap()[1].recv().unwrap();
    let text = recovered
        .term
        .lock()
        .grid()
        .display_iter()
        .map(|cell| cell.cell.c)
        .collect::<String>();
    assert!(text.contains("fresh snapshot"));
    assert!(text.contains("recovered output"));
    cx.run_until_parked();
    fake.fail_attach.store(true, Ordering::SeqCst);
    fake.emit("daemon/disconnected", serde_json::Value::Null);
    fake.emit("daemon/reconnected", serde_json::Value::Null);
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    assert!(visual.debug_bounds("REMOTE_TERMINAL_FAILED").is_some());
    fake.fail_attach.store(false, Ordering::SeqCst);
    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(workspace::Item::Tab("remote-tab".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert!(!fake.attaches.lock().unwrap().last().unwrap().take_control);
    window
        .read_with(&cx, |root, _| {
            assert!(root
                .remote_window
                .as_ref()
                .unwrap()
                .terminals
                .contains_key("s"))
        })
        .unwrap();
}

#[test]
fn pair_again_uses_current_directory_identity_and_replaces_pin_entry_when_it_changes() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    let window = add_window(&mut cx, &store, temp.path(), "identity-change");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    {
        let mut rows = fake.computers.lock().unwrap();
        rows[1].fingerprint = "identity-B".into();
        rows[1].paired = false;
    }
    clients.update(&mut cx, |clients, cx| clients.refresh(cx));
    cx.run_until_parked();
    window
        .update(&mut cx, |root, window, cx| {
            assert_eq!(
                root.remote_window.as_ref().unwrap().computer.fingerprint,
                "identity-B"
            );
            root.open_computer_pair(
                computers()[1].clone(),
                pairing::Purpose::Open { new_window: false },
                window,
                cx,
            );
            let dialog = root.client_ui.pair.as_ref().unwrap();
            assert_eq!(dialog.computer.fingerprint, "identity-B");
            let field = dialog.field.clone();
            field.update(cx, |field, cx| {
                field.replace_text_in_range(None, "001234", window, cx)
            });
        })
        .unwrap();
    fake.computers.lock().unwrap()[1].fingerprint = "identity-C".into();
    clients.update(&mut cx, |clients, cx| clients.refresh(cx));
    cx.run_until_parked();
    window
        .update(&mut cx, |root, window, cx| {
            let dialog = root.client_ui.pair.as_ref().unwrap();
            assert_eq!(dialog.computer.fingerprint, "identity-C");
            assert!(dialog.field.read(cx).value().0.is_empty());
            assert!(dialog.state == pairing::PairState::Entry);
            let field = dialog.field.clone();
            field.update(cx, |field, cx| {
                field.replace_text_in_range(None, "001234", window, cx)
            });
            root.submit_computer_pin(window, cx);
        })
        .unwrap();
    fake.computers.lock().unwrap()[1].fingerprint = "identity-D".into();
    cx.run_until_parked();
    assert_eq!(fake.pairs.load(Ordering::SeqCst), 0);
    window
        .read_with(&cx, |root, cx| {
            let dialog = root.client_ui.pair.as_ref().unwrap();
            assert_eq!(dialog.computer.fingerprint, "identity-D");
            assert!(dialog.field.read(cx).value().0.is_empty());
        })
        .unwrap();
}

#[cfg(unix)]
#[test]
fn remote_windows_do_not_suppress_local_notifications_and_clicks_return_to_local_content() {
    use runner_core::protocol::{
        node::{NodeRow, NodeType},
        status::{AgentObservation, AgentStatus, HumanInteraction, Lifecycle, WaitReason},
    };
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, _, _) = fixture(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store)
    });
    let row = NodeRow {
        id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
        parent_id: None,
        position: 0,
        node_type: NodeType::Tab,
        name: Some("Local tab".into()),
        ref_id: None,
        layout: Some(
            PaneLayout::single(Some("direct-agent"), &[])
                .serialize()
                .unwrap(),
        ),
        pinned_position: None,
        last_completed_at: None,
        last_viewed_at: None,
        created_at: String::new(),
    };
    store
        .read_with(&cx, |store, _| store.client.clone())
        .node_tab_upsert(runner_core::protocol::node::NodeTabUpsertInput {
            id: row.id.clone(),
            parent_id: None,
            name: "Local tab".into(),
            layout: row.layout.clone().unwrap(),
        })
        .unwrap();
    store.update(&mut cx, |store, _| store.nodes = vec![row.clone()]);
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        crate::notifications::install(cx);
    });
    let window = add_window(&mut cx, &store, temp.path(), "main");
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.update(|window, _| window.activate_window());
    window
        .update(&mut cx, |root, window, cx| {
            root.tabs = TabSet::from_rows(&[row]);
            root.route = AppRoute::Chat;
            root.choose_computer(computers()[1].clone(), false, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    store.update(&mut cx, |store, cx| {
        store.session_statuses.insert(
            "direct-agent".into(),
            AgentStatus {
                lifecycle: Lifecycle::Running,
                observation: AgentObservation {
                    interactions: vec![HumanInteraction {
                        id: "local-approval".into(),
                        reason: WaitReason::Approval,
                        owners: Vec::new(),
                        since: 1,
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        store.revisions.activity += 1;
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    cx.update(crate::notifications::view_changed);
    cx.run_until_parked();
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    cx.simulate_system_notification_response(gpui::SystemNotificationResponse {
        tag: "direct-agent".into(),
        action_id: None,
    });
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| {
            assert!(root.remote_window.is_none());
            assert_eq!(root.route, AppRoute::Chat);
            assert_eq!(
                root.tabs.active().unwrap().focused_session_id(),
                Some("direct-agent")
            );
        })
        .unwrap();
    assert!(cx.delivered_system_notifications().is_empty());
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| crate::notifications::open("mission-agent", cx));
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| {
            assert!(root.remote_window.is_none());
            assert_eq!(root.route, AppRoute::Mission("test-mission".into()));
        })
        .unwrap();
}

#[test]
fn duplicate_pairing_labels_leave_each_device_unmatched_and_neither_computer_can_revoke_it() {
    for names in [
        ["Studio".to_owned(), "Studio".to_owned()],
        [
            format!("{}A", "名".repeat(22)),
            format!("{}B", "名".repeat(22)),
        ],
    ] {
        let mut computers = computers();
        computers[1].name = names[0].clone();
        computers.push(Computer {
            id: "other".into(),
            name: names[1].clone(),
            ..Default::default()
        });
        let trust = RemoteTrust {
            devices: vec![
                device(model::device_name(&names[0]), "first-key"),
                device(model::device_name(&names[1]), "second-key"),
            ],
            ..Default::default()
        };
        let rows = model::computer_rows(&computers, &trust);
        assert_eq!(rows.len(), 5);
        assert!(rows[1].access == Access::Outgoing);
        assert!(rows[2].access == Access::Unpaired);
        for computer in &computers[1..] {
            assert!(model::incoming_devices(computer, &computers, &trust).is_empty());
        }
        assert!(rows[1].devices.is_empty() && rows[2].devices.is_empty());
        for (row, fingerprint) in rows[3..].iter().zip(["first-key", "second-key"]) {
            assert!(row.computer.is_none());
            assert_eq!(row.devices.len(), 1);
            assert_eq!(row.devices[0].fingerprint, fingerprint);
        }
        computers[0].name = names[0].clone();
        computers.pop();
        assert!(model::incoming_devices(&computers[1], &computers, &trust).is_empty());
    }
}

#[test]
fn slow_overview_and_directory_do_not_block_other_computers() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (_, fake, _) = fixture(&mut cx, temp.path());
    for id in ["other", "forget"] {
        fake.computers.lock().unwrap().push(Computer {
            id: id.into(),
            paired: true,
            ..Default::default()
        });
    }
    let (unblock, blocked) = mpsc::channel();
    fake.overview_gates
        .lock()
        .unwrap()
        .insert("host".into(), blocked);
    let (started, entered) = mpsc::channel();
    *fake.overview_started.lock().unwrap() = Some(started);
    let (commands, requests) = futures::channel::mpsc::unbounded();
    let (updates, mut responses) = futures::channel::mpsc::unbounded();
    let jobs = Arc::new(Mutex::new(Vec::new()));
    let workers = jobs.clone();
    let client = fake.client();
    let dispatcher = std::thread::spawn(move || {
        futures::executor::block_on(model::run_commands(client, requests, updates, move |job| {
            workers
                .lock()
                .unwrap()
                .push(std::thread::spawn(move || futures::executor::block_on(job)));
        }))
    });
    let (observed, results) = mpsc::channel();
    let observer = std::thread::spawn(move || {
        futures::executor::block_on(async move {
            while let Some(update) = responses.next().await {
                observed.send(update).unwrap();
            }
        })
    });
    let timeout = std::time::Duration::from_secs(5);
    commands
        .unbounded_send(Command::Preview("host".into()))
        .unwrap();
    assert_eq!(entered.recv_timeout(timeout).unwrap(), "host");
    commands.unbounded_send(Command::Refresh).unwrap();
    assert!(matches!(
        results.recv_timeout(timeout).unwrap(),
        model::Update::Computers(Ok(_))
    ));
    let (release_directory, block_directory) = mpsc::channel();
    let (directory_started, directory_entered) = mpsc::channel();
    *fake.directory_gate.lock().unwrap() = Some(block_directory);
    *fake.directory_started.lock().unwrap() = Some(directory_started);
    commands.unbounded_send(Command::Refresh).unwrap();
    directory_entered.recv_timeout(timeout).unwrap();
    commands
        .unbounded_send(Command::Acquire("other".into()))
        .unwrap();
    commands
        .unbounded_send(Command::Retry("other".into()))
        .unwrap();
    commands
        .unbounded_send(Command::Forget("forget".into()))
        .unwrap();
    let (mut connected, mut forgotten) = (0, false);
    while connected < 2 || !forgotten {
        match results.recv_timeout(timeout).unwrap() {
            model::Update::Connection(id, Connection::Connected) if id == "other" => connected += 1,
            model::Update::Forgotten(id, true) if id == "forget" => forgotten = true,
            _ => {}
        }
    }
    assert_eq!(fake.pairs.load(Ordering::SeqCst), 0);
    commands
        .unbounded_send(Command::Release("other".into()))
        .unwrap();
    drop(commands);
    dispatcher.join().unwrap();
    unblock.send(()).unwrap();
    release_directory.send(()).unwrap();
    for job in std::mem::take(&mut *jobs.lock().unwrap()) {
        job.join().unwrap();
    }
    observer.join().unwrap();
    assert_eq!(
        fake.follows.lock().unwrap().last(),
        Some(&("other".into(), false))
    );
}

#[test]
fn forgetting_locally_closes_the_mirror_and_uses_plain_pairing_copy_despite_late_events() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, clients) = fixture(&mut cx, temp.path());
            let window = add_window(&mut cx, &store, temp.path(), "forget");
            window
                .update(&mut cx, |root, window, cx| {
                    root.choose_computer(computers()[1].clone(), false, window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            clients.read_with(&cx, |clients, _| {
                clients.send(Command::Forget("host".into()))
            });
            cx.run_until_parked();
            for (state, error) in [
                ("disconnected", ""),
                ("reconnecting", "Remote channel: unknownDevice."),
                ("connected", ""),
            ] {
                fake.emit(
                    "remote/connection",
                    serde_json::json!({"computer":"host","state":state,"error":error}),
                );
            }
            fake.emit(
                "remote/overview",
                serde_json::json!({"computer":"host","overview":overview()}),
            );
            fake.emit("daemon/reconnected", serde_json::Value::Null);
            cx.run_until_parked();
            assert!(
                clients.read_with(&cx, |clients, _| clients.connection("host"))
                    == Connection::Forgotten
            );
            window
                .read_with(&cx, |root, _| {
                    let remote = root.remote_window.as_ref().unwrap();
                    assert!(remote.connection == Connection::Forgotten);
                    assert!(remote.terminals.is_empty());
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            assert!(visual.debug_bounds("REMOTE_CONNECTION_CARD").is_some());
            window
                .update(&mut cx, |root, window, cx| {
                    root.open_computer_pair(
                        computers()[1].clone(),
                        pairing::Purpose::Open { new_window: false },
                        window,
                        cx,
                    );
                    assert!(!root.client_ui.pair.as_ref().unwrap().again);
                })
                .unwrap();
        });
    }
}

#[test]
fn mission_members_receive_equal_width_and_empty_shells_draw_no_strip() {
    use runner_core::companion::Pane;
    fn widths(pane: &Pane, width: f64, found: &mut Vec<(String, f64)>) {
        match pane {
            Pane::Leaf { session: Some(id) } => found.push((id.clone(), width)),
            Pane::Split { sizes, a, b, .. } => {
                let total = sizes[0] + sizes[1];
                widths(a, width * sizes[0] / total, found);
                widths(b, width * sizes[1] / total, found);
            }
            _ => {}
        }
    }
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    let window = add_window(&mut cx, &store, temp.path(), "mission");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    for count in [0, 1, 2, 3, 4, 7, 24] {
        let mut overview = overview();
        overview.missions[0].members = (0..count).map(|i| serde_json::from_value(serde_json::json!({"handle":format!("slot-{i}"),"session":format!("session-{i}"),"lead":i==0})).unwrap()).collect();
        let mut found = Vec::new();
        widths(
            &workspace::mission_panes(&overview.missions[0].members),
            1.,
            &mut found,
        );
        assert_eq!(found.len(), count);
        for (i, (id, width)) in found.iter().enumerate() {
            assert_eq!(id, &format!("session-{i}"));
            assert!((width - 1. / count as f64).abs() < 1e-10);
        }
        fake.emit(
            "remote/overview",
            serde_json::json!({"computer":"host","overview":overview}),
        );
        window
            .update(&mut cx, |root, _, cx| {
                root.select_remote_item(workspace::Item::Mission("remote-mission".into()), cx)
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        assert!(visual.debug_bounds("REMOTE_SHELLS").is_none());
    }
}

#[test]
fn take_control_failure_is_visible_and_reselecting_current_computer_preserves_the_mirror() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, _) = fixture(&mut cx, temp.path());
            *fake.controller.lock().unwrap() = TerminalController::Browser {
                channel: 0,
                name: "Host desktop".into(),
            };
            let window = add_window(&mut cx, &store, temp.path(), "control");
            window
                .update(&mut cx, |root, window, cx| {
                    root.choose_computer(computers()[1].clone(), false, window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            let original = window
                .read_with(&cx, |root, _| {
                    root.remote_window.as_ref().unwrap().terminals["s"]
                        .terminal
                        .clone()
                })
                .unwrap();
            let attaches = fake.attaches.lock().unwrap().len();
            let follows = fake.follows.lock().unwrap().len();
            window
                .update(&mut cx, |root, window, cx| {
                    root.choose_computer(computers()[1].clone(), false, window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            window
                .read_with(&cx, |root, _| {
                    assert!(Arc::ptr_eq(
                        &original,
                        &root.remote_window.as_ref().unwrap().terminals["s"].terminal
                    ))
                })
                .unwrap();
            assert_eq!(fake.attaches.lock().unwrap().len(), attaches);
            assert_eq!(fake.follows.lock().unwrap().len(), follows);
            fake.fail_take.store(true, Ordering::SeqCst);
            window
                .update(&mut cx, |root, _, cx| root.take_remote_control("s", cx))
                .unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            assert!(visual.debug_bounds("REMOTE_CONTROL_FAILED").is_some());
            fake.fail_take.store(false, Ordering::SeqCst);
            window
                .update(&mut cx, |root, _, cx| root.take_remote_control("s", cx))
                .unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            assert!(visual.debug_bounds("REMOTE_CONTROL_FAILED").is_none());
            assert_eq!(fake.takes.lock().unwrap().len(), 2);
            fake.fail_take.store(true, Ordering::SeqCst);
            window
                .update(&mut cx, |root, _, cx| root.take_remote_control("s", cx))
                .unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            assert!(visual.debug_bounds("REMOTE_CONTROL_FAILED").is_some());
            window
                .update(&mut cx, |root, _, cx| {
                    root.select_remote_item(workspace::Item::Mission("remote-mission".into()), cx)
                })
                .unwrap();
            cx.run_until_parked();
            *fake.controller.lock().unwrap() = TerminalController::Desktop;
            window
                .update(&mut cx, |root, _, cx| {
                    root.select_remote_item(workspace::Item::Tab("remote-tab".into()), cx)
                })
                .unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            assert!(visual.debug_bounds("REMOTE_CONTROL_FAILED").is_none());
            assert!(visual.debug_bounds("REMOTE_TAKEN_BACK").is_none());
            assert!(fake.attaches.lock().unwrap().last().unwrap().take_control);
            let attaches = fake.attaches.lock().unwrap().len();
            fake.emit("remote/connection", serde_json::json!({"computer":"host","state":"disconnected","error":"route closed"}));
            cx.run_until_parked();
            window
                .update(&mut cx, |root, window, cx| {
                    root.choose_computer(computers()[1].clone(), false, window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            assert_eq!(fake.attaches.lock().unwrap().len(), attaches + 1);
            window
                .read_with(&cx, |root, _| {
                    let remote = root.remote_window.as_ref().unwrap();
                    assert!(remote.connection == Connection::Connected);
                    assert!(!Arc::ptr_eq(&original, &remote.terminals["s"].terminal));
                })
                .unwrap();
        });
    }
}

#[test]
fn column_split_divides_panes_horizontally_without_insetting_the_second_header() {
    use runner_core::companion::{Pane, SplitDirection};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    let mut overview = overview();
    overview.tabs[0].panes = Pane::Split {
        split: SplitDirection::Column,
        sizes: [0.5, 0.5],
        a: Box::new(Pane::Leaf {
            session: Some("s".into()),
        }),
        b: Box::new(Pane::Leaf {
            session: Some("second".into()),
        }),
    };
    overview
        .sessions
        .insert("second".into(), overview.sessions["s"].clone());
    *fake.overview.lock().unwrap() = overview;
    let window = add_window(&mut cx, &store, temp.path(), "column");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let first = visual.debug_bounds("REMOTE_TERMINAL_HEADER_s").unwrap();
    let second = visual
        .debug_bounds("REMOTE_TERMINAL_HEADER_second")
        .unwrap();
    assert_eq!(first.origin.x, second.origin.x);
    assert_eq!(first.size.width, second.size.width);
    assert!(second.origin.y > first.bottom());
}

#[test]
fn pairing_again_restores_existing_windows_follow_and_replaces_its_terminal() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for forget_here in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, clients) = fixture(&mut cx, temp.path());
        let window = add_window(&mut cx, &store, temp.path(), "pair-again");
        window
            .update(&mut cx, |root, window, cx| {
                root.choose_computer(computers()[1].clone(), false, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        let original = window
            .read_with(&cx, |root, _| {
                root.remote_window.as_ref().unwrap().terminals["s"]
                    .terminal
                    .clone()
            })
            .unwrap();
        if forget_here {
            clients.read_with(&cx, |clients, _| {
                clients.send(Command::Forget("host".into()))
            });
        } else {
            fake.computers.lock().unwrap()[1].paired = false;
            fake.emit("remote/connection", serde_json::json!({"computer":"host","state":"disconnected","error":"Remote channel: unknownDevice."}));
        }
        cx.run_until_parked();
        let follows_before = fake.follows.lock().unwrap().len();
        *fake.snapshot.lock().unwrap() = b"newly paired snapshot".to_vec();
        window
            .update(&mut cx, |root, window, cx| {
                root.open_computer_pair(
                    computers()[1].clone(),
                    pairing::Purpose::Open { new_window: false },
                    window,
                    cx,
                );
                let field = root.client_ui.pair.as_ref().unwrap().field.clone();
                field.update(cx, |field, cx| {
                    field.replace_text_in_range(None, "001234", window, cx)
                });
                root.submit_computer_pin(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(fake.pairs.load(Ordering::SeqCst), 1);
        assert!(fake.follows.lock().unwrap()[follows_before..].contains(&("host".into(), true)));
        window
            .read_with(&cx, |root, _| {
                assert!(root.client_ui.pair.is_none());
                let remote = root.remote_window.as_ref().unwrap();
                assert!(remote.connection == Connection::Connected);
                assert!(!Arc::ptr_eq(&original, &remote.terminals["s"].terminal));
            })
            .unwrap();
        let mut pushed = overview();
        pushed.tabs[0].name = "After pairing".into();
        fake.emit(
            "remote/overview",
            serde_json::json!({"computer":"host","overview":pushed}),
        );
        fake.emit("remote/session-status", serde_json::json!({"computer":"host","status":{"session":"s","status":"working","unread":false}}));
        cx.run_until_parked();
        clients.read_with(&cx, |clients, _| {
            assert_eq!(clients.overviews["host"].tabs[0].name, "After pairing");
            assert!(clients.overviews["host"].sessions["s"].status.status == Status::Working);
        });
    }
}
