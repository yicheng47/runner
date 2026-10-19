use super::workspace::Item;
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
    remote::{PairedDevice, RemoteAccount, RemoteState, RemoteStatus, RemoteTerminal, StopReason},
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
pub(crate) struct Fake {
    base: runner_daemon::daemon::InProcessTransport,
    status: Mutex<RemoteStatus>,
    directory_calls: AtomicUsize,
    enrolls: AtomicUsize,
    computers: Mutex<Vec<Computer>>,
    overview: Mutex<Overview>,
    overview_calls: AtomicUsize,
    follow_push: AtomicBool,
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
    fail_input: AtomicBool,
    fail_take: AtomicBool,
    fail_directory: AtomicBool,
    overview_gates: Mutex<HashMap<String, mpsc::Receiver<()>>>,
    overview_started: Mutex<Option<mpsc::Sender<String>>>,
    directory_gate: Mutex<Option<mpsc::Receiver<()>>>,
    directory_started: Mutex<Option<mpsc::Sender<()>>>,
    cancels: Arc<AtomicUsize>,
    features: Mutex<Vec<String>>,
    action_requests: Mutex<Vec<(String, serde_json::Value)>>,
    local_requests: Mutex<Vec<std::mem::Discriminant<Request>>>,
    action_results:
        Mutex<HashMap<String, std::collections::VecDeque<Result<serde_json::Value, ClientError>>>>,
    model_session_removal: AtomicBool,
}
impl Fake {
    fn model_action(
        &self,
        value: &serde_json::Value,
    ) -> Option<Result<serde_json::Value, ClientError>> {
        if !self.model_session_removal.load(Ordering::SeqCst) {
            return None;
        }
        use runner_core::companion::Pane;
        fn remove(pane: &mut Pane, id: &str) -> (bool, bool) {
            match pane {
                Pane::Leaf { session } => {
                    let changed = session.as_deref() == Some(id);
                    if changed {
                        *session = None;
                    }
                    (changed, session.is_some())
                }
                Pane::Split { a, b, .. } => {
                    let (changed_a, populated_a) = remove(a, id);
                    let (changed_b, populated_b) = remove(b, id);
                    (changed_a || changed_b, populated_a || populated_b)
                }
                Pane::Unknown(_) => panic!("session-removal fixture has an unknown pane"),
            }
        }
        let kind = value["type"].as_str()?;
        let mut overview = self.overview.lock().unwrap();
        match kind {
            "close" | "archive" if kind == "close" || value["target"]["kind"] == "session" => {
                let id = if kind == "close" {
                    value["session"].as_str().unwrap()
                } else {
                    value["target"]["id"].as_str().unwrap()
                };
                overview.sessions.remove(id);
                overview.tabs.retain_mut(|tab| {
                    let before = tab.shells.len();
                    tab.shells.retain(|shell| shell != id);
                    let (changed, populated) = remove(&mut tab.panes, id);
                    (!changed && before == tab.shells.len()) || populated || !tab.shells.is_empty()
                });
                Some(Ok(serde_json::Value::Null))
            }
            "archive" if value["target"]["kind"] == "tab" => {
                let id = value["target"]["id"].as_str().unwrap();
                if !overview.tabs.iter().any(|tab| tab.id == id) {
                    return Some(Err(ClientError::msg("No sidebar item has that id.")));
                }
                overview.tabs.retain(|tab| tab.id != id);
                Some(Ok(serde_json::Value::Null))
            }
            "layout" => {
                let id = value["target"]["id"].as_str().unwrap();
                let Some(tab) = overview.tabs.iter_mut().find(|tab| tab.id == id) else {
                    return Some(Err(ClientError::msg("No sidebar item has that id.")));
                };
                tab.panes = serde_json::from_value(value["panes"].clone()).unwrap();
                tab.shells = serde_json::from_value(value["shells"].clone()).unwrap();
                Some(Ok(serde_json::json!({"target":value["target"]})))
            }
            _ => None,
        }
    }
    pub(crate) fn set_features(&self, features: &[&str]) {
        *self.features.lock().unwrap() = features
            .iter()
            .map(|feature| (*feature).to_owned())
            .collect();
    }
    pub(crate) fn queue_result(&self, kind: &str, result: Result<serde_json::Value, ClientError>) {
        self.action_results
            .lock()
            .unwrap()
            .entry(kind.into())
            .or_default()
            .push_back(result);
    }
    pub(crate) fn request_count(&self, kind: &str) -> usize {
        action_trace(self, kind).len()
    }
    pub(crate) fn local_call_count(&self) -> usize {
        self.local_requests.lock().unwrap().len()
    }

    pub(crate) fn set_overview(&self, data: Overview) {
        *self.overview.lock().unwrap() = data.clone();
        self.emit(
            "remote/overview",
            serde_json::json!({"computer":"host", "overview":data}),
        );
    }
    fn client(self: &Arc<Self>) -> DaemonClient {
        DaemonClient::new(self.clone())
    }
    pub(crate) fn emit(&self, name: &str, payload: serde_json::Value) {
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
            Request::remote_status {} => {
                Response::remote_status(Ok(self.status.lock().unwrap().clone()))
            }
            Request::remote_enroll_start { .. } => {
                self.enrolls.fetch_add(1, Ordering::SeqCst);
                Response::remote_enroll_start(Err(ClientError::msg("test enrollment refused")))
            }
            Request::remote_computers {} => {
                self.directory_calls.fetch_add(1, Ordering::SeqCst);
                let gate = self.directory_gate.lock().unwrap().take();
                if let Some(gate) = gate {
                    if let Some(started) = self.directory_started.lock().unwrap().as_ref() {
                        started.send(()).unwrap();
                    }
                    gate.recv().unwrap();
                }
                Response::remote_computers(if self.fail_directory.load(Ordering::SeqCst) {
                    Err(ClientError::msg("test directory unavailable"))
                } else {
                    Ok(self.computers.lock().unwrap().clone())
                })
            }
            Request::remote_follow { computer, follow } => {
                self.follows
                    .lock()
                    .unwrap()
                    .push((computer.clone(), follow));
                if follow && self.follow_push.load(Ordering::SeqCst) {
                    self.emit("remote/overview", serde_json::json!({"computer":computer,"overview":self.overview.lock().unwrap().clone()}));
                }
                Response::remote_follow(Ok(()))
            }
            Request::remote_overview { computer } => {
                self.overview_calls.fetch_add(1, Ordering::SeqCst);
                let overview = self.overview.lock().unwrap().clone();
                let gate = self.overview_gates.lock().unwrap().remove(&computer);
                if let Some(gate) = gate {
                    if let Some(started) = self.overview_started.lock().unwrap().as_ref() {
                        started.send(computer).unwrap();
                    }
                    gate.recv().unwrap();
                }
                Response::remote_overview(Ok(runner_core::remote::RemoteOverview(overview)))
            }
            Request::remote_request { computer, request } => {
                let value = serde_json::to_value(request).unwrap();
                let kind = value["type"].as_str().unwrap().to_owned();
                self.action_requests
                    .lock()
                    .unwrap()
                    .push((computer, value.clone()));
                let result = self.action_results.lock().unwrap().get_mut(&kind).and_then(|results| results.pop_front()).unwrap_or_else(|| {
                    if let Some(result) = self.model_action(&value) { return result; }
                    Ok(match kind.as_str() {
                    "catalog" => serde_json::json!({"roles":[{"id":"host-role","name":"Host coder","handle":"host-coder","runtime":"codex","model":null,"effort":null,"speed":null}],"runtimes":[{"name":"codex","label":"Host Codex","available":true,"nativeFork":true,"defaultModel":null,"defaultEffort":null,"models":[],"efforts":[],"speeds":["standard","fast"]}],"accounts":[{"id":"host-account","name":"Host account"}],"activeAccount":"host-account","crews":[],"projects":[{"id":"host-project","name":"Host project","path":"C:\\work"}],"recentFolders":["C:\\work","\\\\nas\\share"]}),
                    "folders" => serde_json::json!({"path":value["path"].as_str().unwrap_or("C:\\Users\\Jason"),"parent":null,"folders":[]}),
                    "startChat" | "startShell" | "fork" => serde_json::json!({"session":"started-session"}),
                    "resume" | "restart" => serde_json::json!({"session":value["session"]}),
                    "feed" => serde_json::json!({"entries":[],"before":null}),
                    "startMission" => serde_json::json!({"mission":"created-mission"}),
                    "resumeMission" => serde_json::json!({"mission":value["mission"],"sessions":["member"]}),
                    "layout" => serde_json::json!({"target":value["target"]}),
                    "createProject" => serde_json::json!({"project":"new-project"}),
                    "deleteProject" => serde_json::json!({"archivedMissions":[],"archivedSessions":[]}),
                    _ => serde_json::Value::Null,
                }) });
                Response::remote_request(result.map(|result| runner_core::remote::RemoteResult {
                    result,
                    features: self.features.lock().unwrap().clone(),
                }))
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
            request => {
                self.local_requests
                    .lock()
                    .unwrap()
                    .push(std::mem::discriminant(&request));
                return self.base.call(request);
            }
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
        if self.fail_input.load(Ordering::SeqCst) {
            Err(ClientError::msg("input was discarded"))
        } else {
            Ok(())
        }
    }
    fn resize(&self, address: &str, _: u64, cols: u16, rows: u16) {
        self.sizes
            .lock()
            .unwrap()
            .push((address.into(), cols, rows));
    }
}
pub(crate) fn choose_host(
    root: &mut NativeRoot,
    window: &mut Window,
    cx: &mut Context<NativeRoot>,
) {
    root.choose_computer(computers()[1].clone(), false, window, cx);
}
pub(crate) fn computers() -> Vec<Computer> {
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

#[test]
fn synthesized_computer_preserves_identity_and_only_shows_the_host_name_when_requested() {
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let status = RemoteStatus {
                host_id: Some("local-host".into()),
                host_name: Some("Local host".into()),
                ..Default::default()
            };
            let named = model::this_computer(&status, true);
            assert_eq!(named.id, "local-host");
            assert_eq!(named.name, "Local host");
            assert!(named.is_self);
            let generic = model::this_computer(&status, false);
            assert_eq!(generic.id, named.id);
            assert_eq!(generic.name, crate::i18n::text("client.this_computer"));
            assert!(generic.is_self);
            let missing = model::this_computer(&RemoteStatus::default(), true);
            assert!(missing.id.is_empty());
            assert_eq!(missing.name, generic.name);
            assert!(missing.is_self);
        });
    }
}
fn sidebar_overview() -> Overview {
    let mut data = overview();
    let mut project_tab = data.tabs[0].clone();
    project_tab.id = "project-tab".into();
    project_tab.name = "Project tab".into();
    project_tab.pinned = false;
    data.tabs.push(project_tab);
    let mut recent_tab = data.tabs[1].clone();
    recent_tab.id = "recent-tab".into();
    recent_tab.name = "Recent tab".into();
    recent_tab.project = None;
    data.tabs.push(recent_tab);
    let mut pinned_mission = data.missions[0].clone();
    pinned_mission.id = "pinned-mission".into();
    pinned_mission.title = "Pinned mission".into();
    pinned_mission.project = Some("p".into());
    pinned_mission.pinned = true;
    data.missions.push(pinned_mission);
    let mut project_mission = data.missions[0].clone();
    project_mission.id = "project-mission".into();
    project_mission.title = "Project mission".into();
    project_mission.project = Some("p".into());
    data.missions.push(project_mission);
    data
}
pub(crate) fn fixture(
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
        status: Mutex::new(RemoteStatus {
            state: RemoteState::Paused,
            ..Default::default()
        }),
        directory_calls: AtomicUsize::new(0),
        enrolls: AtomicUsize::new(0),
        computers: Mutex::new(computers()),
        overview: Mutex::new(overview()),
        overview_calls: AtomicUsize::new(0),
        follow_push: AtomicBool::new(false),
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
        fail_input: AtomicBool::new(false),
        fail_take: AtomicBool::new(false),
        fail_directory: AtomicBool::new(false),
        overview_gates: Mutex::default(),
        overview_started: Mutex::default(),
        directory_gate: Mutex::default(),
        directory_started: Mutex::default(),
        cancels: Arc::new(AtomicUsize::new(0)),
        features: Mutex::new(Vec::new()),
        action_requests: Mutex::new(Vec::new()),
        local_requests: Mutex::new(Vec::new()),
        action_results: Mutex::new(HashMap::new()),
        model_session_removal: AtomicBool::new(false),
    });
    let clients = cx.new(|cx| model::ClientState::new(fake.client(), cx));
    store.update(cx, |store, _| {
        store.settings.local_only = false;
        store.client = fake.client();
        store.remote_clients = clients.clone();
    });
    clients.update(cx, |clients, cx| clients.refresh(cx));
    cx.run_until_parked();
    store.update(cx, |store, _| {
        store.remote_status = Some(fake.status.lock().unwrap().clone());
    });
    (store, fake, clients)
}

#[test]
fn remote_layout_round_trips_through_the_host_desktop_model() {
    use runner_daemon::companion::core::{Core, DaemonCore};
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, _, _) = fixture(&mut cx, temp.path());
    let core = Arc::new(store.read_with(&cx, |store, _| store.test_core.clone()));
    let host = DaemonCore(Arc::downgrade(&core));
    for sizes in [[0.4, 0.6], [40., 60.], [4., 6.]] {
        let request = serde_json::from_value(serde_json::json!({
            "type":"layout", "id":1,
            "target":{"kind":"tab","id":"01ARZ3NDEKTSV4RRFFQ69G5FAV"},
            "name":"Split", "shells":[],
            "panes":{"id":"s1","split":"row","sizes":sizes,
                "a":{"id":"p1","session":null},
                "b":{"id":"s2","split":"column","sizes":[0.25,0.75],
                    "a":{"id":"p2","session":null},
                    "b":{"id":"p3","session":null}}}
        }))
        .unwrap();
        host.action(request).unwrap();
        let rows = runner_daemon::ops::node::node_list(&core).unwrap();
        let row = rows
            .iter()
            .find(|row| row.id == "01ARZ3NDEKTSV4RRFFQ69G5FAV")
            .unwrap();
        let layout = PaneLayout::from_node_row(row).unwrap();
        let runner_app::pane_layout::PaneNode::Split(split) = layout.root else {
            panic!("split")
        };
        assert_eq!(split.sizes, [40., 60.]);
        let runner_app::pane_layout::PaneNode::Split(nested) = *split.b else {
            panic!("nested split")
        };
        assert_eq!(nested.sizes, [25., 75.]);
    }
}
pub(crate) fn add_window(
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
    assert_eq!(*fake.follows.lock().unwrap(), vec![("host".into(), true)]);
    clients.update(&mut cx, |clients, cx| clients.set_enabled(false, cx));
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
fn shared_remote_sidebar_controls_do_not_open_local_creation_or_change_local_state() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, _, _) = fixture(&mut cx, temp.path());
    let window = add_window(&mut cx, &store, temp.path(), "remote-sidebar-actions");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let original = window
        .read_with(&cx, |root, _| {
            (
                root.route.clone(),
                root.tabs
                    .tabs()
                    .iter()
                    .map(|tab| tab.serialize().unwrap())
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    for selector in [
        "SIDEBAR_WORKSPACE_NEW_CHAT",
        "SIDEBAR_WORKSPACE:workspace-role",
        "SIDEBAR_WORKSPACE:workspace-crew",
    ] {
        assert!(visual.debug_bounds(selector).is_none());
        window
            .read_with(&visual, |root, _| {
                assert!(root.remote_window.is_some());
                assert!(root.route == original.0);
                assert!(root.start_chat_modal.is_none());
                assert!(root.start_mission_modal.is_none());
                assert!(root.project_modal.is_none());
                assert_eq!(
                    root.tabs
                        .tabs()
                        .iter()
                        .map(|tab| tab.serialize().unwrap())
                        .collect::<Vec<_>>(),
                    original.1
                );
            })
            .unwrap();
    }
}

#[test]
fn remote_sidebar_node_metadata_preserves_mixed_order_and_updates_placement() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    let mut data = sidebar_overview();
    data.projects.insert(
        0,
        runner_core::companion::Project {
            id: "q".into(),
            name: "Other project".into(),
        },
    );
    data.sidebar = Some(serde_json::from_value(serde_json::json!([
        {"kind":"tab","id":"recent-tab","parent":null,"position":1,"pinnedPosition":null},
        {"kind":"tab","id":"remote-tab","parent":"p","position":0,"pinnedPosition":1},
        {"kind":"project","id":"q","parent":null,"position":1,"pinnedPosition":null},
        {"kind":"mission","id":"project-mission","parent":"p","position":1,"pinnedPosition":null},
        {"kind":"mission","id":"pinned-mission","parent":"p","position":2,"pinnedPosition":0},
        {"kind":"tab","id":"project-tab","parent":"p","position":2,"pinnedPosition":null},
        {"kind":"mission","id":"remote-mission","parent":null,"position":0,"pinnedPosition":null},
        {"kind":"project","id":"p","parent":null,"position":0,"pinnedPosition":null},
        {"kind":"futureKind","id":"future","parent":null,"position":3,"pinnedPosition":null}
    ])).unwrap());
    fake.set_overview(data.clone());
    let window = add_window(&mut cx, &store, temp.path(), "sidebar-node-order");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let before = |visual: &mut VisualTestContext, a, b| {
        assert!(
            visual.debug_bounds(a).unwrap().top() < visual.debug_bounds(b).unwrap().top(),
            "{a} must precede {b}"
        );
    };
    before(
        &mut visual,
        "SIDEBAR_ITEM_TITLE:Pinned mission",
        "SIDEBAR_ITEM_TITLE:Remote tab",
    );
    before(&mut visual, "SIDEBAR_PROJECT:p", "SIDEBAR_PROJECT:q");
    before(
        &mut visual,
        "SIDEBAR_ITEM_TITLE:Project mission",
        "SIDEBAR_ITEM_TITLE:Project tab",
    );
    before(
        &mut visual,
        "SIDEBAR_ITEM_TITLE:Remote mission",
        "SIDEBAR_ITEM_TITLE:Recent tab",
    );
    let moved = data
        .sidebar
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|node| node.id == "project-tab")
        .unwrap();
    moved.parent = None;
    moved.position = -1;
    data.tabs
        .iter_mut()
        .find(|tab| tab.id == "project-tab")
        .unwrap()
        .project = None;
    fake.emit(
        "remote/overview",
        serde_json::json!({"computer":"host","overview":data}),
    );
    visual.run_until_parked();
    before(
        &mut visual,
        "SIDEBAR_SECTION:RECENTS",
        "SIDEBAR_ITEM_TITLE:Project tab",
    );
    before(
        &mut visual,
        "SIDEBAR_ITEM_TITLE:Project tab",
        "SIDEBAR_ITEM_TITLE:Remote mission",
    );
    data.sidebar = Some(Vec::new());
    fake.emit(
        "remote/overview",
        serde_json::json!({"computer":"host","overview":data}),
    );
    visual.run_until_parked();
    assert!(visual
        .debug_bounds("SIDEBAR_ITEM_TITLE:Remote tab")
        .is_none());
    assert!(visual.debug_bounds("SIDEBAR_PROJECT:p").is_none());
}

#[test]
fn remote_status_adapter_keeps_reasons_and_shared_rollup_priority() {
    use runner_app::ui::agent_status::{StatusKind, StatusPresentation, StatusRollup};
    use runner_core::companion::{NeedsYouReason, SessionStatus};
    for (status, reason, kind) in [
        (
            Status::NeedsYou,
            Some(NeedsYouReason::Approval),
            StatusKind::Approval,
        ),
        (
            Status::NeedsYou,
            Some(NeedsYouReason::Answer),
            StatusKind::Answer,
        ),
        (
            Status::NeedsYou,
            Some(NeedsYouReason::Unknown),
            StatusKind::NeedsYou,
        ),
        (Status::NeedsYou, None, StatusKind::NeedsYou),
        (
            Status::NeedsYou,
            Some(NeedsYouReason::Unrecognized("future".into())),
            StatusKind::NeedsYou,
        ),
        (
            Status::Unrecognized("future".into()),
            None,
            StatusKind::Idle,
        ),
        (Status::Working, None, StatusKind::Working),
        (Status::Idle, None, StatusKind::Idle),
        (Status::Starting, None, StatusKind::Starting),
        (Status::Stopped, None, StatusKind::Stopped),
        (Status::Error, None, StatusKind::Error),
    ] {
        for unread in [false, true] {
            let wire = SessionStatus {
                status: status.clone(),
                reason: reason.clone(),
                unread,
            };
            let mapped = model::agent_status(&wire);
            let presentation = StatusPresentation::new(&mapped);
            assert_eq!(presentation.kind, kind);
            assert!(!presentation.estimated);
            let back = SessionStatus::from(&mapped);
            assert_eq!(back.status, wire.status.effective());
            assert_eq!(back.unread, unread);
            assert_eq!(
                back.reason,
                (status == Status::NeedsYou).then_some(
                    reason
                        .clone()
                        .unwrap_or(NeedsYouReason::Unknown)
                        .effective()
                )
            );
        }
    }
    let mut rollup = StatusRollup {
        entries: [
            ("unread", Status::Idle, None, true),
            ("work", Status::Working, None, false),
            (
                "approval",
                Status::NeedsYou,
                Some(NeedsYouReason::Approval),
                false,
            ),
            (
                "answer",
                Status::NeedsYou,
                Some(NeedsYouReason::Answer),
                false,
            ),
            ("error", Status::Error, None, false),
        ]
        .into_iter()
        .map(|(id, status, reason, unread)| {
            (
                id.into(),
                model::agent_status(&SessionStatus {
                    status,
                    reason,
                    unread,
                }),
            )
        })
        .collect(),
    };
    assert_eq!(rollup.target(), Some("error"));
    assert_eq!(
        rollup.tooltip(),
        "1 error · 1 approval needed · 1 answer needed · 1 working · 1 unread response"
    );
    rollup.entries.pop();
    assert_eq!(rollup.target(), Some("approval"));
    rollup.entries.drain(2..);
    assert_eq!(rollup.target(), Some("work"));
    rollup.entries.pop();
    assert_eq!(rollup.target(), Some("unread"));
}

#[test]
fn remote_sidebar_reuses_local_status_indicators_and_tracks_pushes() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for mission in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        let mut data = overview();
        if mission {
            data.tabs.clear();
            data.missions[0]
                .members
                .push(runner_core::companion::Member {
                    handle: "coder".into(),
                    session: "s".into(),
                    lead: true,
                });
        } else {
            data.missions.clear();
        }
        fake.set_overview(data.clone());
        let window = add_window(&mut cx, &store, temp.path(), "shared-sidebar-status");
        window
            .update(&mut cx, |root, window, cx| {
                root.choose_computer(computers()[1].clone(), false, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        for (status, reason, unread, glyph) in [
            ("needsYou", Some("approval"), true, Some("Approval")),
            ("needsYou", Some("answer"), false, Some("Answer")),
            ("needsYou", None, false, Some("NeedsYou")),
            ("needsYou", Some("future"), false, Some("NeedsYou")),
            ("future", None, false, None),
            ("working", None, true, Some("Working")),
            ("idle", None, true, Some("Unread")),
            ("error", None, true, Some("Error")),
            ("starting", None, false, None),
            ("stopped", None, false, None),
            ("idle", None, false, None),
        ] {
            data.sessions.get_mut("s").unwrap().status = serde_json::from_value(
                serde_json::json!({"status": status, "reason": reason, "unread": unread}),
            )
            .unwrap();
            if status == "working" || reason == Some("answer") {
                fake.emit(
                    "remote/session-status",
                    serde_json::json!({"computer":"host","status":{"session":"s","status":status,"reason":reason,"unread":unread}}),
                );
            } else {
                fake.emit(
                    "remote/overview",
                    serde_json::json!({"computer":"host","overview":data}),
                );
            }
            visual.run_until_parked();
            if mission {
                window
                    .update(&mut visual, |root, _, cx| {
                        root.remote_window.as_mut().unwrap().active = None;
                        cx.notify();
                    })
                    .unwrap();
                visual.run_until_parked();
            }
            let row = visual.debug_bounds("REMOTE_SIDEBAR_ROW").unwrap();
            for (candidate, selector) in [
                ("Approval", "STATUS_ROLLUP:Approval"),
                ("Answer", "STATUS_ROLLUP:Answer"),
                ("NeedsYou", "STATUS_ROLLUP:NeedsYou"),
                ("Working", "STATUS_ROLLUP:Working"),
                ("Unread", "STATUS_ROLLUP:Unread"),
                ("Error", "STATUS_ROLLUP:Error"),
            ] {
                let bounds = visual.debug_bounds(selector);
                assert_eq!(bounds.is_some(), glyph == Some(candidate), "{status}");
                if let Some(bounds) = bounds {
                    assert!(bounds.top() >= row.top() && bounds.bottom() <= row.bottom());
                }
            }
            if glyph == Some("Unread") {
                let dot = visual.debug_bounds("STATUS_ROLLUP_UNREAD_DOT").unwrap();
                assert_eq!(dot.size, size(px(6.), px(6.)));
            }
        }
        if mission {
            data.missions[0].asks.push(runner_core::companion::Ask {
                question: "q".into(),
                from: "coder".into(),
                prompt: "Continue?".into(),
                choices: None,
            });
            fake.emit(
                "remote/overview",
                serde_json::json!({"computer":"host","overview":data}),
            );
            visual.run_until_parked();
            assert!(visual.debug_bounds("STATUS_ROLLUP:Answer").is_some());
        }
        assert!(fake.enrolls.load(Ordering::SeqCst) == 0);
        assert!(fake.input.lock().unwrap().is_empty());
        assert!(fake.takes.lock().unwrap().is_empty());
    }
}

#[test]
fn remote_sidebar_groups_pins_projects_and_recents_and_collapses_project_children() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    let mut data = sidebar_overview();
    fake.set_overview(data.clone());
    let window = add_window(&mut cx, &store, temp.path(), "sidebar-structure");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let local_settings = store.read_with(&visual, |store, _| {
        (
            store.settings.sidebar_projects_open,
            store.settings.sidebar_chats_open,
            store.settings.sidebar_collapsed_projects.clone(),
        )
    });
    let pinned = visual.debug_bounds("SIDEBAR_SECTION:PINNED").unwrap();
    let projects = visual.debug_bounds("SIDEBAR_SECTION:PROJECTS").unwrap();
    let recents = visual.debug_bounds("SIDEBAR_SECTION:RECENTS").unwrap();
    assert!(pinned.top() < projects.top() && projects.top() < recents.top());
    for selector in [
        "SIDEBAR_ITEM_TITLE:Remote tab",
        "SIDEBAR_ITEM_TITLE:Pinned mission",
    ] {
        let row = visual.debug_bounds(selector).unwrap();
        assert!(row.top() > pinned.bottom() && row.bottom() < projects.top());
    }
    let project = visual.debug_bounds("SIDEBAR_PROJECT:p").unwrap();
    let child = visual
        .debug_bounds("SIDEBAR_ITEM_TITLE:Project tab")
        .unwrap();
    let recent = visual
        .debug_bounds("SIDEBAR_ITEM_TITLE:Recent tab")
        .unwrap();
    assert!(child.top() > project.bottom() && child.bottom() < recents.top());
    assert!(child.left() > recent.left());
    assert!(recent.top() > recents.bottom());
    assert!(
        visual
            .debug_bounds("SIDEBAR_ITEM_TITLE:Remote mission")
            .unwrap()
            .top()
            > recents.bottom()
    );
    assert!(visual.debug_bounds("SIDEBAR_SECTION:MISSIONS").is_none());
    visual.simulate_click(project.center(), Default::default());
    visual.run_until_parked();
    assert!(visual
        .debug_bounds("SIDEBAR_ITEM_TITLE:Project tab")
        .is_none());
    assert!(visual
        .debug_bounds("SIDEBAR_ITEM_TITLE:Project mission")
        .is_none());
    assert!(visual
        .debug_bounds("SIDEBAR_ITEM_TITLE:Remote tab")
        .is_some());
    visual.simulate_click(project.center(), Default::default());
    visual.run_until_parked();
    let child = visual
        .debug_bounds("SIDEBAR_ITEM_TITLE:Project tab")
        .unwrap();
    visual.simulate_click(child.center(), Default::default());
    visual.run_until_parked();
    assert!(visual
        .debug_bounds("WORKSPACE_HEADER_TITLE:Project tab")
        .is_some());
    data.tabs[1].pinned = true;
    fake.emit(
        "remote/overview",
        serde_json::json!({"computer":"host","overview":data}),
    );
    visual.run_until_parked();
    let moved = visual
        .debug_bounds("SIDEBAR_ITEM_TITLE:Project tab")
        .unwrap();
    let projects = visual.debug_bounds("SIDEBAR_SECTION:PROJECTS").unwrap();
    assert!(moved.bottom() < projects.top());
    visual.simulate_click(
        point(projects.left() + px(24.), projects.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("SIDEBAR_PROJECT:p").is_none());
    assert!(visual
        .debug_bounds("SIDEBAR_ITEM_TITLE:Remote tab")
        .is_some());
    visual.simulate_click(
        point(projects.left() + px(24.), projects.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("SIDEBAR_PROJECT:p").is_some());
    let recents = visual.debug_bounds("SIDEBAR_SECTION:RECENTS").unwrap();
    visual.simulate_click(
        point(recents.left() + px(24.), recents.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(visual
        .debug_bounds("SIDEBAR_ITEM_TITLE:Recent tab")
        .is_none());
    assert_eq!(
        store.read_with(&visual, |store, _| (
            store.settings.sidebar_projects_open,
            store.settings.sidebar_chats_open,
            store.settings.sidebar_collapsed_projects.clone()
        )),
        local_settings
    );
}

#[test]
fn remote_sidebar_uses_session_titles_for_unnamed_tabs_and_tracks_overview_updates() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, _) = fixture(&mut cx, temp.path());
            let mut data = overview();
            data.tabs[0].name.clear();
            data.sessions.get_mut("s").unwrap().title = "Project chat title".into();
            let mut recent = data.tabs[0].clone();
            recent.id = "recent".into();
            recent.project = None;
            recent.panes = runner_core::companion::Pane::Leaf {
                session: Some("recent-session".into()),
            };
            let mut session = data.sessions["s"].clone();
            session.title = "Recent chat title".into();
            data.sessions.insert("recent-session".into(), session);
            data.tabs.push(recent);
            fake.set_overview(data.clone());
            let window = add_window(&mut cx, &store, temp.path(), "remote-titles");
            window
                .update(&mut cx, |root, window, cx| {
                    root.choose_computer(computers()[1].clone(), false, window, cx);
                })
                .unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            for selector in [
                "SIDEBAR_ITEM_TITLE:Project chat title",
                "SIDEBAR_ITEM_TITLE:Recent chat title",
                "WORKSPACE_HEADER_TITLE:Project chat title",
            ] {
                let bounds = visual.debug_bounds(selector).expect(selector);
                assert!(bounds.size.width > px(0.) && bounds.size.height > px(0.));
            }
            data.sessions.get_mut("s").unwrap().title = "Updated chat title".into();
            fake.emit(
                "remote/overview",
                serde_json::json!({"computer":"host","overview":data}),
            );
            visual.run_until_parked();
            assert!(visual
                .debug_bounds("SIDEBAR_ITEM_TITLE:Project chat title")
                .is_none());
            assert!(visual
                .debug_bounds("SIDEBAR_ITEM_TITLE:Updated chat title")
                .is_some());
            assert!(visual
                .debug_bounds("WORKSPACE_HEADER_TITLE:Updated chat title")
                .is_some());
            data.tabs[0].name = "Named tab".into();
            fake.emit(
                "remote/overview",
                serde_json::json!({"computer":"host","overview":data}),
            );
            visual.run_until_parked();
            assert!(visual
                .debug_bounds("SIDEBAR_ITEM_TITLE:Named tab")
                .is_some());
            assert!(visual
                .debug_bounds("WORKSPACE_HEADER_TITLE:Named tab")
                .is_some());
            data.tabs[0].name.clear();
            data.tabs[0].panes = runner_core::companion::Pane::Split {
                split: runner_core::companion::SplitDirection::Row,
                sizes: [0.5, 0.5],
                a: Box::new(runner_core::companion::Pane::Leaf {
                    session: Some("recent-session".into()),
                }),
                b: Box::new(runner_core::companion::Pane::Leaf {
                    session: Some("s".into()),
                }),
            };
            fake.emit(
                "remote/overview",
                serde_json::json!({"computer":"host","overview":data}),
            );
            visual.run_until_parked();
            assert!(visual
                .debug_bounds("WORKSPACE_HEADER_TITLE:Recent chat title")
                .is_some());
            data.tabs[0].panes = runner_core::companion::Pane::Leaf { session: None };
            fake.emit(
                "remote/overview",
                serde_json::json!({"computer":"host","overview":data}),
            );
            visual.run_until_parked();
            let empty = match locale {
                Locale::English => "WORKSPACE_HEADER_TITLE:Empty tab",
                Locale::SimplifiedChinese => "WORKSPACE_HEADER_TITLE:空标签页",
            };
            assert!(visual.debug_bounds("START_SURFACE").is_some());
            window
                .read_with(&visual, |root, cx| {
                    assert_eq!(
                        root.tab_label(root.front_page_layout().unwrap(), cx),
                        empty.trim_start_matches("WORKSPACE_HEADER_TITLE:")
                    )
                })
                .unwrap();
            data.tabs[0].panes = runner_core::companion::Pane::Leaf {
                session: Some("s".into()),
            };
            data.sessions.get_mut("s").unwrap().title.clear();
            fake.emit(
                "remote/overview",
                serde_json::json!({"computer":"host","overview":data}),
            );
            visual.run_until_parked();
            assert!(visual.debug_bounds("SIDEBAR_ITEM_TITLE:Codex").is_some());
            assert!(visual
                .debug_bounds("WORKSPACE_HEADER_TITLE:Codex")
                .is_some());
        });
    }
}

#[test]
fn disabling_remote_access_returns_every_window_to_this_computer() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    let first = add_window(&mut cx, &store, temp.path(), "remote-disable-first");
    let second = add_window(&mut cx, &store, temp.path(), "remote-disable-second");
    for window in [first, second] {
        window
            .update(&mut cx, |root, window, cx| {
                root.visit_computer(computers()[1].clone(), window, cx)
            })
            .unwrap();
    }
    cx.run_until_parked();
    for window in [first, second] {
        window
            .read_with(&cx, |root, _| assert!(root.remote_window.is_some()))
            .unwrap();
    }
    *fake.status.lock().unwrap() = RemoteStatus::default();
    store.update(&mut cx, |store, cx| {
        store.remote_status = Some(RemoteStatus::default());
        store.update_settings(
            |settings| {
                settings.local_only = true;
                true
            },
            false,
            cx,
        );
    });
    cx.run_until_parked();
    for window in [first, second] {
        window
            .read_with(&cx, |root, _| {
                assert!(root.remote_window.is_none());
                assert!(!root.client_ui.picker_open);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        assert!(visual.debug_bounds("COMPUTER_PICKER_TRIGGER").is_none());
        assert!(visual.debug_bounds("SIDEBAR_SETTINGS").is_some());
    }
    assert!(fake
        .follows
        .lock()
        .unwrap()
        .contains(&("host".into(), false)));
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
                assert!(visual.debug_bounds("REMOTE_DISABLED_CREATE").is_none());
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
fn picker_status_precedes_unpaired_detail_from_directory() {
    for (locale, online, offline) in [
        (
            Locale::English,
            "Online · Not paired",
            "Offline · seen 1 hour ago · Not paired",
        ),
        (
            Locale::SimplifiedChinese,
            "在线 · 未配对",
            "离线 · 最后在线 1 小时前 · 未配对",
        ),
    ] {
        with_locale(locale, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (_store, fake, clients) = fixture(&mut cx, temp.path());
            let remote = computers().into_iter().find(|c| !c.is_self).unwrap();
            *fake.computers.lock().unwrap() = vec![
                Computer {
                    id: "unpaired-online".into(),
                    online: true,
                    paired: false,
                    ..remote.clone()
                },
                Computer {
                    id: "unpaired-offline".into(),
                    online: false,
                    paired: false,
                    last_seen_at: Some(
                        (chrono::Utc::now() - chrono::Duration::minutes(90)).to_rfc3339(),
                    ),
                    ..remote
                },
            ];
            let before = fake.directory_calls.load(Ordering::SeqCst);
            clients.update(&mut cx, |clients, cx| clients.refresh(cx));
            cx.run_until_parked();
            assert_eq!(fake.directory_calls.load(Ordering::SeqCst), before + 1);
            let details = clients.read_with(&cx, |clients, _| {
                clients
                    .computers
                    .iter()
                    .map(picker::computer_detail)
                    .collect::<Vec<_>>()
            });
            assert_eq!(details, [online, offline]);
        });
    }
}

#[test]
fn picker_encloses_rows_errors_and_hint_in_an_opaque_panel() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            cx.update(|cx| {
                cx.set_global(runner_app::appearance::Appearance {
                    reduce_transparency: false,
                    ..Default::default()
                });
            });
            let (store, fake, clients) = fixture(&mut cx, temp.path());
            let window = add_window(&mut cx, &store, temp.path(), "picker-panel");
            for failed in [false, true] {
                fake.fail_directory.store(failed, Ordering::SeqCst);
                clients.update(&mut cx, |clients, cx| clients.refresh(cx));
                cx.run_until_parked();
                assert_eq!(
                    clients.read_with(&cx, |clients, _| clients.directory_error),
                    failed
                );
                for variant in [
                    theme::ThemeVariant::Carbon,
                    theme::ThemeVariant::RunnerLight,
                ] {
                    for glass in [false, true] {
                        for zoom in [1., 1.5] {
                            store.update(&mut cx, |store, _| {
                                store.settings.app_zoom = zoom;
                                store.settings.app_theme = if variant.is_light() {
                                    theme::ThemeIntent::Light
                                } else {
                                    theme::ThemeIntent::Dark
                                };
                                store.settings.light_app_theme = theme::LightTheme::RunnerLight;
                                store.settings.dark_app_theme = theme::DarkTheme::Runner;
                                store.settings.window_material = if glass {
                                    runner_app::appearance::WindowMaterial::Glass
                                } else {
                                    runner_app::appearance::WindowMaterial::Solid
                                };
                            });
                            window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
                            cx.run_until_parked();
                            let mut visual = VisualTestContext::from_window(window.into(), &cx);
                            let trigger = visual.debug_bounds("COMPUTER_PICKER_TRIGGER").unwrap();
                            visual.simulate_click(trigger.center(), Default::default());
                            visual.run_until_parked();
                            assert_eq!(theme::active_variant(), variant);
                            assert_eq!(theme::is_glass(), glass && cfg!(target_os = "macos"));
                            crate::theme_snapshot::assert_fill(
                                &mut visual,
                                "COMPUTER_PICKER_PANEL",
                                theme::colors().panel,
                            );
                            let panel = visual.debug_bounds("COMPUTER_PICKER_PANEL").unwrap();
                            assert_eq!(panel.size.width, px(320. * zoom));
                            assert!(panel.top() > trigger.bottom());
                            assert!(visual.debug_bounds("COMPUTER_PICKER_HINT").is_none());
                            let content = visual.debug_bounds("COMPUTER_PICKER_ROW").unwrap();
                            assert!(panel.contains(&content.origin));
                            assert!(panel.contains(&content.bottom_right()));
                            let error = visual.debug_bounds("COMPUTER_PICKER_ERROR");
                            assert_eq!(error.is_some(), failed);
                            if let Some(error) = error {
                                assert!(panel.contains(&error.origin));
                                assert!(panel.contains(&error.bottom_right()));
                            }
                            visual.simulate_keystrokes("escape");
                            visual.run_until_parked();
                            assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
                            visual.simulate_click(trigger.center(), Default::default());
                            visual.run_until_parked();
                            assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_some());
                            visual.simulate_click(trigger.center(), Default::default());
                            visual.run_until_parked();
                            assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
                            visual.simulate_click(trigger.center(), Default::default());
                            visual.run_until_parked();
                            assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_some());
                            visual.simulate_click(point(px(1400.), px(850.)), Default::default());
                            visual.run_until_parked();
                            assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
                        }
                    }
                }
            }
        });
    }
}

#[test]
fn signed_in_picker_shows_attention_dots_and_never_counts() {
    use runner_core::protocol::status::{
        AgentObservation, AgentStatus, HumanInteraction, WaitReason,
    };
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    let window = add_window(&mut cx, &store, temp.path(), "no-picker-counters");
    for remote in [false, true] {
        if remote {
            window
                .update(&mut cx, |root, window, cx| {
                    root.choose_computer(computers()[1].clone(), false, window, cx);
                })
                .unwrap();
        }
        cx.run_until_parked();
        store.update(&mut cx, |store, cx| {
            store.session_statuses.insert(
                "local-attention".into(),
                AgentStatus {
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
            cx.notify();
        });
        clients.update(&mut cx, |clients, cx| {
            clients.overviews.insert("host".into(), overview());
            cx.notify();
        });
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.run_until_parked();
        assert_eq!(fake.overview_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            visual.debug_bounds("COMPUTER_TRIGGER_ATTENTION").is_some(),
            !remote
        );
        let trigger = visual.debug_bounds("COMPUTER_PICKER_TRIGGER").unwrap();
        assert!(visual
            .debug_bounds("COMPUTER_PICKER_TRIGGER_BADGE")
            .is_none());
        visual.simulate_click(trigger.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("COMPUTER_PICKER_REMOTE_ROW").is_some());
        assert_eq!(fake.overview_calls.load(Ordering::SeqCst), 1);
        assert!(visual.debug_bounds("COMPUTER_PICKER_ROW_BADGE").is_none());
        let dot = visual.debug_bounds("COMPUTER_ROW_ATTENTION").unwrap();
        assert_eq!(dot.size, size(px(7.), px(7.)));
        fake.emit("remote/session-status", serde_json::json!({"computer":"host","status":{"session":"s","status":"idle","unread":true}}));
        visual.run_until_parked();
        assert!(visual.debug_bounds("COMPUTER_ROW_ATTENTION").is_none());
        assert!(visual.debug_bounds("COMPUTER_TRIGGER_ATTENTION").is_none());
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
    }
}

#[test]
fn signed_out_has_no_picker_even_with_cached_directory_and_opens_account_from_avatar() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            for remote_state in [
                None,
                Some(RemoteState::Off),
                Some(RemoteState::Stopped {
                    reason: StopReason::Removed,
                }),
            ] {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let (store, fake, clients) = fixture(&mut cx, temp.path());
                let status = RemoteStatus {
                    state: remote_state.clone().unwrap_or_default(),
                    host_id: Some("old-host".into()),
                    account: Some(RemoteAccount {
                        username: "old-account".into(),
                        display_name: "Old account".into(),
                    }),
                    ..Default::default()
                };
                *fake.status.lock().unwrap() = status.clone();
                store.update(&mut cx, |store, _| {
                    store.remote_status = remote_state.as_ref().map(|_| status.clone())
                });
                let calls = fake.directory_calls.load(Ordering::SeqCst);
                let window = add_window(&mut cx, &store, temp.path(), "signed-out-no-picker");
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                visual.run_until_parked();
                clients.update(&mut visual, |clients, cx| {
                    clients.directory_error = true;
                    clients.action_failed = true;
                    cx.notify();
                });
                visual.run_until_parked();
                assert!(visual.debug_bounds("COMPUTER_PICKER_TRIGGER").is_none());
                assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
                assert!(visual.debug_bounds("COMPUTER_PICKER_ROW").is_none());
                assert!(visual.debug_bounds("COMPUTER_PICKER_SIGN_IN").is_none());
                assert_eq!(fake.directory_calls.load(Ordering::SeqCst), calls);
                let avatar = visual.debug_bounds("SIDEBAR_ACCOUNT").unwrap();
                visual.simulate_click(avatar.center(), Default::default());
                visual.run_until_parked();
                let sign_in = visual.debug_bounds("ACCOUNT_MENU_ITEM_1").unwrap();
                visual.simulate_click(sign_in.center(), Default::default());
                visual.run_until_parked();
                assert!(visual.debug_bounds("ACCOUNT_STATUS_CARD").is_some());
                assert_eq!(fake.enrolls.load(Ordering::SeqCst), 0);
            }
        });
    }
}

#[test]
fn settings_computers_refresh_retries_failures_and_disables_while_loading() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, clients) = fixture(&mut cx, temp.path());
            let window = add_window(&mut cx, &store, temp.path(), "computers-refresh");
            window
                .update(&mut cx, |root, _, cx| {
                    root.route = AppRoute::Settings;
                    root.settings_page.pane = SettingsPane::Remote;
                    cx.notify();
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            visual.run_until_parked();
            let refresh = visual.debug_bounds("REMOTE_COMPUTERS_REFRESH").unwrap();
            let initial = clients.read_with(&visual, |clients, _| {
                clients
                    .computers
                    .iter()
                    .map(|computer| computer.id.clone())
                    .collect::<Vec<_>>()
            });
            let calls = fake.directory_calls.load(Ordering::SeqCst);

            clients.update(&mut visual, |clients, cx| {
                clients.loading = true;
                cx.notify();
            });
            visual.run_until_parked();
            visual.simulate_click(refresh.center(), Default::default());
            visual.run_until_parked();
            assert_eq!(fake.directory_calls.load(Ordering::SeqCst), calls);

            clients.update(&mut visual, |clients, cx| {
                clients.loading = false;
                cx.notify();
            });
            fake.fail_directory.store(true, Ordering::SeqCst);
            visual.run_until_parked();
            visual.simulate_click(refresh.center(), Default::default());
            visual.run_until_parked();
            assert_eq!(fake.directory_calls.load(Ordering::SeqCst), calls + 1);
            assert!(visual.debug_bounds("REMOTE_COMPUTERS_ERROR").is_some());
            clients.read_with(&visual, |clients, _| {
                assert_eq!(
                    clients
                        .computers
                        .iter()
                        .map(|computer| computer.id.clone())
                        .collect::<Vec<_>>(),
                    initial
                );
                assert!(!clients.loading);
            });

            fake.fail_directory.store(false, Ordering::SeqCst);
            fake.computers.lock().unwrap().push(Computer {
                id: "new-computer".into(),
                name: "New computer".into(),
                ..Default::default()
            });
            visual.simulate_click(refresh.center(), Default::default());
            visual.run_until_parked();
            assert_eq!(fake.directory_calls.load(Ordering::SeqCst), calls + 2);
            assert!(visual.debug_bounds("REMOTE_COMPUTERS_ERROR").is_none());
            clients.read_with(&visual, |clients, _| {
                assert_eq!(clients.computers.len(), initial.len() + 1);
                assert!(!clients.loading);
            });
            window
                .read_with(&visual, |root, _| {
                    assert_eq!(root.route, AppRoute::Settings);
                    assert_eq!(root.settings_page.pane, SettingsPane::Remote);
                })
                .unwrap();
        });
    }
}

#[test]
fn account_sign_in_resumes_directory_refresh_and_signed_in_failures_remain_errors() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (_, fake, clients) = fixture(&mut cx, temp.path());
    for remote_state in [
        RemoteState::Paused,
        RemoteState::Connecting { attempt: 1 },
        RemoteState::Connected {
            since: chrono::Utc::now(),
        },
        RemoteState::Reconnecting {
            attempt: 1,
            next_retry_at: chrono::Utc::now(),
        },
    ] {
        let off = RemoteStatus::default();
        *fake.status.lock().unwrap() = off.clone();
        fake.emit("remote/status", serde_json::to_value(off).unwrap());
        cx.run_until_parked();
        let calls = fake.directory_calls.load(Ordering::SeqCst);
        clients.update(&mut cx, |clients, cx| clients.refresh(cx));
        fake.emit("daemon/reconnected", serde_json::json!({}));
        cx.run_until_parked();
        assert_eq!(fake.directory_calls.load(Ordering::SeqCst), calls);
        clients.read_with(&cx, |clients, _| assert!(clients.computers.is_empty()));

        let status = RemoteStatus {
            state: remote_state,
            ..Default::default()
        };
        *fake.status.lock().unwrap() = status.clone();
        fake.fail_directory.store(false, Ordering::SeqCst);
        fake.emit("remote/status", serde_json::to_value(status).unwrap());
        cx.run_until_parked();
        assert_eq!(fake.directory_calls.load(Ordering::SeqCst), calls + 1);
        clients.read_with(&cx, |clients, _| {
            assert_eq!(clients.computers.len(), 2);
            assert!(!clients.directory_error);
        });
        fake.fail_directory.store(true, Ordering::SeqCst);
        clients.update(&mut cx, |clients, cx| clients.refresh(cx));
        cx.run_until_parked();
        clients.read_with(&cx, |clients, _| assert!(clients.directory_error));
    }
}

#[test]
fn picker_dismisses_at_tab_boundaries_and_escape_works_after_internal_traversal() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for signed in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        if !signed {
            *fake.status.lock().unwrap() = RemoteStatus::default();
            store.update(&mut cx, |store, _| {
                store.remote_status = Some(RemoteStatus::default())
            });
        }
        let window = add_window(&mut cx, &store, temp.path(), "picker-focus");
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.run_until_parked();
        window
            .update(&mut visual, |root, _, cx| {
                root.route = AppRoute::Chat;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        if !signed {
            assert!(visual.debug_bounds("COMPUTER_PICKER_TRIGGER").is_none());
            assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
            continue;
        }
        let trigger = visual.debug_bounds("COMPUTER_PICKER_TRIGGER").unwrap();
        visual.simulate_click(trigger.center(), Default::default());
        visual.run_until_parked();
        visual.simulate_keystrokes("tab escape");
        visual.run_until_parked();
        assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
        for direction in ["tab", "shift-tab"] {
            visual.simulate_click(trigger.center(), Default::default());
            visual.run_until_parked();
            let mut dismissed = false;
            for _ in 0..16 {
                visual.simulate_keystrokes(direction);
                visual.run_until_parked();
                dismissed = window
                    .update(&mut visual, |root, window, cx| {
                        if root.client_ui.picker_open {
                            assert!(
                                root.client_ui.picker_focus.contains_focused(window, cx),
                                "focus left an open computer picker"
                            );
                            false
                        } else {
                            assert!(root.root_focus.is_focused(window));
                            true
                        }
                    })
                    .unwrap();
                if dismissed {
                    break;
                }
            }
            assert!(dismissed, "Tab traversal did not reach the picker boundary");
            visual.simulate_keystrokes("escape enter space");
            visual.run_until_parked();
            assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
            assert!(visual.debug_bounds("ACCOUNT_STATUS_CARD").is_none());
            window
                .read_with(&visual, |root, _| assert_eq!(root.route, AppRoute::Chat))
                .unwrap();
            assert_eq!(fake.enrolls.load(Ordering::SeqCst), 0);
            assert_eq!(fake.pairs.load(Ordering::SeqCst), 0);
        }
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
fn paused_remote_chats_and_shells_wait_for_a_running_push_before_attaching() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            for shell in [false, true] {
                for status in [Status::Stopped, Status::Error] {
                    let temp = tempfile::tempdir().unwrap();
                    let mut cx = TestAppContext::single();
                    let (store, fake, clients) = fixture(&mut cx, temp.path());
                    enable_front_page(&fake);
                    let mut data = overview();
                    let session = data.sessions.get_mut("s").unwrap();
                    session.status.status = status.clone();
                    if shell {
                        session.kind = runner_core::companion::SessionKind::Shell;
                        session.runtime = None;
                    }
                    fake.set_overview(data);
                    cx.run_until_parked();
                    clients.read_with(&cx, |clients, _| {
                        assert_eq!(
                            clients.overviews["host"].sessions["s"].status.status,
                            status
                        );
                    });
                    fake.fail_attach.store(true, Ordering::SeqCst);
                    let window = add_window(&mut cx, &store, temp.path(), "paused-remote");
                    window.update(&mut cx, choose_host).unwrap();
                    cx.run_until_parked();
                    let mut visual = VisualTestContext::from_window(window.into(), &cx);
                    assert!(visual.debug_bounds("SESSION_ENDED_CARD").is_some());
                    assert!(visual.debug_bounds("REMOTE_TERMINAL_FAILED").is_none());
                    assert!(fake.attaches.lock().unwrap().is_empty());
                    let size = window
                        .read_with(&visual, |root, _| root.remote_terminal_size("s"))
                        .unwrap()
                        .unwrap();
                    assert!(size.0 >= 20 && size.1 >= 8);
                    window
                        .read_with(&visual, |root, cx| {
                            assert!(root.front_page_client(cx).supports(&["resume"]));
                            assert!(matches!(
                                root.remote_window.as_ref().unwrap().connection,
                                Connection::Connected
                            ));
                        })
                        .unwrap();
                    let selector = if shell {
                        "BUTTON:ended-s-restart"
                    } else {
                        "BUTTON:ended-s-resume"
                    };
                    let button = visual
                        .debug_bounds(selector)
                        .expect("visible ended primary button");
                    assert!(button.size.width > px(0.) && button.size.height > px(0.));
                    let position = button.center();
                    visual.simulate_mouse_down(position, MouseButton::Left, Default::default());
                    visual.run_until_parked();
                    let pressed = visual
                        .debug_bounds(selector)
                        .expect("ended primary button after press");
                    let position = pressed.center();
                    let size = window
                        .read_with(&visual, |root, _| root.remote_terminal_size("s"))
                        .unwrap()
                        .unwrap();
                    assert!(size.0 >= 20 && size.1 >= 8);
                    visual.simulate_mouse_up(position, MouseButton::Left, Default::default());
                    visual.run_until_parked();
                    let resumes = action_trace(&fake, "resume");
                    assert_eq!(resumes.len(), 1, "shell={shell} status={status:?} before={button:?} pressed={pressed:?} size={size:?}");
                    assert_eq!(resumes[0]["session"], "s");
                    assert_eq!(resumes[0]["cols"], size.0);
                    assert_eq!(resumes[0]["rows"], size.1);
                    assert!(fake.attaches.lock().unwrap().is_empty());
                    assert!(visual.debug_bounds("SESSION_ENDED_CARD").is_some());
                    fake.fail_attach.store(false, Ordering::SeqCst);
                    fake.emit("remote/session-status", serde_json::json!({"computer":"host","status":{"session":"s","status":"working","unread":false}}));
                    visual.run_until_parked();
                    assert_eq!(fake.attaches.lock().unwrap().len(), 1);
                    assert!(visual.debug_bounds("SESSION_ENDED_CARD").is_none());
                    assert!(visual.debug_bounds("REMOTE_TERMINAL_FAILED").is_none());
                    let terminal = window
                        .read_with(&visual, |root, _| {
                            root.remote_window.as_ref().unwrap().terminals["s"]
                                .terminal
                                .clone()
                        })
                        .unwrap();
                    fake.emit("remote/session-status", serde_json::json!({"computer":"host","status":{"session":"s","status":"stopped","unread":false}}));
                    visual.run_until_parked();
                    assert!(visual.debug_bounds("SESSION_ENDED_CARD").is_some());
                    assert!(visual.debug_bounds("REMOTE_TERMINAL_FAILED").is_none());
                    assert_eq!(fake.attaches.lock().unwrap().len(), 1);
                    window
                        .read_with(&visual, |root, cx| {
                            assert!(Arc::ptr_eq(
                                &terminal,
                                &root.remote_window.as_ref().unwrap().terminals["s"].terminal
                            ));
                            assert!(!root.remote_terminal_interactive("s", cx));
                        })
                        .unwrap();
                }
            }
        });
    }
}

#[test]
fn obsolete_remote_attach_results_cannot_replace_a_resumed_terminal() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for first_fails in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, clients) = fixture(&mut cx, temp.path());
        let mut data = overview();
        data.sessions.get_mut("s").unwrap().status.status = Status::Stopped;
        fake.set_overview(data);
        cx.run_until_parked();
        let window = add_window(&mut cx, &store, temp.path(), "paused-attach-race");
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        assert!(fake.attaches.lock().unwrap().is_empty());
        fake.fail_attach.store(first_fails, Ordering::SeqCst);
        *fake.snapshot.lock().unwrap() = b"previous terminal".to_vec();
        clients.update(&mut cx, |clients, _| {
            clients
                .overviews
                .get_mut("host")
                .unwrap()
                .sessions
                .get_mut("s")
                .unwrap()
                .status
                .status = Status::Working;
        });
        window
            .update(&mut cx, |root, _, cx| root.sync_remote_state(cx))
            .unwrap();
        for _ in 0..1000 {
            if fake.attaches.lock().unwrap().len() == 1 {
                break;
            }
            assert!(cx.background_executor.tick());
        }
        assert_eq!(fake.attaches.lock().unwrap().len(), 1);
        window
            .read_with(&cx, |root, _| {
                assert!(root.remote_window.as_ref().unwrap().terminals.is_empty())
            })
            .unwrap();
        clients.update(&mut cx, |clients, _| {
            clients
                .overviews
                .get_mut("host")
                .unwrap()
                .sessions
                .get_mut("s")
                .unwrap()
                .status
                .status = Status::Stopped;
        });
        window
            .update(&mut cx, |root, _, cx| root.sync_remote_state(cx))
            .unwrap();
        fake.fail_attach.store(false, Ordering::SeqCst);
        *fake.snapshot.lock().unwrap() = b"resumed terminal".to_vec();
        clients.update(&mut cx, |clients, _| {
            clients
                .overviews
                .get_mut("host")
                .unwrap()
                .sessions
                .get_mut("s")
                .unwrap()
                .status
                .status = Status::Working;
        });
        window
            .update(&mut cx, |root, _, cx| root.sync_remote_state(cx))
            .unwrap();
        for _ in 0..1000 {
            if fake.attaches.lock().unwrap().len() == 2 {
                break;
            }
            assert!(cx.background_executor.tick());
        }
        assert_eq!(fake.attaches.lock().unwrap().len(), 2);
        cx.run_until_parked();
        let terminal = window
            .read_with(&cx, |root, _| {
                root.remote_window.as_ref().unwrap().terminals["s"]
                    .terminal
                    .clone()
            })
            .unwrap();
        let text = terminal
            .term
            .lock()
            .grid()
            .display_iter()
            .map(|cell| cell.cell.c)
            .collect::<String>();
        assert!(text.contains("resumed terminal"));
        assert!(!text.contains("previous terminal"));
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        assert!(visual.debug_bounds("REMOTE_TERMINAL_FAILED").is_none());
        assert!(visual.debug_bounds("SESSION_ENDED_CARD").is_none());
    }
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
fn captured_load_reply_cannot_replace_newer_pushes_or_a_newer_request() {
    for push in [
        "remote/overview",
        "remote/connection",
        "remote/session-status",
        "remote/feed",
        "new-load",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (_, fake, clients) = fixture(&mut cx, temp.path());
        let (release, blocked) = mpsc::channel();
        fake.overview_gates
            .lock()
            .unwrap()
            .insert("host".into(), blocked);
        let (started, captured) = mpsc::channel();
        *fake.overview_started.lock().unwrap() = Some(started);
        let client = fake.client();
        let (updates, mut responses) = futures::channel::mpsc::unbounded();
        let worker = std::thread::spawn(move || model::load(&client, "host", &updates, &mut 0));
        captured
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let begin = futures::executor::block_on(responses.next()).unwrap();
        clients.update(&mut cx, |state, _| state.apply(begin));
        let mut newer = overview();
        newer.tabs[0].name = "Newer pushed tab".into();
        let session = newer.sessions.keys().next().unwrap().clone();
        clients.update(&mut cx, |state, _| {
            state.overviews.insert("host".into(), overview());
            state.connections.insert("host".into(), Connection::Reconnecting);
            if matches!(push, "remote/connection" | "new-load") {
                state.apply_event(ClientEvent { name:"remote/overview".into(), payload:serde_json::json!({"computer":"host","overview":newer}) });
            }
            let payload = match push {
                "remote/overview" => serde_json::json!({"computer":"host","overview":newer}),
                "remote/session-status" => serde_json::json!({"computer":"host","status":{"session":session,"status":"working","reason":null,"unread":true}}),
                _ => serde_json::json!({"computer":"host","state":"reconnecting"}),
            };
            if push == "new-load" { state.apply(model::Update::LoadStarted("host".into(), 2)); }
            else { state.apply_event(ClientEvent { name: push.into(), payload }); }
        });
        release.send(()).unwrap();
        worker.join().unwrap();
        futures::executor::block_on(async {
            while let Some(update) = responses.next().await {
                clients.update(&mut cx, |state, _| state.apply(update));
            }
        });
        clients.read_with(&cx, |state, _| {
            let pushed = matches!(push, "remote/overview" | "remote/connection" | "new-load");
            assert_eq!(
                state.overviews["host"].tabs[0].name,
                if pushed {
                    "Newer pushed tab".to_owned()
                } else {
                    overview().tabs[0].name.clone()
                }
            );
            assert!(
                state.connection("host")
                    == if matches!(push, "remote/connection" | "new-load") {
                        Connection::Reconnecting
                    } else {
                        Connection::Connected
                    }
            );
            if push == "remote/session-status" {
                assert!(state.overviews["host"].sessions[&session].status.unread);
            }
        });
    }
}

#[test]
fn input_error_events_render_for_current_remote_chat_mission_and_drawer_during_route_loss() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for surface in ["chat", "mission", "chat-drawer", "mission-drawer"] {
        for order in ["connected", "connection-first", "error-first"] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, _) = fixture(&mut cx, temp.path());
            let mut data = overview();
            if surface.starts_with("mission") {
                data.tabs.clear();
                data.missions[0]
                    .members
                    .push(runner_core::companion::Member {
                        handle: "coder".into(),
                        session: "s".into(),
                        lead: true,
                    });
            }
            let session = if surface.ends_with("drawer") {
                let mut shell = data.sessions["s"].clone();
                shell.kind = runner_core::companion::SessionKind::Shell;
                data.sessions.insert("drawer-shell".into(), shell);
                if surface.starts_with("mission") {
                    data.missions[0].shells.push("drawer-shell".into());
                } else {
                    data.tabs[0].shells.push("drawer-shell".into());
                }
                "drawer-shell"
            } else {
                "s"
            };
            fake.set_overview(data);
            let window = add_window(&mut cx, &store, temp.path(), surface);
            window
                .update(&mut cx, |root, window, cx| {
                    root.choose_computer(computers()[1].clone(), false, window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            if surface.starts_with("mission") {
                window
                    .update(&mut cx, |root, window, cx| {
                        root.remote_window
                            .as_ref()
                            .unwrap()
                            .mission_workspace
                            .update(cx, |workspace, cx| {
                                workspace.focus_status_session("remote-mission", "s", window, cx)
                            });
                        root.attach_remote_panes(cx);
                    })
                    .unwrap();
                cx.run_until_parked();
            }
            if surface.ends_with("drawer") {
                window
                    .update(&mut cx, |root, window, cx| {
                        root.toggle_terminal_drawer(window, cx)
                    })
                    .unwrap();
                cx.run_until_parked();
            }
            window
                .read_with(&cx, |root, _| {
                    assert!(root
                        .remote_window
                        .as_ref()
                        .unwrap()
                        .terminals
                        .contains_key(session))
                })
                .unwrap();
            if order == "connection-first" {
                fake.emit("remote/connection", serde_json::json!({"computer":"host","state":"reconnecting","error":"route lost"}));
                cx.run_until_parked();
            }
            fake.emit("session/input-error", serde_json::json!({"session_id":runner_core::remote::terminal_address("host",session),"message":"Paste was discarded"}));
            cx.run_until_parked();
            if order == "error-first" {
                fake.emit("remote/connection", serde_json::json!({"computer":"host","state":"disconnected","error":"route lost"}));
                cx.run_until_parked();
            }
            window
                .read_with(&cx, |root, _| {
                    assert_eq!(
                        root.remote_window.as_ref().unwrap().action_error.as_deref(),
                        Some("Paste was discarded")
                    )
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            assert!(
                visual.debug_bounds("REMOTE_ACTION_ERROR").is_some(),
                "{surface}/{order}"
            );
        }
    }
}

#[test]
fn remote_input_errors_and_late_write_callbacks_do_not_leak_to_other_targets() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    let window = add_window(&mut cx, &store, temp.path(), "input-isolation");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    for address in [
        "s".to_owned(),
        runner_core::remote::terminal_address("other", "s"),
        runner_core::remote::terminal_address("host", "hidden"),
    ] {
        fake.emit(
            "session/input-error",
            serde_json::json!({"session_id":address,"message":"wrong target"}),
        );
        cx.run_until_parked();
        window
            .read_with(&cx, |root, _| {
                assert!(root.remote_window.as_ref().unwrap().action_error.is_none())
            })
            .unwrap();
    }
    let old_input = window
        .read_with(&cx, |root, _| {
            root.remote_window.as_ref().unwrap().terminals["s"]
                .input
                .clone()
        })
        .unwrap();
    fake.fail_input.store(true, Ordering::SeqCst);
    window
        .update(&mut cx, |_, window, cx| {
            old_input.update(cx, |input, cx| {
                input.replace_text_in_range(None, "paste", window, cx)
            })
        })
        .unwrap();
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| {
            assert_eq!(
                root.remote_window.as_ref().unwrap().action_error.as_deref(),
                Some("input was discarded")
            )
        })
        .unwrap();
    window
        .update(&mut cx, |root, window, cx| {
            root.return_to_this_computer(window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    fake.emit("session/input-error", serde_json::json!({"session_id":runner_core::remote::terminal_address("host","s"),"message":"old target"}));
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| {
            assert!(root.remote_window.is_none());
            assert!(root.chat_error.is_none());
        })
        .unwrap();
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |_, window, cx| {
            old_input.update(cx, |input, cx| {
                input.replace_text_in_range(None, "stale", window, cx)
            })
        })
        .unwrap();
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| {
            assert!(root.remote_window.as_ref().unwrap().action_error.is_none())
        })
        .unwrap();
}

#[test]
fn pair_then_open_connects_when_follow_pushes_without_a_new_connection_event() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    fake.computers.lock().unwrap()[1].paired = false;
    fake.follow_push.store(true, Ordering::SeqCst);
    clients.update(&mut cx, |state, cx| state.refresh(cx));
    cx.run_until_parked();
    let window = add_window(&mut cx, &store, temp.path(), "pair-open-push");
    window
        .update(&mut cx, |root, window, cx| {
            let computer = fake.computers.lock().unwrap()[1].clone();
            root.open_computer_pair(
                computer,
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
    assert!(fake
        .follows
        .lock()
        .unwrap()
        .contains(&("host".into(), true)));
    window
        .read_with(&cx, |root, _| {
            let remote = root.remote_window.as_ref().unwrap();
            assert!(remote.connection == Connection::Connected);
            assert!(remote.terminals.contains_key("s"));
        })
        .unwrap();
}

#[test]
fn retry_on_a_busy_live_route_connects_despite_data_pushes_during_load() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    fake.follow_push.store(true, Ordering::SeqCst);
    fake.overview
        .lock()
        .unwrap()
        .sessions
        .get_mut("s")
        .unwrap()
        .status
        .status = Status::Working;
    let window = add_window(&mut cx, &store, temp.path(), "retry-busy-push");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let calls = fake.overview_calls.load(Ordering::SeqCst);
    clients.update(&mut cx, |state, cx| {
        state.connections.insert("host".into(), Connection::Opening);
        state.send(Command::Retry("host".into()));
        cx.notify();
    });
    cx.run_until_parked();
    assert!(fake.overview_calls.load(Ordering::SeqCst) > calls);
    assert_eq!(fake.pairs.load(Ordering::SeqCst), 0);
    clients.read_with(&cx, |state, _| {
        assert!(state.connection("host") == Connection::Connected);
        assert!(state.overviews["host"].sessions["s"].status.status == Status::Working);
    });
    window
        .read_with(&cx, |root, _| {
            assert!(root.remote_window.as_ref().unwrap().connection == Connection::Connected)
        })
        .unwrap();
}

#[test]
fn duplicate_incoming_keys_keep_separate_device_rows_and_revoke_menus() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, _, _) = fixture(&mut cx, temp.path());
    let computers = computers();
    let trust = RemoteTrust {
        devices: vec![
            device("Studio", "first-key"),
            device("Studio", "second-key"),
        ],
        ..Default::default()
    };
    let rows = model::computer_rows(&computers, &trust);
    assert_eq!(rows.len(), computers.len() + 2);
    assert!(rows[1].devices.is_empty());
    let window = add_window(&mut cx, &store, temp.path(), "duplicate-devices");
    window
        .update(&mut cx, |root, _, cx| {
            for (row, key) in rows[computers.len()..]
                .iter()
                .zip(["first-key", "second-key"])
            {
                assert!(row.computer.is_none());
                assert_eq!(row.devices.len(), 1);
                assert_eq!(row.devices[0].fingerprint, key);
                assert!(root.computer_menu(row.computer, &row.devices, cx).is_some());
            }
            let menus = root.client_ui.menus.borrow();
            assert!(menus.contains_key("first-key-first-key-false"));
            assert!(menus.contains_key("second-key-second-key-false"));
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
        .unbounded_send(Command::Acquire("host".into()))
        .unwrap();
    assert_eq!(entered.recv_timeout(timeout).unwrap(), "host");
    assert!(
        matches!(results.recv_timeout(timeout).unwrap(), model::Update::LoadStarted(id, 1) if id == "host")
    );
    commands.unbounded_send(Command::Refresh).unwrap();
    assert!(matches!(
        results.recv_timeout(timeout).unwrap(),
        model::Update::Access(true)
    ));
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
            model::Update::Connection(id, _, Connection::Connected) if id == "other" => {
                connected += 1
            }
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
    let follows = fake.follows.lock().unwrap();
    for id in ["host", "other"] {
        assert_eq!(
            follows
                .iter()
                .filter(|(computer, follow)| computer == id && !follow)
                .count(),
            1
        );
    }
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
fn mission_members_use_the_shared_workspace_and_empty_shells_draw_no_drawer() {
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
        assert!(visual.debug_bounds("MISSION_HEADER").is_some());
        assert!(visual.debug_bounds("MISSION_TABS").is_some());
        assert!(visual.debug_bounds("MISSION_RAIL").is_some());
        assert!(visual.debug_bounds("mission-terminal-drawer").is_none());
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
    fake.set_overview(overview);
    let window = add_window(&mut cx, &store, temp.path(), "column");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let first = visual.debug_bounds("PANE_HEADER:s").unwrap();
    let second = visual.debug_bounds("PANE_HEADER:second").unwrap();
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
        assert_eq!(
            fake.follows.lock().unwrap().last(),
            Some(&("host".into(), false))
        );
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

pub(crate) fn enable_front_page(fake: &Fake) {
    *fake.features.lock().unwrap() = [
        "catalog",
        "folders",
        "startChat",
        "startShell",
        "stop",
        "resume",
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
        "stopMission",
        "resumeMission",
    ]
    .map(str::to_owned)
    .to_vec();
}
pub(crate) fn action_trace(fake: &Fake, kind: &str) -> Vec<serde_json::Value> {
    fake.action_requests
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, request)| request["type"] == kind)
        .map(|(_, request)| request.clone())
        .collect()
}

#[test]
fn shared_remote_start_uses_host_choices_and_keeps_success_pending_until_push() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let window = add_window(&mut cx, &store, temp.path(), "host-start");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let local_settings = store.read_with(&cx, |store, _| store.settings.clone());
    fake.local_requests.lock().unwrap().clear();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let create = visual.debug_bounds("SIDEBAR_WORKSPACE_NEW_CHAT").unwrap();
    visual.simulate_click(create.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("START_CHAT_FORM").is_some());
    window
        .read_with(&visual, |root, cx| {
            let (_, cwd) = root.start_chat_modal.as_ref().unwrap().test_inputs();
            assert_eq!(cwd.read(cx).text(), "");
        })
        .unwrap();
    visual.update(|window, cx| window.dispatch_action(Box::new(ConfirmStartChat), cx));
    visual.run_until_parked();
    let starts = action_trace(&fake, "startChat");
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0]["runtime"], "codex");
    for key in ["cwd", "model", "effort", "speed", "role"] {
        assert!(starts[0][key].is_null(), "{key}");
    }
    assert_eq!(starts[0]["project"], "p");
    assert!(starts[0]["cols"].as_u64().unwrap() >= 20);
    assert!(starts[0]["rows"].as_u64().unwrap() >= 8);
    assert!(action_trace(&fake, "rename").is_empty());
    window
        .read_with(&visual, |root, _| {
            let remote = root.remote_window.as_ref().unwrap();
            assert_eq!(remote.layouts.len(), 1);
            assert_eq!(remote.activate_session.as_deref(), Some("started-session"));
            assert_eq!(root.tabs.tabs().len(), 0);
        })
        .unwrap();
    let mut pushed = overview();
    let mut tab = pushed.tabs[0].clone();
    tab.id = "started-tab".into();
    tab.panes = runner_core::companion::Pane::Leaf {
        session: Some("started-session".into()),
    };
    pushed.tabs.push(tab);
    pushed
        .sessions
        .insert("started-session".into(), pushed.sessions["s"].clone());
    fake.set_overview(pushed);
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| {
            let remote = root.remote_window.as_ref().unwrap();
            assert!(remote.active == Some(RemoteItem::Tab("started-tab".into())));
            assert!(remote.activate_session.is_none());
        })
        .unwrap();
    assert_eq!(
        store.read_with(&visual, |store, _| store
            .settings
            .default_working_dir
            .clone()),
        local_settings.default_working_dir
    );
    assert!(fake.local_requests.lock().unwrap().is_empty());
    let mut listed = store.read_with(&visual, |store, cx| {
        store.remote_clients.read(cx).overviews["host"].clone()
    });
    listed.projects.push(runner_core::companion::Project {
        id: "other-project".into(),
        name: "Other project".into(),
    });
    fake.set_overview(listed);
    visual.run_until_parked();
    let project = visual
        .debug_bounds("SIDEBAR_PROJECT:other-project")
        .unwrap();
    visual.simulate_click(project.center(), Default::default());
    visual.run_until_parked();
    window
        .update(&mut visual, |root, window, cx| {
            root.open_new_tab_modal(&NewTab, window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(action_trace(&fake, "catalog").len(), 3);
    visual.update(|window, cx| window.dispatch_action(Box::new(ConfirmStartChat), cx));
    visual.run_until_parked();
    assert_eq!(
        action_trace(&fake, "startChat")[1]["project"],
        "other-project"
    );
}

#[test]
fn remote_chat_rename_and_shell_placement_retry_do_not_start_twice() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    fake.action_results.lock().unwrap().insert(
        "rename".into(),
        [Err(ClientError::msg("rename refused"))].into(),
    );
    let window = add_window(&mut cx, &store, temp.path(), "partial-start");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, window, cx| {
            root.open_new_tab_modal(&NewTab, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, window, cx| {
            let (title, _) = root.start_chat_modal.as_ref().unwrap().test_inputs();
            title.update(cx, |field, cx| {
                field.replace_text_in_range(None, "Host title", window, cx)
            });
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.update(|window, cx| window.dispatch_action(Box::new(ConfirmStartChat), cx));
    visual.run_until_parked();
    assert_eq!(action_trace(&fake, "startChat").len(), 1);
    assert_eq!(action_trace(&fake, "rename").len(), 1);
    assert!(visual.debug_bounds("START_CHAT_FORM").is_some());
    visual.update(|window, cx| window.dispatch_action(Box::new(ConfirmStartChat), cx));
    visual.run_until_parked();
    assert_eq!(action_trace(&fake, "startChat").len(), 1);
    assert_eq!(action_trace(&fake, "rename").len(), 2);
    fake.action_results.lock().unwrap().insert(
        "layout".into(),
        [Err(ClientError::msg("placement refused"))].into(),
    );
    window
        .update(&mut visual, |root, _, cx| {
            let draft = root.front_page_layout().unwrap().clone();
            root.remote_start_shell(draft.clone(), None, (83, 27), cx);
            let mut another = draft;
            another.id = "another-tab".into();
            root.remote_start_shell(another, None, (83, 27), cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(action_trace(&fake, "startShell").len(), 1);
    assert_eq!(action_trace(&fake, "layout").len(), 1);
    window
        .update(&mut visual, |root, _, cx| {
            let draft = root.front_page_layout().unwrap().clone();
            assert!(draft.drawer_shells().is_empty());
            root.remote_start_shell(draft, None, (83, 27), cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(action_trace(&fake, "startShell").len(), 1);
    assert_eq!(action_trace(&fake, "layout").len(), 2);
    assert_eq!(action_trace(&fake, "startShell")[0]["cols"], 83);
    assert_eq!(action_trace(&fake, "startShell")[0]["rows"], 27);
}

#[test]
fn remote_layout_waits_for_topology_push_and_serializes_each_tab() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let window = add_window(&mut cx, &store, temp.path(), "layout-push");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let draft = window
        .update(&mut cx, |root, _, cx| {
            let mut draft = root.front_page_layout().unwrap().clone();
            let pane = draft.focused_pane_id.clone();
            draft.split(&pane, SplitOrientation::Column).unwrap();
            root.remote_layout(draft.clone(), false, cx);
            draft
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, _, cx| {
            assert_eq!(root.front_page_layout().unwrap().root.leaves().len(), 1);
            root.remote_layout(draft.clone(), false, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(action_trace(&fake, "layout").len(), 1);
    let mut pushed = overview();
    pushed.sessions.get_mut("s").unwrap().title = "Title only".into();
    fake.set_overview(pushed.clone());
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| {
            assert_eq!(root.remote_window.as_ref().unwrap().layout_hints.len(), 1)
        })
        .unwrap();
    pushed.tabs[0].panes = serde_json::from_value(serde_json::json!({"split":"column","sizes":[0.5,0.5],"a":{"session":"s"},"b":{"session":null}})).unwrap();
    fake.set_overview(pushed.clone());
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| {
            let layout = root.front_page_layout().unwrap();
            assert_eq!(layout.root.leaves()[1].id, draft.root.leaves()[1].id);
            assert!(root.remote_window.as_ref().unwrap().layout_hints.is_empty());
        })
        .unwrap();
    let attaches = fake.attaches.lock().unwrap().len();
    fake.set_overview(pushed);
    cx.run_until_parked();
    assert_eq!(fake.attaches.lock().unwrap().len(), attaches);
    window
        .update(&mut cx, |root, _, cx| {
            let mut next = root.front_page_layout().unwrap().clone();
            let pane = next.root.leaves()[0].id.clone();
            next.split(&pane, SplitOrientation::Row).unwrap();
            root.remote_layout(next, false, cx);
            let clients = root.app_store.read(cx).remote_clients.clone();
            clients.update(cx, |clients, cx| {
                clients.apply_event(ClientEvent {
                    name: "remote/overview".into(),
                    payload: serde_json::json!({"computer":"host","overview":overview()}),
                });
                cx.notify();
            });
            root.sync_remote_state(cx);
            assert_eq!(root.front_page_layout().unwrap().root.leaves().len(), 1);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| {
            assert_eq!(root.front_page_layout().unwrap().root.leaves().len(), 1);
            assert!(root.remote_window.as_ref().unwrap().layout_hints.is_empty());
        })
        .unwrap();
    assert_eq!(action_trace(&fake, "layout").len(), 2);
    window
        .update(&mut cx, |root, window, cx| {
            root.close_remote_front_page(window, cx)
        })
        .unwrap();
}

#[test]
fn capability_bootstrap_cannot_replace_pushed_state_when_another_window_opens() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let stale = overview();
    let mut newest = stale.clone();
    newest.tabs[0].name = "Latest".into();
    newest.sessions.get_mut("s").unwrap().status.status = Status::Working;
    fake.set_overview(newest.clone());
    cx.run_until_parked();
    fake.action_results.lock().unwrap().insert(
        "overview".into(),
        [Ok(serde_json::to_value(stale).unwrap())].into(),
    );
    for name in ["bootstrap-one", "bootstrap-two"] {
        let window = add_window(&mut cx, &store, temp.path(), name);
        window
            .update(&mut cx, |root, window, cx| {
                root.choose_computer(computers()[1].clone(), false, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
    }
    clients.read_with(&cx, |clients, _| {
        assert_eq!(clients.overviews["host"].tabs[0].name, "Latest");
        assert!(clients.overviews["host"].sessions["s"].status.status == Status::Working);
    });
    assert_eq!(action_trace(&fake, "overview").len(), 1);
}

#[test]
fn host_folder_menu_preserves_typed_paths_until_choice_and_retries_explicitly() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    fake.computers.lock().unwrap()[1].info.platform = "windows".into();
    store
        .read_with(&cx, |store, _| store.remote_clients.clone())
        .update(&mut cx, |clients, cx| clients.refresh(cx));
    cx.run_until_parked();
    let window = add_window(&mut cx, &store, temp.path(), "folder-menu");
    window
        .update(&mut cx, |root, window, cx| {
            let mut host = computers()[1].clone();
            host.info.platform = "windows".into();
            root.choose_computer(host, false, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, window, cx| {
            root.open_new_tab_modal(&NewTab, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let (cwd, picker) = window
        .read_with(&cx, |root, _| {
            let modal = root.start_chat_modal.as_ref().unwrap();
            (modal.test_inputs().1, modal.test_folder_picker().unwrap())
        })
        .unwrap();
    window
        .update(&mut cx, |_, window, cx| {
            picker.update(cx, |picker, cx| picker.toggle(window, cx))
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    assert!(visual.debug_bounds("HOST_FOLDER_MENU").is_some());
    assert!(visual.debug_bounds("HOST_FOLDER_ROW:choice-0").is_some());
    assert!(visual.debug_bounds("HOST_FOLDER_ROW:choice-1").is_some());
    assert!(visual.debug_bounds("HOST_FOLDER_ROW:choice-2").is_none());
    assert_eq!(
        cwd.read_with(&visual, |field, _| field.text().to_owned()),
        ""
    );
    let project = visual.debug_bounds("HOST_FOLDER_ROW:choice-0").unwrap();
    visual.simulate_click(project.center(), Default::default());
    visual.run_until_parked();
    assert_eq!(
        cwd.read_with(&visual, |field, _| field.text().to_owned()),
        "C:\\work"
    );
    window
        .update(&mut visual, |_, window, cx| {
            picker.update(cx, |picker, cx| picker.toggle(window, cx))
        })
        .unwrap();
    visual.run_until_parked();
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(visual.debug_bounds("HOST_FOLDER_MENU").is_none());
    assert_eq!(
        cwd.read_with(&visual, |field, _| field.text().to_owned()),
        "C:\\work"
    );
    fake.action_results.lock().unwrap().insert("folders".into(), [Ok(serde_json::json!({"path":"C:\\Users\\Jason", "parent":"C:\\Users", "folders":["work"]})), Err(ClientError::msg("denied")), Ok(serde_json::json!({"path":"C:\\Users\\Jason\\work", "parent":"C:\\Users\\Jason", "folders":[]}))].into());
    window
        .update(&mut visual, |_, window, cx| {
            picker.update(cx, |picker, cx| picker.toggle(window, cx))
        })
        .unwrap();
    visual.run_until_parked();
    let browse = visual.debug_bounds("HOST_FOLDER_ROW:browse").unwrap();
    visual.simulate_click(browse.center(), Default::default());
    visual.run_until_parked();
    assert!(action_trace(&fake, "folders").last().unwrap()["path"].is_null());
    assert!(visual.debug_bounds("HOST_FOLDER_ROW:parent").is_some());
    let child = visual.debug_bounds("HOST_FOLDER_ROW:child-0").unwrap();
    visual.simulate_click(child.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("HOST_FOLDER_FAILED").is_some());
    assert_eq!(
        cwd.read_with(&visual, |field, _| field.text().to_owned()),
        "C:\\work"
    );
    let count = action_trace(&fake, "folders").len();
    visual.run_until_parked();
    assert_eq!(action_trace(&fake, "folders").len(), count);
    let retry = visual.debug_bounds("HOST_FOLDER_ROW:retry").unwrap();
    visual.simulate_click(retry.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("HOST_FOLDER_EMPTY").is_some());
    assert_eq!(
        action_trace(&fake, "folders").last().unwrap()["path"],
        "C:\\Users\\Jason\\work"
    );
    visual.simulate_keystrokes("up enter");
    visual.run_until_parked();
    assert_eq!(
        cwd.read_with(&visual, |field, _| field.text().to_owned()),
        "C:\\Users\\Jason\\work"
    );
    assert!(visual.debug_bounds("HOST_FOLDER_MENU").is_none());
    fake.action_results.lock().unwrap().insert(
        "folders".into(),
        [Err(ClientError::msg("invalid folder"))].into(),
    );
    window
        .update(&mut visual, |_, window, cx| {
            cwd.read(cx).focus_handle().focus(window, cx);
            cwd.update(cx, |field, cx| {
                field.replace_text_in_range(None, "\\\\nas\\share\\missing", window, cx)
            });
        })
        .unwrap();
    visual.run_until_parked();
    window
        .update(&mut visual, |_, window, cx| {
            assert!(
                cwd.read(cx).focus_handle().is_focused(window),
                "cwd did not focus"
            )
        })
        .unwrap();
    visual.simulate_keystrokes("tab");
    visual.run_until_parked();
    assert!(
        picker.read_with(&visual, |picker, _| picker.path_error.is_some()),
        "folder calls: {:?}",
        action_trace(&fake, "folders")
    );
    assert!(cwd.read_with(&visual, |field, _| field
        .text()
        .ends_with("\\\\nas\\share\\missing")));
    window
        .update(&mut visual, |_, window, cx| {
            picker.update(cx, |picker, cx| picker.toggle(window, cx))
        })
        .unwrap();
    visual.run_until_parked();
    visual.simulate_click(point(px(1.), px(1.)), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("HOST_FOLDER_MENU").is_none());
}

#[test]
fn picker_follows_only_online_paired_computers_and_releases_on_lock_offline_and_off() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    {
        let mut directory = fake.computers.lock().unwrap();
        for (id, online, paired) in [
            ("offline", false, true),
            ("unpaired", true, false),
            ("other", true, true),
        ] {
            directory.push(Computer {
                id: id.into(),
                online,
                paired,
                ..Default::default()
            });
        }
    }
    clients.update(&mut cx, |clients, cx| clients.refresh(cx));
    cx.run_until_parked();
    let follows = fake.follows.lock().unwrap().clone();
    assert!(follows.contains(&("other".into(), true)));
    assert!(!follows
        .iter()
        .any(|(id, _)| id == "offline" || id == "unpaired"));
    fake.emit("remote/connection", serde_json::json!({"computer":"other", "state":"disconnected", "error":"Remote channel: pinLocked."}));
    cx.run_until_parked();
    assert!(fake
        .follows
        .lock()
        .unwrap()
        .contains(&("other".into(), false)));
    fake.computers.lock().unwrap()[1].online = false;
    clients.update(&mut cx, |clients, cx| clients.refresh(cx));
    cx.run_until_parked();
    assert!(fake
        .follows
        .lock()
        .unwrap()
        .contains(&("host".into(), false)));
    fake.computers.lock().unwrap()[1].online = true;
    clients.update(&mut cx, |clients, cx| clients.refresh(cx));
    cx.run_until_parked();
    let window = add_window(&mut cx, &store, temp.path(), "attention-release");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        fake.follows
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, follow)| id == "host" && *follow)
            .count(),
        2
    );
    window
        .read_with(&cx, |root, cx| {
            assert!(root.front_page_client(cx).supports(&["stop"]))
        })
        .unwrap();
    fake.computers.lock().unwrap()[1].paired = false;
    clients.update(&mut cx, |clients, cx| clients.refresh(cx));
    cx.run_until_parked();
    assert_eq!(
        fake.follows.lock().unwrap().last(),
        Some(&("host".into(), false))
    );
    window
        .read_with(&cx, |root, cx| {
            assert!(!root.front_page_client(cx).supports(&["stop"]));
            assert!(root.remote_window.as_ref().unwrap().connection == Connection::Revoked);
        })
        .unwrap();
    fake.emit(
        "remote/status",
        serde_json::to_value(RemoteStatus::default()).unwrap(),
    );
    cx.run_until_parked();
    window
        .update(&mut cx, |root, window, cx| {
            root.return_to_this_computer(window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        fake.follows.lock().unwrap().last(),
        Some(&("host".into(), false))
    );
}

#[test]
fn missing_folder_capability_keeps_typed_host_paths_and_rejects_local_routes() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    *fake.features.lock().unwrap() = vec!["catalog".into(), "startChat".into()];
    let window = add_window(&mut cx, &store, temp.path(), "typed-host-path");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let route = window.read_with(&cx, |root, _| root.route.clone()).unwrap();
    fake.local_requests.lock().unwrap().clear();
    window
        .update(&mut cx, |root, window, cx| {
            root.enter_settings_route(None, window, cx);
            root.open_roles(window, cx);
            root.open_crews(window, cx);
            root.set_route(AppRoute::ArchivedChat, cx);
            root.open_start_mission_modal(
                None,
                runner_core::protocol::project::ProjectScope::Root,
                window,
                cx,
            );
            assert_eq!(root.route, route);
            assert!(root.start_mission_modal.is_none());
            root.open_new_tab_modal(&NewTab, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, window, cx| {
            let modal = root.start_chat_modal.as_ref().unwrap();
            let (_, cwd) = modal.test_inputs();
            cwd.update(cx, |field, cx| {
                field.replace_text_in_range(None, r"\\nas\share\work", window, cx)
            });
            modal
                .test_folder_picker()
                .unwrap()
                .update(cx, |picker, cx| {
                    picker.toggle(window, cx);
                    assert!(!picker.open);
                });
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.update(|window, cx| window.dispatch_action(Box::new(ConfirmStartChat), cx));
    visual.run_until_parked();
    assert_eq!(
        action_trace(&fake, "startChat")[0]["cwd"],
        r"\\nas\share\work"
    );
    assert!(action_trace(&fake, "folders").is_empty());
    assert!(action_trace(&fake, "rename").is_empty());
    assert!(fake.local_requests.lock().unwrap().is_empty());
    window
        .update(&mut visual, |root, window, cx| {
            root.open_new_tab_modal(&NewTab, window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("START_CHAT_FORM").is_some());
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"reconnecting"}),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("START_CHAT_FORM").is_none());
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"connected"}),
    );
    visual.run_until_parked();
    assert_eq!(action_trace(&fake, "startChat").len(), 1);
    window
        .read_with(&visual, |root, _| assert_eq!(root.route, route))
        .unwrap();
}

#[test]
fn placed_starts_ignore_intermediate_tabs_and_activate_only_the_destination() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for placement in ["chat", "split-shell", "drawer"] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        let mut initial = overview();
        initial.tabs[0].panes = serde_json::from_value(serde_json::json!({"split":"column","sizes":[0.5,0.5],"a":{"session":"s"},"b":{"session":null}})).unwrap();
        fake.set_overview(initial.clone());
        cx.run_until_parked();
        let window = add_window(&mut cx, &store, temp.path(), placement);
        window
            .update(&mut cx, |root, window, cx| {
                root.choose_computer(computers()[1].clone(), false, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        let pane = window
            .read_with(&cx, |root, _| {
                root.front_page_layout().unwrap().root.leaves()[1]
                    .id
                    .clone()
            })
            .unwrap();
        if placement == "chat" {
            window
                .update(&mut cx, |root, window, cx| {
                    root.open_pane_chat_modal(&pane, window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            window
                .update(&mut cx, |_, window, cx| {
                    window.dispatch_action(Box::new(ConfirmStartChat), cx)
                })
                .unwrap();
        } else {
            window
                .update(&mut cx, |root, _, cx| {
                    let draft = root.front_page_layout().unwrap().clone();
                    root.remote_start_shell(
                        draft,
                        (placement == "split-shell").then(|| pane.clone()),
                        (81, 25),
                        cx,
                    );
                })
                .unwrap();
        }
        let mut intermediate = initial.clone();
        let mut standalone = initial.tabs[0].clone();
        standalone.id = "temporary-start-tab".into();
        standalone.panes =
            serde_json::from_value(serde_json::json!({"session":"started-session"})).unwrap();
        standalone.shells.clear();
        intermediate.tabs.push(standalone);
        let mut session = intermediate.sessions["s"].clone();
        session.kind = if placement == "chat" {
            runner_core::companion::SessionKind::Chat
        } else {
            runner_core::companion::SessionKind::Shell
        };
        intermediate
            .sessions
            .insert("started-session".into(), session);
        window
            .update(&mut cx, |root, _, cx| {
                let clients = root.app_store.read(cx).remote_clients.clone();
                clients.update(cx, |clients, cx| {
                    clients.apply_event(ClientEvent {
                        name: "remote/overview".into(),
                        payload: serde_json::json!({"computer":"host","overview":intermediate}),
                    });
                    cx.notify();
                });
                root.sync_remote_state(cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .read_with(&cx, |root, _| {
                let remote = root.remote_window.as_ref().unwrap();
                assert_eq!(
                    remote.activate_session.as_deref(),
                    Some("started-session"),
                    "{placement}"
                );
                assert!(matches!(&remote.active, Some(Item::Tab(id)) if id == "remote-tab"));
                assert_eq!(
                    root.front_page_layout().unwrap().root.leaves()[1].session_id,
                    None
                );
            })
            .unwrap();
        intermediate.tabs.pop();
        if placement == "drawer" {
            intermediate.tabs[0].shells.push("started-session".into());
        } else {
            intermediate.tabs[0].panes = serde_json::from_value(serde_json::json!({"split":"column","sizes":[0.5,0.5],"a":{"session":"s"},"b":{"session":"started-session"}})).unwrap();
        }
        fake.set_overview(intermediate);
        cx.run_until_parked();
        window
            .read_with(&cx, |root, _| {
                let remote = root.remote_window.as_ref().unwrap();
                assert!(remote.activate_session.is_none(), "{placement}");
                assert!(matches!(&remote.active, Some(Item::Tab(id)) if id == "remote-tab"));
                let layout = root.front_page_layout().unwrap();
                if placement == "drawer" {
                    assert!(layout.drawer_open());
                    assert_eq!(layout.active_drawer_shell(), Some("started-session"));
                } else {
                    assert_eq!(layout.focused_pane_id, pane);
                    assert_eq!(layout.focused_session_id(), Some("started-session"));
                }
            })
            .unwrap();
        // Selection of an independently removed tab is reconciled too.
        window
            .update(&mut cx, |root, _, cx| {
                root.remote_window.as_mut().unwrap().active = Some(Item::Tab("removed-tab".into()));
                root.sync_remote_state(cx);
                assert_eq!(root.front_page_layout().unwrap().id, "remote-tab");
            })
            .unwrap();
    }
}

#[test]
fn a_newer_host_topology_supersedes_a_chat_fill_result_and_allows_the_next_edit() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let mut initial = overview();
    initial.tabs[0].panes = serde_json::from_value(serde_json::json!({"split":"column","sizes":[0.5,0.5],"a":{"session":"s"},"b":{"session":null}})).unwrap();
    fake.set_overview(initial);
    cx.run_until_parked();
    let window = add_window(&mut cx, &store, temp.path(), "superseded-chat");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, window, cx| {
            let pane = root.front_page_layout().unwrap().root.leaves()[1]
                .id
                .clone();
            root.open_pane_chat_modal(&pane, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |_, window, cx| {
            window.dispatch_action(Box::new(ConfirmStartChat), cx)
        })
        .unwrap();
    window
        .update(&mut cx, |root, _, cx| {
            let clients = root.app_store.read(cx).remote_clients.clone();
            clients.update(cx, |clients, cx| {
                clients.apply_event(ClientEvent {
                    name: "remote/overview".into(),
                    payload: serde_json::json!({"computer":"host","overview":overview()}),
                });
                cx.notify();
            });
            root.sync_remote_state(cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(action_trace(&fake, "startChat").len(), 1);
    assert_eq!(action_trace(&fake, "layout").len(), 1);
    window
        .update(&mut cx, |root, _, cx| {
            let remote = root.remote_window.as_ref().unwrap();
            assert!(remote.layout_hints.is_empty());
            assert!(remote.activate_session.is_none());
            assert_eq!(root.front_page_layout().unwrap().root.leaves().len(), 1);
            let mut next = root.front_page_layout().unwrap().clone();
            let pane = next.focused_pane_id.clone();
            next.split(&pane, SplitOrientation::Row).unwrap();
            root.remote_layout(next, false, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(action_trace(&fake, "layout").len(), 2);
}

#[test]
fn remote_resume_and_pane_shortcuts_use_the_remote_layout_with_local_start_saved() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for saved_start in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        let mut data = overview();
        let mut second = data.sessions["s"].clone();
        second.status.status = Status::Stopped;
        data.sessions.get_mut("s").unwrap().status.status = Status::Stopped;
        data.sessions.insert("remote-second".into(), second);
        data.tabs[0].panes = serde_json::from_value(serde_json::json!({"split":"column","sizes":[0.5,0.5],"a":{"session":"s"},"b":{"session":"remote-second"}})).unwrap();
        fake.set_overview(data);
        cx.run_until_parked();
        let window = add_window(&mut cx, &store, temp.path(), "resume-and-cycle");
        window
        .update(&mut cx, |root, window, cx| {
            let mut local = PaneLayout::single(None, &["local-only-session".into()]);
            local.split("p1", SplitOrientation::Row).unwrap();
            if let runner_app::pane_layout::PaneNode::Split(split) = &mut local.root {
                if let runner_app::pane_layout::PaneNode::Leaf(a) = split.a.as_mut() { a.id = "local-a".into(); }
                if let runner_app::pane_layout::PaneNode::Leaf(b) = split.b.as_mut() { b.id = "local-b".into(); }
            }
            local.focused_pane_id = "local-a".into();
            let row = serde_json::from_value(serde_json::json!({"id":"local-tab","parent_id":null,"position":0,"type":"tab","name":"Local tab","ref_id":null,"layout":local.serialize().unwrap(),"pinned_position":null,"last_completed_at":null,"last_viewed_at":null,"created_at":"2026-10-23T00:00:00Z"})).unwrap();
            root.tabs = TabSet::from_rows(&[row]);
            root.route = if saved_start { AppRoute::Chat } else { AppRoute::Roles };
            if saved_start { root.tabs.clear_active(); }
            root.choose_computer(computers()[1].clone(), false, window, cx);
        })
        .unwrap();
        cx.run_until_parked();
        let local_calls = fake.local_call_count();
        window
            .update(&mut cx, |root, window, cx| {
                root.focus_next_chat_pane(&FocusNextPane, window, cx);
                assert_eq!(
                    root.front_page_layout().unwrap().focused_session_id(),
                    Some("remote-second")
                );
                root.resume_focused_session(&ResumeFocusedSession, window, cx);
                root.focus_previous_chat_pane(&FocusPreviousPane, window, cx);
                assert_eq!(
                    root.front_page_layout().unwrap().focused_session_id(),
                    Some("s")
                );
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(action_trace(&fake, "resume")[0]["session"], "remote-second");
        window
            .update(&mut cx, |root, window, cx| {
                root.resume_chats(vec!["s".into(), "remote-second".into()], window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        let resumes = action_trace(&fake, "resume");
        assert_eq!(resumes.len(), 3);
        assert!(resumes
            .iter()
            .all(|r| r["session"] == "s" || r["session"] == "remote-second"));
        assert_eq!(fake.local_call_count(), local_calls);
        window
            .read_with(&cx, |root, _| {
                assert_eq!(
                    root.route,
                    if saved_start {
                        AppRoute::Chat
                    } else {
                        AppRoute::Roles
                    }
                );
                assert_eq!(root.tabs.active().is_none(), saved_start);
                assert!(root.tabs.tabs()[0].contains_session("local-only-session"));
                assert_eq!(root.tabs.tabs()[0].root.leaves().len(), 2);
            })
            .unwrap();
    }
}

#[test]
fn remote_split_shortcuts_use_target_focus_with_no_or_disjoint_local_tabs() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for saved_start in [true, false] {
        for (shortcut, direction) in [("cmd-d", "row"), ("shift-cmd-d", "column")] {
            for capable in [true, false] {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                cx.update(|cx| {
                    crate::keymap::install_bindings(
                        cx,
                        &crate::keymap::KeymapOverrides::new(),
                        false,
                    )
                });
                let (store, fake, _) = fixture(&mut cx, temp.path());
                enable_front_page(&fake);
                if !capable {
                    fake.set_features(&["overview"]);
                }
                let window = add_window(&mut cx, &store, temp.path(), "split-shortcuts");
                window.update(&mut cx, |root, window, cx| {
                    let mut local = PaneLayout::single(None, &["local-only-session".into()]);
                    if let runner_app::pane_layout::PaneNode::Leaf(leaf) = &mut local.root {
                        leaf.id = "local-only-pane".into();
                    }
                    local.focused_pane_id = "local-only-pane".into();
                    let row = serde_json::from_value(serde_json::json!({"id":"local-tab","parent_id":null,"position":0,"type":"tab","name":"Local tab","ref_id":null,"layout":local.serialize().unwrap(),"pinned_position":null,"last_completed_at":null,"last_viewed_at":null,"created_at":"2026-10-23T00:00:00Z"})).unwrap();
                    root.tabs = TabSet::from_rows(&[row]);
                    root.route = if saved_start { AppRoute::Chat } else { AppRoute::Roles };
                    if saved_start { root.tabs.clear_active(); }
                    choose_host(root, window, cx);
                }).unwrap();
                cx.run_until_parked();
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                window
                    .update(&mut visual, |root, window, cx| {
                        window.activate_window();
                        root.focus_remote_terminal("s", window, cx);
                    })
                    .unwrap();
                visual.run_until_parked();
                let local_calls = fake.local_call_count();
                visual.simulate_keystrokes(&crate::keymap::platform_default(shortcut));
                visual.run_until_parked();
                let layouts = action_trace(&fake, "layout");
                assert_eq!(
                    layouts.len(),
                    usize::from(capable),
                    "{shortcut} saved_start={saved_start}"
                );
                if capable {
                    assert_eq!(layouts[0]["target"]["id"], "remote-tab");
                    assert_eq!(layouts[0]["panes"]["split"], direction);
                    window
                        .read_with(&visual, |root, _| {
                            assert_eq!(root.front_page_layout().unwrap().root.leaves().len(), 1)
                        })
                        .unwrap();
                    let mut pushed = overview();
                    pushed.tabs[0].panes =
                        serde_json::from_value(layouts[0]["panes"].clone()).unwrap();
                    fake.set_overview(pushed);
                    visual.run_until_parked();
                    window
                        .read_with(&visual, |root, _| {
                            assert_eq!(root.front_page_layout().unwrap().root.leaves().len(), 2)
                        })
                        .unwrap();
                }
                assert_eq!(fake.local_call_count(), local_calls);
                window
                    .read_with(&visual, |root, _| {
                        assert_eq!(root.tabs.active().is_none(), saved_start);
                        assert_eq!(root.tabs.tabs()[0].root.leaves().len(), 1);
                        assert_eq!(root.tabs.tabs()[0].focused_pane_id, "local-only-pane");
                        assert_eq!(
                            root.route,
                            if saved_start {
                                AppRoute::Chat
                            } else {
                                AppRoute::Roles
                            }
                        );
                    })
                    .unwrap();
            }
        }
    }
}

#[test]
fn remote_shell_split_and_empty_fill_inherit_the_owning_host_cwd() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for cwd in [r"D:\work\shell-folder", r"\\nas\share\shell-folder"] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        let mut data = overview();
        data.sessions.get_mut("s").unwrap().kind = runner_core::companion::SessionKind::Shell;
        data.sessions.get_mut("s").unwrap().cwd = Some(cwd.into());
        fake.set_overview(data);
        cx.run_until_parked();
        let window = add_window(&mut cx, &store, temp.path(), "shell-cwd");
        window
            .update(&mut cx, |root, window, cx| {
                root.choose_computer(computers()[1].clone(), false, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, window, cx| {
                let layout = root.front_page_layout().unwrap();
                let pane = layout.focused_pane_id.clone();
                root.remote_window
                    .as_mut()
                    .unwrap()
                    .pane_bounds
                    .insert(PaneKey::new("remote-tab", &pane), size(px(1000.), px(600.)));
                root.split_pane(&pane, SplitOrientation::Column, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(action_trace(&fake, "startShell")[0]["cwd"], cwd);
        let mut data = overview();
        data.sessions.get_mut("s").unwrap().kind = runner_core::companion::SessionKind::Shell;
        data.sessions.get_mut("s").unwrap().cwd = Some(cwd.into());
        data.tabs[0].panes = serde_json::from_value(serde_json::json!({"split":"column","sizes":[0.5,0.5],"a":{"session":"s"},"b":{"session":null}})).unwrap();
        fake.set_overview(data);
        cx.run_until_parked();
        window
            .update(&mut cx, |root, _, cx| {
                let mut draft = root.front_page_layout().unwrap().clone();
                let empty = draft.root.leaves()[1].id.clone();
                draft.focus_pane(&empty);
                root.remote_start_shell(draft, Some(empty), (80, 24), cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(action_trace(&fake, "startShell")[1]["cwd"], cwd);
    }
}

#[test]
fn last_member_removal_relies_on_host_tab_deletion_and_empty_tabs_archive_explicitly() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for case in ["chat", "shell", "archive-all", "batch", "empty-tab"] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        fake.model_session_removal.store(true, Ordering::SeqCst);
        let mut data = overview();
        if case == "shell" {
            data.sessions.get_mut("s").unwrap().kind = runner_core::companion::SessionKind::Shell;
        }
        if case == "archive-all" || case == "batch" {
            let mut shell = data.sessions["s"].clone();
            shell.kind = runner_core::companion::SessionKind::Shell;
            data.sessions.insert("drawer-shell".into(), shell);
            data.tabs[0].shells.push("drawer-shell".into());
        }
        if case == "batch" {
            for id in ["batch-remove", "keep"] {
                data.sessions.insert(id.into(), data.sessions["s"].clone());
            }
            let mut tab = data.tabs[0].clone();
            tab.id = "surviving-tab".into();
            tab.shells.clear();
            tab.panes = serde_json::from_value(serde_json::json!({"split":"column","sizes":[0.5,0.5],"a":{"session":"batch-remove"},"b":{"session":"keep"}})).unwrap();
            data.tabs.push(tab);
        }
        if case == "empty-tab" {
            data.tabs[0].panes = runner_core::companion::Pane::Leaf { session: None };
            data.sessions.clear();
        }
        fake.set_overview(data);
        cx.run_until_parked();
        let window = add_window(&mut cx, &store, temp.path(), case);
        window
            .update(&mut cx, |root, window, cx| {
                root.choose_computer(computers()[1].clone(), false, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, window, cx| match case {
                "chat" | "shell" => {
                    let pane = root.front_page_layout().unwrap().focused_pane_id.clone();
                    root.remote_close_session("s", Some(&pane), false, cx);
                }
                "archive-all" => {
                    root.remote_archive_sessions(vec!["s".into(), "drawer-shell".into()], cx)
                }
                "batch" => root.remote_archive_sessions(
                    vec!["s".into(), "drawer-shell".into(), "batch-remove".into()],
                    cx,
                ),
                "empty-tab" => root.close_remote_front_page(window, cx),
                _ => unreachable!(),
            })
            .unwrap();
        cx.run_until_parked();
        let tab_archives = action_trace(&fake, "archive")
            .into_iter()
            .filter(|request| request["target"]["kind"] == "tab")
            .count();
        assert_eq!(tab_archives, usize::from(case == "empty-tab"), "{case}");
        assert_eq!(
            action_trace(&fake, "layout").len(),
            usize::from(case == "batch"),
            "{case}"
        );
        window
            .read_with(&cx, |root, _| {
                let remote = root.remote_window.as_ref().unwrap();
                assert!(
                    remote.action_error.is_none(),
                    "{case}: {:?}",
                    remote.action_error
                );
                assert!(remote.actions_pending.is_empty());
                assert!(
                    remote.layouts.contains_key("remote-tab"),
                    "a result changed visible state"
                );
            })
            .unwrap();
        let pushed = fake.overview.lock().unwrap().clone();
        assert!(pushed.tabs.iter().all(|tab| tab.id != "remote-tab"));
        fake.set_overview(pushed);
        cx.run_until_parked();
        window
            .read_with(&cx, |root, _| {
                let remote = root.remote_window.as_ref().unwrap();
                assert!(!remote.layouts.contains_key("remote-tab"));
                assert!(remote.action_error.is_none());
                assert!(remote.layout_hints.is_empty());
                if case == "batch" {
                    assert_eq!(remote.layouts["surviving-tab"].all_session_ids(), ["keep"]);
                }
            })
            .unwrap();
    }
}

#[test]
fn navigating_before_results_or_confirming_pushes_cancels_remote_activation() {
    use crate::surfaces::front_page::actions::SessionAction;
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for operation in [
        "chat-fill",
        "split-shell",
        "drawer",
        "resume",
        "fork",
        "new-tab",
    ] {
        for before_result in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, _) = fixture(&mut cx, temp.path());
            enable_front_page(&fake);
            let mut data = overview();
            data.tabs[0].panes = serde_json::from_value(serde_json::json!({"split":"column","sizes":[0.5,0.5],"a":{"session":"s"},"b":{"session":null}})).unwrap();
            if operation == "resume" {
                data.sessions.get_mut("s").unwrap().status.status = Status::Stopped;
            }
            data.sessions
                .insert("b-session".into(), data.sessions["s"].clone());
            let mut tab_b = data.tabs[0].clone();
            tab_b.id = "tab-b".into();
            tab_b.panes = runner_core::companion::Pane::Leaf {
                session: Some("b-session".into()),
            };
            data.tabs.push(tab_b);
            fake.set_overview(data.clone());
            cx.run_until_parked();
            let window = add_window(&mut cx, &store, temp.path(), operation);
            window
                .update(&mut cx, |root, window, cx| {
                    root.choose_computer(computers()[1].clone(), false, window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            if operation == "chat-fill" {
                window
                    .update(&mut cx, |root, window, cx| {
                        let pane = root.front_page_layout().unwrap().root.leaves()[1]
                            .id
                            .clone();
                        root.open_pane_chat_modal(&pane, window, cx);
                    })
                    .unwrap();
                cx.run_until_parked();
            }
            window
                .update(&mut cx, |root, window, cx| match operation {
                    "chat-fill" => window.dispatch_action(Box::new(ConfirmStartChat), cx),
                    "split-shell" | "drawer" => {
                        let draft = root.front_page_layout().unwrap().clone();
                        let pane =
                            (operation == "split-shell").then(|| draft.root.leaves()[1].id.clone());
                        root.remote_start_shell(draft, pane, (80, 24), cx);
                    }
                    "resume" => {
                        root.remote_session_action("s", SessionAction::Resume, (80, 24), cx)
                    }
                    "fork" => root.remote_session_action("s", SessionAction::Fork, (80, 24), cx),
                    "new-tab" => {
                        let mut draft = PaneLayout::single(None, &[]);
                        draft.id = "created-tab".into();
                        root.remote_layout(draft, true, cx);
                    }
                    _ => unreachable!(),
                })
                .unwrap();
            if !before_result {
                cx.run_until_parked();
            }
            let destination = if before_result {
                Item::Tab("tab-b".into())
            } else {
                Item::Mission("remote-mission".into())
            };
            window
                .update(&mut cx, |root, _, cx| {
                    if before_result {
                        assert!(
                            !root
                                .remote_window
                                .as_ref()
                                .unwrap()
                                .actions_pending
                                .is_empty(),
                            "{operation}"
                        );
                    }
                    root.select_remote_item(destination.clone(), cx);
                })
                .unwrap();
            cx.run_until_parked();
            window
                .read_with(&cx, |root, _| {
                    let remote = root.remote_window.as_ref().unwrap();
                    assert!(
                        remote.active == Some(destination.clone()),
                        "{operation}, before_result={before_result}"
                    );
                    assert!(remote.activate_session.is_none());
                    assert!(remote.activate_placement.is_none());
                    assert!(remote.activate_tab.is_none());
                    assert!(remote.actions_pending.is_empty());
                    assert!(remote.action_error.is_none());
                    assert!(remote.shell_placement.lock().unwrap().is_none());
                })
                .unwrap();
            match operation {
                "chat-fill" | "split-shell" => data.tabs[0].panes = serde_json::from_value(serde_json::json!({"split":"column","sizes":[0.5,0.5],"a":{"session":"s"},"b":{"session":"started-session"}})).unwrap(),
                "drawer" => data.tabs[0].shells.push("started-session".into()),
                "resume" => data.sessions.get_mut("s").unwrap().status.status = Status::Working,
                "fork" | "new-tab" => {
                    let mut tab = data.tabs[0].clone();
                    tab.id = if operation == "fork" { "fork-tab" } else { "created-tab" }.into();
                    tab.panes = runner_core::companion::Pane::Leaf { session: (operation == "fork").then(|| "started-session".into()) };
                    data.tabs.push(tab);
                }
                _ => unreachable!(),
            }
            let mut started = data.sessions["s"].clone();
            if operation == "split-shell" || operation == "drawer" {
                started.kind = runner_core::companion::SessionKind::Shell;
            }
            data.sessions.insert("started-session".into(), started);
            fake.set_overview(data);
            cx.run_until_parked();
            window
                .read_with(&cx, |root, _| {
                    let remote = root.remote_window.as_ref().unwrap();
                    assert!(
                        remote.active == Some(destination.clone()),
                        "{operation}, before_result={before_result}"
                    );
                    assert!(!remote.layouts["remote-tab"].drawer_open());
                    assert!(remote.actions_pending.is_empty());
                    assert!(remote.layout_hints.is_empty());
                })
                .unwrap();
        }
    }
}

#[test]
fn late_shell_placement_failure_settles_without_showing_errors_or_retrying_in_another_tab() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let mut data = overview();
    let mut tab = data.tabs[0].clone();
    tab.id = "tab-b".into();
    data.tabs.push(tab);
    fake.set_overview(data);
    cx.run_until_parked();
    let window = add_window(&mut cx, &store, temp.path(), "late-shell-error");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    fake.queue_result("layout", Err(ClientError::msg("placement failed")));
    window
        .update(&mut cx, |root, _, cx| {
            let draft = root.front_page_layout().unwrap().clone();
            root.remote_start_shell(draft, None, (80, 24), cx);
            root.select_remote_item(Item::Tab("tab-b".into()), cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, _, cx| {
            let remote = root.remote_window.as_ref().unwrap();
            assert!(remote.actions_pending.is_empty());
            assert!(remote.action_error.is_none());
            assert!(remote.shell_placement.lock().unwrap().is_none());
            let draft = root.front_page_layout().unwrap().clone();
            root.remote_start_shell(draft, None, (80, 24), cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(action_trace(&fake, "startShell").len(), 2);
    assert_eq!(
        action_trace(&fake, "layout").last().unwrap()["target"]["id"],
        "tab-b"
    );
}

#[test]
fn navigation_cancels_a_start_form_waiting_for_its_catalog_without_holding_the_new_form() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let window = add_window(&mut cx, &store, temp.path(), "catalog-navigation");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let catalogs_before = action_trace(&fake, "catalog").len();
    window
        .update(&mut cx, |root, window, cx| {
            root.open_new_tab_modal(&NewTab, window, cx);
            assert!(root
                .remote_window
                .as_ref()
                .unwrap()
                .actions_pending
                .contains("catalog"));
            root.select_remote_item(Item::Mission("remote-mission".into()), cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, window, cx| {
            assert!(root.start_chat_modal.is_none());
            let remote = root.remote_window.as_ref().unwrap();
            assert!(remote.active == Some(Item::Mission("remote-mission".into())));
            assert!(remote.actions_pending.is_empty());
            root.open_new_tab_modal(&NewTab, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| assert!(root.start_chat_modal.is_some()))
        .unwrap();
    assert_eq!(action_trace(&fake, "catalog").len(), catalogs_before + 2);
}

pub(crate) fn enable_missions(fake: &Fake) {
    enable_front_page(fake);
    fake.features.lock().unwrap().extend(
        [
            "feed",
            "follow",
            "unfollow",
            "answer",
            "post",
            "restart",
            "startMission",
        ]
        .map(str::to_owned),
    );
}

pub(crate) fn library_catalog(count: usize) -> serde_json::Value {
    serde_json::json!({
        "roles": (0..count).map(|i| serde_json::json!({
            "id":format!("host-role-{i}"), "name":format!("Host role {i}"), "handle":format!("host-coder-{i}"),
            "runtime":"codex", "model":"host-model", "effort":"high", "speed":null
        })).collect::<Vec<_>>(),
        "runtimes":[{"name":"codex","label":"Host Codex","available":true,"nativeFork":true,"defaultModel":null,"defaultEffort":null,"models":[],"efforts":[],"speeds":["standard","fast"]}],
        "accounts":[],"activeAccount":"",
        "crews": if count == 0 { vec![] } else { vec![serde_json::json!({"id":"host-crew","name":"Host duo","slots":(0..count.min(2)).map(|i| serde_json::json!({"handle":format!("worker-{i}"),"role":format!("host-role-{i}"),"lead":i==0})).collect::<Vec<_>>()})] },
        "projects":[],"recentFolders":[]
    })
}

#[test]
fn remote_libraries_show_host_rows_without_management_or_local_requests() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let window = add_window(&mut cx, &store, temp.path(), "remote-libraries");
    window
        .update(&mut cx, |root, window, cx| {
            root.open_roles(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let original_route = window.read_with(&cx, |root, _| root.route.clone()).unwrap();
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    fake.local_requests.lock().unwrap().clear();
    fake.action_requests.lock().unwrap().clear();
    fake.queue_result("catalog", Ok(library_catalog(2)));
    let role_entry = visual
        .debug_bounds("SIDEBAR_WORKSPACE:workspace-role")
        .unwrap();
    visual.simulate_click(role_entry.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("ROLE_ROW:host-role-0").is_some());
    assert!(visual.debug_bounds("ROLE_ROW:host-role-1").is_some());
    assert!(visual
        .debug_bounds("ROLE_TABLE_CELL Model host-model")
        .is_some());
    for selector in ["NEW_ROLE", "EMPTY_NEW_ROLE", "NEW_CREW", "EMPTY_NEW_CREW"] {
        assert!(visual.debug_bounds(selector).is_none(), "{selector}");
    }
    let row = visual.debug_bounds("ROLE_ROW:host-role-0").unwrap();
    visual.simulate_click(row.center(), Default::default());
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| {
            assert!(root.start_chat_modal.is_none());
            assert_eq!(root.route, original_route);
            assert!(root.remote_window.as_ref().unwrap().active == Some(Item::Roles));
        })
        .unwrap();
    fake.queue_result("catalog", Ok(library_catalog(2)));
    let crew_entry = visual
        .debug_bounds("SIDEBAR_WORKSPACE:workspace-crew")
        .unwrap();
    visual.simulate_click(crew_entry.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("CREW_ROW:host-crew").is_some());
    assert!(visual.debug_bounds("NEW_CREW").is_none());
    let row = visual.debug_bounds("CREW_ROW:host-crew").unwrap();
    visual.simulate_click(row.center(), Default::default());
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| {
            assert!(root.start_mission_modal.is_none());
            assert_eq!(root.route, original_route);
            assert!(root.remote_window.as_ref().unwrap().active == Some(Item::Crews));
        })
        .unwrap();
    assert!(fake.local_requests.lock().unwrap().is_empty());
    assert!(fake
        .action_requests
        .lock()
        .unwrap()
        .iter()
        .all(|(computer, request)| computer == "host" && request["type"] == "catalog"));
    window
        .update(&mut visual, |root, window, cx| {
            root.choose_computer(computers()[0].clone(), false, window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| {
            assert!(root.remote_window.is_none());
            assert_eq!(root.route, original_route);
        })
        .unwrap();
    assert!(visual.debug_bounds("NEW_ROLE").is_some());
}

#[test]
fn remote_libraries_require_catalog_and_gate_launch_actions_separately() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for catalog in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        fake.set_features(if catalog { &["catalog"] } else { &[] });
        let window = add_window(&mut cx, &store, temp.path(), "library-capabilities");
        window
            .update(&mut cx, |root, window, cx| {
                root.choose_computer(computers()[1].clone(), false, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        assert_eq!(
            visual
                .debug_bounds("SIDEBAR_WORKSPACE:workspace-role")
                .is_some(),
            catalog
        );
        assert_eq!(
            visual
                .debug_bounds("SIDEBAR_WORKSPACE:workspace-crew")
                .is_some(),
            catalog
        );
        assert!(visual.debug_bounds("SIDEBAR_WORKSPACE_NEW_CHAT").is_none());
        fake.queue_result("catalog", Ok(library_catalog(2)));
        window
            .update(&mut visual, |root, window, cx| root.open_roles(window, cx))
            .unwrap();
        visual.run_until_parked();
        assert_eq!(
            visual.debug_bounds("ROLE_ROW:host-role-0").is_some(),
            catalog
        );
        assert!(visual.debug_bounds("ROLE_ROW_CHAT:host-role-0").is_none());
        fake.queue_result("catalog", Ok(library_catalog(2)));
        window
            .update(&mut visual, |root, window, cx| root.open_crews(window, cx))
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("CREW_ROW_START:host-crew").is_none());
        assert!(action_trace(&fake, "startChat").is_empty());
        assert!(action_trace(&fake, "startMission").is_empty());
    }
}

#[test]
fn remote_libraries_filter_page_and_refresh_errors_without_local_preferences() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let window = add_window(&mut cx, &store, temp.path(), "library-search");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let sizes = store.read_with(&cx, |store, _| {
        (
            store.settings.roles_page_size,
            store.settings.crews_page_size,
        )
    });
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    fake.queue_result("catalog", Err(ClientError::msg("host catalog unavailable")));
    window
        .update(&mut visual, |root, window, cx| root.open_roles(window, cx))
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("PAGINATED_LIST_ERROR").is_some());
    fake.queue_result("catalog", Ok(library_catalog(25)));
    let refresh = visual.debug_bounds("REMOTE_LIBRARY_REFRESH").unwrap();
    visual.simulate_click(refresh.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("PAGINATED_LIST_ERROR").is_none());
    window
        .update(&mut visual, |root, _, cx| root.set_role_page_size(10, cx))
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("ROLE_ROW:host-role-0").is_some());
    assert!(visual.debug_bounds("ROLE_ROW:host-role-10").is_none());
    window
        .update(&mut visual, |root, _, cx| root.set_role_page(2, cx))
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("ROLE_ROW:host-role-0").is_none());
    assert!(visual.debug_bounds("ROLE_ROW:host-role-10").is_some());
    let reads = action_trace(&fake, "catalog").len();
    window
        .update(&mut visual, |root, _, cx| {
            root.set_role_query("host-coder-24".into(), cx)
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("ROLE_ROW:host-role-24").is_some());
    assert!(visual.debug_bounds("ROLE_ROW:host-role-10").is_none());
    assert_eq!(action_trace(&fake, "catalog").len(), reads);
    assert_eq!(
        store.read_with(&visual, |store, _| (
            store.settings.roles_page_size,
            store.settings.crews_page_size
        )),
        sizes
    );
    fake.queue_result("catalog", Ok(library_catalog(2)));
    window
        .update(&mut visual, |root, window, cx| root.open_crews(window, cx))
        .unwrap();
    visual.run_until_parked();
    window
        .update(&mut visual, |root, _, cx| {
            root.set_crew_query("host-coder-1".into(), cx)
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("CREW_ROW:host-crew").is_some());
    fake.queue_result("catalog", Ok(library_catalog(0)));
    window
        .update(&mut visual, |root, _, cx| root.refresh_remote_library(cx))
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("CREW_ROW:host-crew").is_none());
    assert!(visual.debug_bounds("EMPTY_NEW_CREW").is_none());
}

#[test]
fn remote_role_library_launch_refreshes_and_preselects_the_clicked_host_role() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let window = add_window(&mut cx, &store, temp.path(), "library-role-launch");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    fake.queue_result("catalog", Ok(library_catalog(2)));
    window
        .update(&mut visual, |root, window, cx| root.open_roles(window, cx))
        .unwrap();
    visual.run_until_parked();
    fake.queue_result("catalog", Ok(library_catalog(2)));
    let chat = visual.debug_bounds("ROLE_ROW_CHAT:host-role-1").unwrap();
    visual.simulate_click(chat.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("START_CHAT_FORM").is_some());
    visual.update(|window, cx| window.dispatch_action(Box::new(ConfirmStartChat), cx));
    visual.run_until_parked();
    let starts = action_trace(&fake, "startChat");
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0]["role"], "host-role-1");
    for field in ["runtime", "cwd", "model", "effort", "speed", "project"] {
        assert!(starts[0][field].is_null(), "{field}");
    }
    assert_eq!(action_trace(&fake, "catalog").len(), 3);
}

#[test]
fn remote_library_catalog_replies_cannot_override_a_refresh_or_local_return() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for return_local in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        let window = add_window(&mut cx, &store, temp.path(), "library-race");
        window
            .update(&mut cx, |root, window, cx| {
                root.choose_computer(computers()[1].clone(), false, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        let reads = action_trace(&fake, "catalog").len();
        fake.queue_result("catalog", Ok(library_catalog(2)));
        window
            .update(&mut visual, |root, window, cx| root.open_roles(window, cx))
            .unwrap();
        for _ in 0..1000 {
            if action_trace(&fake, "catalog").len() > reads {
                break;
            }
            assert!(visual.background_executor.tick());
        }
        assert_eq!(action_trace(&fake, "catalog").len(), reads + 1);
        window
            .read_with(&visual, |root, _| {
                assert!(root
                    .remote_window
                    .as_ref()
                    .unwrap()
                    .library_catalog
                    .is_none())
            })
            .unwrap();
        if return_local {
            window
                .update(&mut visual, |root, window, cx| {
                    root.choose_computer(computers()[0].clone(), false, window, cx)
                })
                .unwrap();
        } else {
            fake.queue_result("catalog", Ok(library_catalog(3)));
            window
                .update(&mut visual, |root, _, cx| root.refresh_remote_library(cx))
                .unwrap();
        }
        visual.run_until_parked();
        if return_local {
            window
                .read_with(&visual, |root, _| assert!(root.remote_window.is_none()))
                .unwrap();
            assert!(visual.debug_bounds("ROLE_ROW:host-role-0").is_none());
        } else {
            assert!(visual.debug_bounds("ROLE_ROW:host-role-2").is_some());
            window
                .read_with(&visual, |root, _| {
                    assert_eq!(
                        root.remote_window
                            .as_ref()
                            .unwrap()
                            .library_catalog
                            .as_ref()
                            .unwrap()
                            .roles
                            .len(),
                        3
                    )
                })
                .unwrap();
        }
    }
}

#[test]
fn remote_role_library_does_not_replace_a_removed_role_with_another() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let window = add_window(&mut cx, &store, temp.path(), "missing-library-role");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    fake.queue_result("catalog", Ok(library_catalog(2)));
    window
        .update(&mut cx, |root, window, cx| root.open_roles(window, cx))
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    fake.queue_result("catalog", Ok(library_catalog(1)));
    let chat = visual.debug_bounds("ROLE_ROW_CHAT:host-role-1").unwrap();
    visual.simulate_click(chat.center(), Default::default());
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| {
            assert!(root.start_chat_modal.is_none());
            assert_eq!(
                root.remote_window.as_ref().unwrap().action_error.as_deref(),
                Some(crate::i18n::text("client.role_unavailable"))
            );
        })
        .unwrap();
    assert!(action_trace(&fake, "startChat").is_empty());
}

#[test]
fn remote_library_catalog_replies_are_scoped_to_host_and_connection_generation() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for change_host in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, clients) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        let window = add_window(&mut cx, &store, temp.path(), "library-target-race");
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        let old = window
            .read_with(&cx, |root, _| {
                root.remote_window
                    .as_ref()
                    .unwrap()
                    .front_page
                    .target
                    .clone()
            })
            .unwrap();
        let reads = action_trace(&fake, "catalog").len();
        fake.queue_result("catalog", Ok(library_catalog(2)));
        window
            .update(&mut cx, |root, window, cx| root.open_roles(window, cx))
            .unwrap();
        for _ in 0..1000 {
            if action_trace(&fake, "catalog").len() > reads {
                break;
            }
            assert!(cx.background_executor.tick());
        }
        assert_eq!(action_trace(&fake, "catalog").len(), reads + 1);
        if change_host {
            let mut other = computers()[1].clone();
            other.id = "other-host".into();
            other.name = "Other host".into();
            other.fingerprint = "other-host-key".into();
            fake.computers.lock().unwrap().push(other.clone());
            window
                .update(&mut cx, |root, window, cx| {
                    root.choose_computer(other, false, window, cx)
                })
                .unwrap();
        } else {
            clients.update(&mut cx, |clients, _| {
                clients.invalidate_capabilities("host")
            });
            window
                .update(&mut cx, |root, _, cx| root.sync_remote_state(cx))
                .unwrap();
        }
        for _ in 0..4 {
            fake.queue_result("catalog", Ok(library_catalog(3)));
        }
        cx.run_until_parked();
        if change_host {
            window
                .update(&mut cx, |root, window, cx| root.open_roles(window, cx))
                .unwrap();
            cx.run_until_parked();
        }
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        assert!(visual.debug_bounds("ROLE_ROW:host-role-2").is_some());
        window
            .read_with(&visual, |root, _| {
                let remote = root.remote_window.as_ref().unwrap();
                assert_ne!(remote.front_page.target, old);
                assert_eq!(remote.library_catalog.as_ref().unwrap().roles.len(), 3);
                assert_eq!(
                    remote.computer.id,
                    if change_host { "other-host" } else { "host" }
                );
            })
            .unwrap();
    }
}

#[test]
fn remote_libraries_fit_narrow_zoomed_windows_in_both_languages() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, _) = fixture(&mut cx, temp.path());
            enable_front_page(&fake);
            let window = add_window(&mut cx, &store, temp.path(), "library-geometry");
            window.update(&mut cx, choose_host).unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            for crews in [false, true] {
                fake.queue_result("catalog", Ok(library_catalog(2)));
                window
                    .update(&mut visual, |root, window, cx| {
                        if crews {
                            root.open_crews(window, cx);
                        } else {
                            root.open_roles(window, cx);
                        }
                    })
                    .unwrap();
                visual.run_until_parked();
                for zoom in [1., 1.3] {
                    for width in [640., 1100.] {
                        visual.simulate_resize(gpui::size(px(width), px(900.)));
                        window
                            .update(&mut visual, |root, window, cx| {
                                root.set_zoom(zoom, window, cx)
                            })
                            .unwrap();
                        visual.run_until_parked();
                        for selector in [
                            "REMOTE_LIBRARY_REFRESH",
                            "PAGINATED_LIST_FOOTER",
                            if crews {
                                "CREW_TABLE_ROWS"
                            } else {
                                "ROLE_TABLE_ROWS"
                            },
                        ] {
                            let bounds = visual.debug_bounds(selector).unwrap();
                            assert!(
                                bounds.left() >= px(0.) && bounds.right() <= px(width + 1.),
                                "{locale:?} {crews} {width} {zoom}: {selector} {bounds:?}"
                            );
                        }
                        assert!(visual.debug_bounds("NEW_ROLE").is_none());
                        assert!(visual.debug_bounds("NEW_CREW").is_none());
                    }
                }
            }
        });
    }
}

#[test]
fn remote_terminal_keyboard_copy_interrupt_and_paste_follow_platform_bindings() {
    use alacritty_terminal::{
        index::{Column, Line, Point, Side},
        selection::SelectionType,
    };
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        crate::keymap::install_bindings(cx, &crate::keymap::KeymapOverrides::new(), false)
    });
    let (store, fake, _) = fixture(&mut cx, temp.path());
    let window = add_window(&mut cx, &store, temp.path(), "remote-clipboard");
    window.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let terminal = window
        .update(&mut visual, |root, window, cx| {
            window.activate_window();
            root.focus_remote_terminal("s", window, cx);
            root.remote_window.as_ref().unwrap().terminals["s"]
                .terminal
                .clone()
        })
        .unwrap();
    visual.run_until_parked();
    fake.input.lock().unwrap().clear();
    terminal.start_selection(
        SelectionType::Simple,
        Point::new(Line(0), Column(0)),
        Side::Left,
    );
    terminal.update_selection(Point::new(Line(0), Column(5)), Side::Right);
    let selection = terminal.selection_text().unwrap();
    visual.simulate_keystrokes(&crate::keymap::platform_default("cmd-c"));
    assert_eq!(
        visual.update(|_, cx| cx.read_from_clipboard().unwrap().text()),
        Some(selection)
    );
    assert!(fake.input.lock().unwrap().is_empty());
    terminal.clear_selection();
    visual.simulate_keystrokes("ctrl-c");
    visual.run_until_parked();
    assert_eq!(fake.input.lock().unwrap().last().unwrap().1, b"\x03");
    visual.update(|_, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("remote paste".into()))
    });
    visual.simulate_keystrokes(&crate::keymap::platform_default("cmd-v"));
    visual.run_until_parked();
    assert_eq!(
        fake.input.lock().unwrap().last().unwrap().1,
        b"remote paste"
    );
}
