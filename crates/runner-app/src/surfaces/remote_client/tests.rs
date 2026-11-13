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
    remote::{
        DeviceKind, PairedDevice, RemoteAccount, RemoteState, RemoteStatus, RemoteTerminal,
        StopReason,
    },
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
    local_usage: Mutex<runner_core::protocol::usage::UsageSnapshot>,
    overview_calls: AtomicUsize,
    follow_push: AtomicBool,
    events: Mutex<Vec<futures::channel::mpsc::UnboundedSender<ClientEvent>>>,
    follows: Mutex<Vec<(String, bool)>>,
    pairs: AtomicUsize,
    pair_pins: Mutex<Vec<runner_core::remote::Secret>>,
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
            "archive" if value["target"]["kind"] == "mission" => {
                let id = value["target"]["id"].as_str().unwrap();
                overview.missions.retain(|mission| mission.id != id);
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
    pub(crate) fn without_features(&self, omit: &[&str]) {
        self.features
            .lock()
            .unwrap()
            .retain(|feature| !omit.contains(&feature.as_str()));
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

    pub(crate) fn attach_count(&self) -> usize {
        self.attaches.lock().unwrap().len()
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
            Request::usage_snapshot {} => {
                Response::usage_snapshot(Ok(self.local_usage.lock().unwrap().clone()))
            }
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
                    "usage" => host_usage(),
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
                self.pair_pins.lock().unwrap().push(pin);
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
pub(crate) fn overview() -> Overview {
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
        pair_pins: Mutex::default(),
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
        local_usage: Mutex::new(Default::default()),
        features: Mutex::new(Vec::new()),
        action_requests: Mutex::new(Vec::new()),
        local_requests: Mutex::new(Vec::new()),
        action_results: Mutex::new(HashMap::new()),
        model_session_removal: AtomicBool::new(false),
    });
    let clients = cx.new(|cx| model::ClientState::new(fake.client(), cx));
    store.update(cx, |store, _| {
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
        kind: Default::default(),
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
fn a_phone_never_matches_an_account_computer_by_name() {
    let computers = computers();
    let phone = PairedDevice {
        kind: DeviceKind::Phone,
        ..device("Studio", "phone-key")
    };
    let trust = RemoteTrust {
        devices: vec![phone.clone()],
        ..Default::default()
    };
    let rows = model::computer_rows(&computers, &trust);
    assert!(rows[1].access == Access::Outgoing);
    assert!(rows[1].devices.is_empty());
    assert!(rows[2].computer.is_none());
    assert_eq!(rows[2].devices[0].fingerprint, "phone-key");
    assert!(model::incoming_devices(&computers[1], &computers, &trust).is_empty());

    let trust = RemoteTrust {
        devices: vec![phone, device("Studio", "host-client")],
        ..Default::default()
    };
    let rows = model::computer_rows(&computers, &trust);
    assert!(rows[1].access == Access::Both);
    assert_eq!(rows[1].devices[0].fingerprint, "host-client");
    assert_eq!(rows[2].devices[0].fingerprint, "phone-key");
}

#[test]
fn a_device_outside_the_account_shows_its_pairing_name_a_kind_icon_and_a_relative_status() {
    let phone = PairedDevice {
        kind: DeviceKind::Phone,
        connected: false,
        last_seen_at: chrono::Utc::now() - chrono::Duration::days(5),
        ..device("Jason's iPhone", "9134 2aa5 19ea 589a 15fc")
    };
    for (locale, seen) in [
        (Locale::English, "Offline · seen 5 days ago"),
        (Locale::SimplifiedChinese, "离线 · 最后在线 5 天前"),
    ] {
        with_locale(locale, || {
            assert_eq!(device_status(&phone, false), seen);
            assert_eq!(
                device_status(&phone, true),
                crate::i18n::text("client.connected_here")
            );
        });
    }
    assert_eq!(model::device_title(&phone), "Jason's iPhone");
    assert_eq!(model::device_icon(&phone), "smartphone.svg");
    for kind in [DeviceKind::Unknown, DeviceKind::Computer] {
        assert_eq!(
            model::device_icon(&PairedDevice {
                kind,
                ..phone.clone()
            }),
            "monitor.svg"
        );
    }
    let unnamed = device("", "630d cd29 66c4 3366 9112");
    assert_eq!(model::device_title(&unnamed), "630d cd29 66c4 3366 9112");
}

#[test]
fn cached_row_menus_follow_a_language_change() {
    use crate::surfaces::settings::page::SettingsPane;
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, _, clients) = fixture(&mut cx, temp.path());
    let window = add_window(&mut cx, &store, temp.path(), "menu-locale");
    window
        .update(&mut cx, |root, _, cx| {
            root.route = crate::AppRoute::Settings;
            root.settings_page.pane = SettingsPane::Remote;
            cx.notify();
        })
        .unwrap();
    let now = chrono::Utc::now();
    store.update(&mut cx, |store, cx| {
        store.remote_status = Some(RemoteStatus {
            state: RemoteState::Connected { since: now },
            account: Some(RemoteAccount {
                username: "jason".into(),
                display_name: "Jason".into(),
            }),
            host_name: Some("Laptop".into()),
            ..Default::default()
        });
        store.remote_trust = Some(RemoteTrust {
            pin_set: true,
            devices: vec![PairedDevice {
                kind: DeviceKind::Phone,
                ..device("Jason's iPhone", "9134 2aa5 19ea 589a 15fc")
            }],
            ..Default::default()
        });
        cx.notify();
    });
    clients.update(&mut cx, |clients, cx| clients.refresh(cx));
    cx.run_until_parked();
    // The paired computer's menu holds Forget; the phone's holds its fingerprint above Revoke.
    let render = |cx: &mut TestAppContext, locale| {
        with_locale(locale, || {
            window.update(cx, |_, _, cx| cx.notify()).unwrap();
            cx.run_until_parked();
            let menus: Vec<_> = window
                .read_with(cx, |root, _| {
                    root.remote_client
                        .ui
                        .menus
                        .borrow()
                        .values()
                        .cloned()
                        .collect()
                })
                .unwrap();
            let mut labels: Vec<_> = menus
                .iter()
                .map(|menu| menu.read_with(cx, |menu, _| menu.labels()))
                .collect();
            labels.sort();
            (menus, labels)
        })
    };
    let (english_menus, english) = render(&mut cx, Locale::English);
    assert_eq!(english, [vec!["Fingerprint", "Revoke"], vec!["Forget"]]);
    let (chinese_menus, chinese) = render(&mut cx, Locale::SimplifiedChinese);
    assert_eq!(chinese, [vec!["忘记"], vec!["指纹", "撤销"]]);
    let mut kept: Vec<_> = english_menus.iter().map(|menu| menu.entity_id()).collect();
    let mut again: Vec<_> = chinese_menus.iter().map(|menu| menu.entity_id()).collect();
    kept.sort();
    again.sort();
    assert_eq!(
        kept, again,
        "the language change must reuse the cached menus"
    );
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
    fake.emit("remote/connection", serde_json::json!({"computer":"host","state":"reconnecting","error":"Remote channel: unknownDevice."}));
    cx.run_until_parked();
    assert!(clients.read_with(&cx, |clients, _| clients.connection("host")) == Connection::Revoked);
    assert_eq!(
        *fake.follows.lock().unwrap(),
        vec![("host".into(), true), ("host".into(), false)]
    );
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
                (
                    "Remote route closed: offline (4409).",
                    pairing::PairState::Offline,
                ),
                (
                    "Pairing timed out; enter the PIN again to retry.",
                    pairing::PairState::TimedOut,
                ),
                (
                    "Remote service is unavailable",
                    pairing::PairState::Unreachable,
                ),
                (
                    "Remote route refused: unavailable (HTTP 503).",
                    pairing::PairState::Unreachable,
                ),
                (
                    "Remote route closed: notFound (4404).",
                    pairing::PairState::Unreachable,
                ),
                ("Remote route closed", pairing::PairState::Unreachable),
                (
                    "Could not list computers: unavailable",
                    pairing::PairState::Unreachable,
                ),
            ] {
                *fake.refusal.lock().unwrap() = Some(reason.into());
                let before = fake.pairs.load(Ordering::SeqCst);
                window
                    .update(&mut cx, |root, window, cx| {
                        root.remote_client.open_computer_pair(
                            computers()[1].clone(),
                            pairing::Purpose::PairOnly,
                            window,
                            cx,
                        );
                        let field = root.remote_client.ui.pair.as_ref().unwrap().field.clone();
                        field.update(cx, |field, cx| {
                            field.replace_text_in_range(None, "001234", window, cx)
                        });
                        root.remote_client.submit_computer_pin(window, cx);
                        root.remote_client.submit_computer_pin(window, cx);
                        assert!(
                            root.remote_client.ui.pair.as_ref().unwrap().state
                                == pairing::PairState::Checking
                        );
                    })
                    .unwrap();
                cx.run_until_parked();
                assert_eq!(fake.pairs.load(Ordering::SeqCst), before + 1);
                let retained = window
                    .read_with(&cx, |root, cx| {
                        let dialog = root.remote_client.ui.pair.as_ref().unwrap();
                        assert!(dialog.state == expected);
                        let field = dialog.field.read(cx);
                        assert!(!field.disabled);
                        assert_eq!(field.invalid, expected == pairing::PairState::WrongPin);
                        assert!(
                            field.value().0
                                == if expected == pairing::PairState::WrongPin {
                                    ""
                                } else {
                                    "001234"
                                }
                        );
                        dialog.field.clone()
                    })
                    .unwrap();
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                assert!(visual.debug_bounds("COMPUTER_PAIR_DIALOG").is_some());
                if matches!(
                    expected,
                    pairing::PairState::Offline
                        | pairing::PairState::TimedOut
                        | pairing::PairState::Unreachable
                        | pairing::PairState::Failed
                ) {
                    assert_eq!(
                        visual.debug_bounds("COMPUTER_PAIR_PIN").is_none(),
                        expected == pairing::PairState::Offline
                    );
                    for action in ["enter", "mouse"] {
                        let before = fake.pairs.load(Ordering::SeqCst);
                        if action == "enter" {
                            cx.simulate_keystrokes(window.into(), "enter");
                        } else {
                            let button = visual.debug_bounds("COMPUTER_PAIR_PRIMARY").unwrap();
                            visual.simulate_click(button.center(), Default::default());
                        }
                        cx.run_until_parked();
                        assert_eq!(fake.pairs.load(Ordering::SeqCst), before + 1);
                        assert!(fake.pair_pins.lock().unwrap().last().unwrap().0 == "001234");
                        window
                            .read_with(&cx, |root, cx| {
                                let dialog = root.remote_client.ui.pair.as_ref().unwrap();
                                assert_eq!(dialog.field, retained);
                                assert!(dialog.field.read(cx).value().0 == "001234");
                                assert!(dialog.state == expected);
                            })
                            .unwrap();
                    }
                    if expected != pairing::PairState::Offline {
                        window
                            .update(&mut cx, |_, window, cx| {
                                retained.update(cx, |field, cx| {
                                    field.replace_text_in_range(Some(0..6), "00123", window, cx);
                                })
                            })
                            .unwrap();
                        let before = fake.pairs.load(Ordering::SeqCst);
                        cx.simulate_keystrokes(window.into(), "enter");
                        let button = visual.debug_bounds("COMPUTER_PAIR_PRIMARY").unwrap();
                        visual.simulate_click(button.center(), Default::default());
                        cx.run_until_parked();
                        assert_eq!(fake.pairs.load(Ordering::SeqCst), before);
                    }
                }
            }
            *fake.refusal.lock().unwrap() = None;
            let before = fake.pairs.load(Ordering::SeqCst);
            window
                .update(&mut cx, |root, window, cx| {
                    state(&root.app_store, cx).update(cx, |clients, _| {
                        clients.changed_identities.insert("host".into());
                    });
                    root.remote_client.open_computer_pair(
                        computers()[1].clone(),
                        pairing::Purpose::Open { new_window: false },
                        window,
                        cx,
                    );
                    let field = root.remote_client.ui.pair.as_ref().unwrap().field.clone();
                    field.update(cx, |field, cx| {
                        field.replace_text_in_range(None, "001234", window, cx)
                    });
                    root.remote_client.submit_computer_pin(window, cx);
                    root.remote_client.ui.pair = None;
                })
                .unwrap();
            cx.run_until_parked();
            assert_eq!(fake.pairs.load(Ordering::SeqCst), before + 1);
            assert!(fake.pair_ops.lock().unwrap().ends_with(&["forget", "pair"]));
            window
                .read_with(&cx, |root, _| assert!(root.remote_client.is_local()))
                .unwrap();
        });
    }
}

#[test]
fn offline_pairing_gates_every_entry_point_and_refreshes_without_submitting() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            for entry in ["picker", "settings", "workspace", "workspace-again"] {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let (store, fake, clients) = fixture(&mut cx, temp.path());
                let window = add_window(&mut cx, &store, temp.path(), "offline-pair");
                if entry.starts_with("workspace") {
                    window.update(&mut cx, choose_host).unwrap();
                    cx.run_until_parked();
                }
                fake.computers.lock().unwrap()[1].online = false;
                fake.computers.lock().unwrap()[1].paired = false;
                clients.update(&mut cx, |clients, cx| {
                    if entry.starts_with("workspace") {
                        clients.connections.insert(
                            "host".into(),
                            if entry == "workspace" {
                                Connection::Forgotten
                            } else {
                                Connection::Revoked
                            },
                        );
                    }
                    clients.refresh(cx);
                });
                cx.run_until_parked();
                if entry == "picker" {
                    let computer = fake.computers.lock().unwrap()[1].clone();
                    window
                        .update(&mut cx, |root, window, cx| {
                            root.choose_computer(computer, false, window, cx)
                        })
                        .unwrap();
                } else {
                    if entry == "settings" {
                        window
                            .update(&mut cx, |root, _, cx| {
                                root.route = crate::AppRoute::Settings;
                                root.settings_page.pane =
                                    crate::surfaces::settings::page::SettingsPane::Remote;
                                cx.notify();
                            })
                            .unwrap();
                    }
                    cx.run_until_parked();
                    let mut visual = VisualTestContext::from_window(window.into(), &cx);
                    let button = visual
                        .debug_bounds(if entry == "settings" {
                            "COMPUTER_PAIR_ACTION"
                        } else {
                            "REMOTE_RECONNECT_ACTION"
                        })
                        .unwrap();
                    visual.simulate_click(button.center(), Default::default());
                }
                cx.run_until_parked();
                window
                    .update(&mut cx, |root, window, cx| {
                        let dialog = root.remote_client.ui.pair.as_ref().unwrap();
                        assert!(dialog.state == pairing::PairState::Offline, "{entry}");
                        assert!(!dialog.field.read(cx).focus.is_focused(window));
                    })
                    .unwrap();
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                assert!(visual.debug_bounds("COMPUTER_PAIR_PIN").is_none());
                let before = fake.directory_calls.load(Ordering::SeqCst);
                cx.simulate_keystrokes(window.into(), "enter");
                cx.run_until_parked();
                assert!(fake.directory_calls.load(Ordering::SeqCst) > before);
                assert_eq!(fake.pairs.load(Ordering::SeqCst), 0);
                fake.computers.lock().unwrap()[1].online = true;
                clients.update(&mut cx, |clients, cx| clients.refresh(cx));
                cx.run_until_parked();
                window
                    .update(&mut cx, |root, window, cx| {
                        let dialog = root.remote_client.ui.pair.as_ref().unwrap();
                        assert!(dialog.state == pairing::PairState::Entry);
                        assert!(dialog.field.read(cx).focus.is_focused(window));
                    })
                    .unwrap();
                assert!(visual.debug_bounds("COMPUTER_PAIR_PIN").is_some());
                assert_eq!(fake.pairs.load(Ordering::SeqCst), 0);
            }
        });
    }
}

#[test]
fn online_listing_refresh_preserves_submit_time_offline_pin_and_never_retries_it() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    let window = add_window(&mut cx, &store, temp.path(), "kept-offline-pin");
    *fake.refusal.lock().unwrap() = Some("Remote route closed: offline (4409).".into());
    window
        .update(&mut cx, |root, window, cx| {
            root.remote_client.open_computer_pair(
                computers()[1].clone(),
                pairing::Purpose::PairOnly,
                window,
                cx,
            );
            root.remote_client
                .ui
                .pair
                .as_ref()
                .unwrap()
                .field
                .update(cx, |field, cx| {
                    field.replace_text_in_range(None, "001234", window, cx)
                });
            root.remote_client.submit_computer_pin(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    clients.update(&mut cx, |clients, cx| clients.refresh(cx));
    cx.run_until_parked();
    window
        .read_with(&cx, |root, cx| {
            let dialog = root.remote_client.ui.pair.as_ref().unwrap();
            assert!(dialog.state == pairing::PairState::Offline);
            assert!(dialog.field.read(cx).value().0 == "001234");
        })
        .unwrap();
    assert_eq!(fake.pairs.load(Ordering::SeqCst), 1);
    *fake.refusal.lock().unwrap() = None;
    cx.simulate_keystrokes(window.into(), "enter");
    cx.run_until_parked();
    assert_eq!(fake.pairs.load(Ordering::SeqCst), 2);
    window
        .read_with(&cx, |root, _| assert!(root.remote_client.ui.pair.is_none()))
        .unwrap();
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
        "SIDEBAR_WORKSPACE_NEW_TAB",
        "SIDEBAR_WORKSPACE:workspace-role",
        "SIDEBAR_WORKSPACE:workspace-crew",
    ] {
        assert!(visual.debug_bounds(selector).is_none());
        window
            .read_with(&visual, |root, _| {
                assert!(root.remote_client.is_active());
                assert!(root.route == original.0);
                assert!(root.start_chat_modal.is_none());
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
    let project = visual.debug_bounds("SIDEBAR_PROJECT:p").unwrap();
    visual.simulate_click(project.center(), Default::default());
    visual.run_until_parked();
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
                startup_error: None,
                status: status.clone(),
                reason: reason.clone(),
                unread,
                error_acknowledged: false,
                interrupted_since: None,
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
    let interrupted = model::agent_status(&SessionStatus {
        startup_error: None,
        status: Status::Idle,
        reason: None,
        unread: false,
        error_acknowledged: false,
        interrupted_since: Some(5),
    });
    assert_eq!(
        StatusPresentation::new(&interrupted).kind,
        StatusKind::Interrupted
    );
    assert_eq!(SessionStatus::from(&interrupted).interrupted_since, Some(5));
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
                    startup_error: None,
                    status,
                    reason,
                    unread,
                    error_acknowledged: false,
                    interrupted_since: None,
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
        for (status, reason, unread, acknowledged, glyph) in [
            ("needsYou", Some("approval"), true, false, Some("Approval")),
            ("needsYou", Some("answer"), false, false, Some("Answer")),
            ("needsYou", None, false, false, Some("NeedsYou")),
            ("needsYou", Some("future"), false, false, Some("NeedsYou")),
            ("future", None, false, false, None),
            ("working", None, true, false, Some("Working")),
            ("idle", None, true, false, Some("Unread")),
            ("error", None, true, false, Some("Error")),
            ("error", None, false, true, None),
            ("error", None, false, false, Some("Error")),
            ("starting", None, false, false, None),
            ("stopped", None, false, false, None),
            ("idle", None, false, false, None),
        ] {
            data.sessions.get_mut("s").unwrap().status = serde_json::from_value(
                serde_json::json!({"status": status, "reason": reason, "unread": unread, "errorAcknowledged": acknowledged}),
            )
            .unwrap();
            if status == "working" || reason == Some("answer") || acknowledged {
                fake.emit(
                    "remote/session-status",
                    serde_json::json!({"computer":"host","status":{"session":"s","status":status,"reason":reason,"unread":unread,"errorAcknowledged":acknowledged}}),
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
                        root.remote_client.workspace_mut().unwrap().active = None;
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
fn remote_error_acknowledgement_preserves_local_rollup_priority() {
    use runner_app::ui::agent_status::StatusRollup;
    use runner_core::protocol::{AgentStatus, Lifecycle};
    for error_since in [Some(100), None] {
        let local = AgentStatus {
            lifecycle: Lifecycle::Error,
            error_since,
            ..Default::default()
        };
        let wire = runner_core::companion::SessionStatus::from(&local);
        let remote = model::agent_status(&wire);
        assert_eq!(remote.lifecycle, local.lifecycle);
        assert_eq!(
            StatusRollup::priority(&remote),
            StatusRollup::priority(&local)
        );
    }
}

#[test]
fn selecting_a_stopped_remote_chat_acknowledges_only_its_visible_attention() {
    for supported in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        if supported {
            fake.features.lock().unwrap().push("markViewed".into());
        }
        let mut data = overview();
        data.sessions.get_mut("s").unwrap().status.status = Status::Error;
        data.sessions
            .insert("hidden".into(), data.sessions["s"].clone());
        fake.set_overview(data.clone());
        cx.run_until_parked();
        let window = add_window(&mut cx, &store, temp.path(), "acknowledge-remote-error");
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        assert_eq!(fake.request_count("markViewed"), 0);
        window
            .update(&mut cx, |root, _, cx| {
                root.select_remote_item(workspace::Item::Tab("remote-tab".into()), cx);
            })
            .unwrap();
        cx.run_until_parked();
        let requests = action_trace(&fake, "markViewed");
        assert_eq!(requests.len(), usize::from(supported));
        if supported {
            assert_eq!(requests[0]["session"], "s");
        }
        assert!(fake.attaches.lock().unwrap().is_empty());
        assert_eq!(fake.request_count("resume"), 0);
        assert_eq!(fake.request_count("restart"), 0);

        data.sessions
            .get_mut("s")
            .unwrap()
            .status
            .error_acknowledged = true;
        data.sessions.get_mut("s").unwrap().status.unread = false;
        fake.set_overview(data);
        cx.run_until_parked();
        window
            .update(&mut cx, |root, _, cx| {
                root.select_remote_item(workspace::Item::Tab("remote-tab".into()), cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(fake.request_count("markViewed"), requests.len());
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
    let project = visual.debug_bounds("SIDEBAR_PROJECT:p").unwrap();
    visual.simulate_click(project.center(), Default::default());
    visual.run_until_parked();
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
    assert!(visual.debug_bounds("PANE_HEADER:s").is_some());
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
                "PANE_TITLE:Project chat title",
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
                .debug_bounds("PANE_TITLE:Updated chat title")
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
                .debug_bounds("PANE_TITLE:Updated chat title")
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
                .debug_bounds("PANE_TITLE:Recent chat title")
                .is_some());
            data.tabs[0].panes = runner_core::companion::Pane::Leaf { session: None };
            fake.emit(
                "remote/overview",
                serde_json::json!({"computer":"host","overview":data}),
            );
            visual.run_until_parked();
            let empty = match locale {
                Locale::English => "PANE_TITLE:Empty tab",
                Locale::SimplifiedChinese => "PANE_TITLE:空标签页",
            };
            assert!(visual.debug_bounds("START_SURFACE").is_some());
            window
                .read_with(&visual, |root, cx| {
                    assert_eq!(
                        root.tab_label(root.front_page_layout().unwrap(), cx),
                        empty.trim_start_matches("PANE_TITLE:")
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
            assert!(visual.debug_bounds("PANE_TITLE:Codex").is_some());
        });
    }
}

#[test]
fn signing_out_returns_every_window_to_this_computer() {
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
            .read_with(&cx, |root, _| assert!(root.remote_client.is_active()))
            .unwrap();
    }
    *fake.status.lock().unwrap() = RemoteStatus::default();
    fake.emit(
        "remote/status",
        serde_json::to_value(RemoteStatus::default()).unwrap(),
    );
    store.update(&mut cx, |store, cx| {
        store.remote_status = Some(RemoteStatus::default());
        cx.notify();
    });
    cx.run_until_parked();
    for window in [first, second] {
        window
            .read_with(&cx, |root, _| {
                assert!(root.remote_client.is_local());
                assert!(!root.remote_client.ui.picker_open);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        assert!(visual.debug_bounds("SIDEBAR_HOST_BUTTON").is_some());
        assert!(visual.debug_bounds("HOST_BUTTON_ATTENTION").is_none());
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
                assert!(visual.debug_bounds("REMOTE_CONNECTION_ROW").is_none());
                assert!(visual.debug_bounds("SIDEBAR_HOST_BUTTON").is_some());
                assert!(visual.debug_bounds("SIDEBAR_SETTINGS").is_some());
                crate::theme_snapshot::assert_border(
                    &mut visual,
                    "SIDEBAR_HOST_BUTTON",
                    theme::with_alpha(theme::accent(), 0.4),
                );
                assert!(visual.debug_bounds("REMOTE_TAKEN_BACK").is_some());
                assert!(visual.debug_bounds("REMOTE_RECONNECTING_PILL").is_none());
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
                window
                    .update(&mut cx, |root, _, cx| {
                        root.remote_client.ui.picker_open = true;
                        cx.notify();
                    })
                    .unwrap();
                cx.run_until_parked();
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                if expected == Connection::Reconnecting {
                    let pill = visual.debug_bounds("REMOTE_RECONNECTING_PILL").unwrap();
                    let viewport = visual.update(|window, _| window.viewport_size());
                    assert_eq!(pill.size.height, px(32.));
                    assert!(pill.size.width <= px(600.) && pill.size.width < viewport.width);
                    crate::theme_snapshot::assert_fill_color(
                        &mut visual,
                        "REMOTE_RECONNECTING_PILL",
                        Some(theme::panel()),
                    );
                    crate::theme_snapshot::assert_border(
                        &mut visual,
                        "REMOTE_RECONNECTING_PILL",
                        theme::with_alpha(theme::warning(), 0.4),
                    );
                } else {
                    assert!(visual.debug_bounds("REMOTE_RECONNECTING_PILL").is_none());
                }
                crate::theme_snapshot::assert_border(
                    &mut visual,
                    "SIDEBAR_HOST_BUTTON",
                    theme::with_alpha(picker::host_tone(expected, false), 0.4),
                );
                assert!(visual.debug_bounds("REMOTE_CONNECTION_ROW").is_none());
                assert!(visual.debug_bounds("COMPUTER_PICKER_CONNECTION").is_some());
                assert!(visual.debug_bounds("COMPUTER_PICKER_CHECK").is_some());
                assert!(visual.debug_bounds("COMPUTER_ROW_ATTENTION").is_none());
                window
                    .update(&mut cx, |root, _, cx| {
                        root.remote_client.ui.picker_open = false;
                        cx.notify();
                    })
                    .unwrap();
                cx.run_until_parked();
            }
            window
                .update(&mut cx, |root, _, cx| {
                    root.remote_client.ui.picker_open = true;
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
                    assert!(root.remote_client.is_local());
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
fn host_button_sits_in_the_footer_and_the_popup_puts_this_computer_first() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    let [this, host] = <[Computer; 2]>::try_from(computers()).ok().unwrap();
    let mut this = this;
    this.info.hostname = "Jasons-MacBook-Air".into();
    let mut host = host;
    host.info.hostname = "build-server".into();
    *fake.computers.lock().unwrap() = vec![host.clone(), this.clone()];
    clients.update(&mut cx, |clients, cx| clients.refresh(cx));
    cx.run_until_parked();
    let window = add_window(&mut cx, &store, temp.path(), "host-button");
    window
        .update(&mut cx, |root, _, cx| {
            root.usage_installed = vec![runner_core::protocol::model::Runtime::Codex];
            cx.notify();
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    visual.run_until_parked();
    for remote in [false, true] {
        let footer = visual.debug_bounds("SIDEBAR_FOOTER_ROW").unwrap();
        let usage = visual.debug_bounds("SIDEBAR_USAGE");
        let status = visual.debug_bounds("SIDEBAR_LINK_STATUS");
        assert_eq!(usage.is_some(), !remote);
        assert!(status.is_none());
        let left = usage;
        let button = visual.debug_bounds("SIDEBAR_HOST_BUTTON").unwrap();
        let gear = visual.debug_bounds("SIDEBAR_SETTINGS").unwrap();
        assert!(left.is_none_or(|left| left.right() <= button.left()));
        assert!(button.right() <= gear.left());
        assert_eq!(button.size, gear.size);
        assert_eq!(button.top(), gear.top());
        assert!(button.top() >= footer.top() && button.bottom() <= footer.bottom());
        assert!(visual.debug_bounds("REMOTE_CONNECTION_ROW").is_none());
        crate::theme_snapshot::assert_fill_color(
            &mut visual,
            "SIDEBAR_HOST_BUTTON",
            remote.then(|| theme::with_alpha(theme::accent(), 0.08)),
        );
        visual.simulate_click(button.center(), Default::default());
        visual.run_until_parked();
        if !remote {
            crate::theme_snapshot::assert_fill_color(
                &mut visual,
                "SIDEBAR_HOST_BUTTON",
                Some(theme::chrome_selected()),
            );
        }
        let panel = visual.debug_bounds("COMPUTER_PICKER_PANEL").unwrap();
        assert!(panel.bottom() < button.top());
        let this_row = visual.debug_bounds("COMPUTER_PICKER_ROW").unwrap();
        let divider = visual.debug_bounds("COMPUTER_PICKER_DIVIDER").unwrap();
        let remote_row = visual.debug_bounds("COMPUTER_PICKER_REMOTE_ROW").unwrap();
        let settings = visual.debug_bounds("COMPUTER_PICKER_SETTINGS").unwrap();
        assert!(panel.contains(&this_row.origin));
        assert!(this_row.top() < remote_row.top());
        assert!(this_row.bottom() <= divider.top());
        assert!(divider.bottom() <= remote_row.top());
        assert!(remote_row.bottom() <= settings.top());
        crate::theme_snapshot::assert_fill_color(
            &mut visual,
            "COMPUTER_PICKER_SELF_TILE",
            Some(theme::with_alpha(theme::accent(), 0.14)),
        );
        let check = visual.debug_bounds("COMPUTER_PICKER_CHECK").unwrap();
        let checked = if remote { remote_row } else { this_row };
        assert!(checked.contains(&check.center()));
        assert_eq!(
            visual.debug_bounds("COMPUTER_PICKER_CONNECTION").is_some(),
            remote
        );
        if remote {
            visual.simulate_click(this_row.center(), Default::default());
            visual.run_until_parked();
            window
                .read_with(&visual, |root, _| assert!(root.remote_client.is_local()))
                .unwrap();
            assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
        } else {
            visual.simulate_click(remote_row.center(), Default::default());
            visual.run_until_parked();
            window
                .read_with(&visual, |root, _| assert!(root.remote_client.is_active()))
                .unwrap();
        }
    }
    assert_eq!(picker::computer_glyph(&this), "laptop.svg");
    assert_eq!(picker::computer_glyph(&host), "server.svg");
    assert_eq!(picker::computer_glyph(&computers()[1]), "monitor.svg");
}

#[test]
fn remote_access_settings_row_opens_this_computers_remote_pane() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for remote in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, _, _) = fixture(&mut cx, temp.path());
        let window = add_window(&mut cx, &store, temp.path(), "picker-settings");
        if remote {
            window.update(&mut cx, choose_host).unwrap();
        }
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.run_until_parked();
        let button = visual.debug_bounds("SIDEBAR_HOST_BUTTON").unwrap();
        visual.simulate_click(button.center(), Default::default());
        visual.run_until_parked();
        let row = visual.debug_bounds("COMPUTER_PICKER_SETTINGS").unwrap();
        visual.simulate_click(row.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
        window
            .read_with(&visual, |root, _| {
                assert!(root.remote_client.is_local());
                assert_eq!(root.route, AppRoute::Settings);
                assert_eq!(
                    root.settings_page.pane,
                    crate::surfaces::settings::page::SettingsPane::Remote
                );
            })
            .unwrap();
    }
}

#[test]
fn popup_keeps_the_shown_computer_when_the_directory_drops_it() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    let window = add_window(&mut cx, &store, temp.path(), "dropped-host");
    window.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    *fake.computers.lock().unwrap() = vec![computers()[0].clone()];
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.run_until_parked();
    let button = visual.debug_bounds("SIDEBAR_HOST_BUTTON").unwrap();
    visual.simulate_click(button.center(), Default::default());
    visual.run_until_parked();
    assert!(clients.read_with(&visual, |clients, _| clients
        .computers
        .iter()
        .all(|computer| computer.id != "host")));
    window
        .read_with(&visual, |root, _| assert!(root.remote_client.is_active()))
        .unwrap();
    let this_row = visual.debug_bounds("COMPUTER_PICKER_ROW").unwrap();
    let remote_row = visual.debug_bounds("COMPUTER_PICKER_REMOTE_ROW").unwrap();
    assert!(this_row.top() < remote_row.top());
    let check = visual.debug_bounds("COMPUTER_PICKER_CHECK").unwrap();
    assert!(remote_row.contains(&check.center()));
    assert!(visual.debug_bounds("COMPUTER_PICKER_CONNECTION").is_some());
    visual.simulate_click(this_row.center(), Default::default());
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| assert!(root.remote_client.is_local()))
        .unwrap();
}

#[test]
fn switching_to_another_computer_closes_the_usage_popover_for_good() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, _, _) = fixture(&mut cx, temp.path());
    let window = add_window(&mut cx, &store, temp.path(), "usage-closes-remote");
    window
        .update(&mut cx, |root, _, cx| {
            root.usage_installed = vec![runner_core::protocol::model::Runtime::Codex];
            cx.notify();
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    visual.run_until_parked();
    let usage = visual.debug_bounds("SIDEBAR_USAGE").unwrap();
    visual.simulate_click(usage.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("USAGE_POPOVER_PANEL").is_some());
    window
        .update(&mut visual, |root, window, cx| {
            choose_host(root, window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("USAGE_POPOVER_PANEL").is_none());
    assert!(window
        .read_with(&visual, |root, _| !root.usage_open)
        .unwrap());
    window
        .update(&mut visual, |root, window, cx| {
            root.return_to_this_computer(window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SIDEBAR_USAGE").is_some());
    assert!(visual.debug_bounds("USAGE_POPOVER_PANEL").is_none());
}

#[test]
fn popup_bars_and_colours_change_at_each_latency_boundary() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for (ms, bars) in [(99, 4), (100, 3), (199, 3), (200, 2), (499, 2), (500, 1)] {
        let slow = ms >= 200;
        let status =
            picker::connection_status(Connection::Connected, Some(ms), false, slow, "Studio");
        assert_eq!(status.mark, picker::LinkMark::Bars(bars));
        assert_eq!(
            status.mark_color,
            if slow {
                theme::warning()
            } else {
                theme::accent()
            }
        );
        assert_eq!(
            status.text_color,
            if slow {
                theme::warning()
            } else {
                theme::faint()
            }
        );
        assert_eq!(status.text, format!("{ms} ms · relay · encrypted"));
        assert_eq!(
            picker::host_tone(Connection::Connected, slow),
            status.mark_color
        );
    }
}

#[test]
fn popup_marks_cover_each_connection_state() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    use picker::LinkMark::{Icon, None as NoMark};
    for (connection, lacks_ping, mark, mark_color, text_color) in [
        (
            Connection::Opening,
            false,
            Icon("loader-circle.svg"),
            theme::muted(),
            theme::faint(),
        ),
        (
            Connection::Connected,
            false,
            Icon("loader-circle.svg"),
            theme::muted(),
            theme::faint(),
        ),
        (
            Connection::Connected,
            true,
            NoMark,
            theme::muted(),
            theme::faint(),
        ),
        (
            Connection::Reconnecting,
            false,
            Icon("refresh-cw.svg"),
            theme::warning(),
            theme::warning(),
        ),
        (
            Connection::Unreachable,
            false,
            Icon("unplug.svg"),
            theme::danger(),
            theme::danger(),
        ),
        (
            Connection::Revoked,
            false,
            NoMark,
            theme::danger(),
            theme::danger(),
        ),
        (
            Connection::Forgotten,
            false,
            NoMark,
            theme::muted(),
            theme::muted(),
        ),
    ] {
        let status = picker::connection_status(connection, None, lacks_ping, false, "Studio");
        assert_eq!(status.mark, mark);
        assert_eq!(status.mark_color, mark_color);
        assert_eq!(status.text_color, text_color);
        assert_eq!(
            status.text,
            picker::connection_subtitle(connection, None, false, "Studio").0
        );
    }
}

#[test]
fn slow_margin_is_shared_by_the_host_button_and_popup_and_resets_on_disconnect() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (_, fake, clients) = fixture(&mut cx, temp.path());
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"connected"}),
    );
    cx.run_until_parked();
    for (ms, slow, bars) in [
        (199, false, 3),
        (200, true, 2),
        (190, true, 3),
        (180, true, 3),
        (179, false, 3),
    ] {
        fake.emit(
            "remote/latency",
            serde_json::json!({"computer":"host","ms":ms}),
        );
        cx.run_until_parked();
        clients.read_with(&cx, |clients, _| {
            assert_eq!(clients.slow("host"), slow);
            let status = picker::connection_status(
                Connection::Connected,
                clients.latency("host"),
                false,
                clients.slow("host"),
                "Studio",
            );
            assert_eq!(status.mark, picker::LinkMark::Bars(bars));
            assert_eq!(
                status.mark_color,
                picker::host_tone(Connection::Connected, clients.slow("host"))
            );
        });
    }
    fake.emit(
        "remote/latency",
        serde_json::json!({"computer":"host","ms":200}),
    );
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"reconnecting"}),
    );
    cx.run_until_parked();
    assert!(!clients.read_with(&cx, |clients, _| clients.slow("host")));
}

#[test]
fn the_connected_row_shows_latency_once_a_sample_has_arrived() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for (locale, measured, unmeasured) in [
        (
            Locale::English,
            "42 ms · relay · encrypted",
            "Connected · relay · encrypted",
        ),
        (
            Locale::SimplifiedChinese,
            "42 ms · 中继 · 已加密",
            "已连接 · 中继 · 已加密",
        ),
    ] {
        with_locale(locale, || {
            let row = |latency| {
                picker::connection_subtitle(Connection::Connected, latency, false, "Studio")
            };
            assert_eq!(row(Some(42)), (measured.to_owned(), theme::faint()));
            assert_eq!(row(None), (unmeasured.to_owned(), theme::faint()));
        });
    }
}

#[test]
fn latency_is_kept_per_computer_until_its_connection_leaves_connected() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (_store, fake, clients) = fixture(&mut cx, temp.path());
    let latency = |cx: &TestAppContext, computer: &'static str| {
        clients.read_with(cx, |clients, _| clients.latency(computer))
    };
    let connection = |state: &str| serde_json::json!({"computer":"host","state":state});
    let sample = |ms: u64| serde_json::json!({"computer":"host","ms":ms});
    fake.emit(
        "remote/latency",
        serde_json::json!({"computer":"other","ms":10}),
    );
    cx.run_until_parked();
    assert_eq!(latency(&cx, "other"), None, "not connected yet");
    fake.emit("remote/connection", connection("connected"));
    fake.emit("remote/latency", sample(42));
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"other","state":"connected"}),
    );
    fake.emit(
        "remote/latency",
        serde_json::json!({"computer":"other","ms":7}),
    );
    cx.run_until_parked();
    assert_eq!(latency(&cx, "host"), Some(42));
    assert_eq!(latency(&cx, "other"), Some(7));
    fake.emit("remote/connection", connection("reconnecting"));
    cx.run_until_parked();
    assert_eq!(latency(&cx, "host"), None);
    assert_eq!(latency(&cx, "other"), Some(7));
    fake.emit("remote/latency", sample(11));
    fake.emit("remote/connection", connection("connected"));
    cx.run_until_parked();
    assert_eq!(latency(&cx, "host"), None, "a reconnect starts again");
    fake.emit("remote/latency", sample(50));
    cx.run_until_parked();
    assert_eq!(latency(&cx, "host"), Some(50));
    fake.emit("daemon/disconnected", serde_json::json!({}));
    cx.run_until_parked();
    assert_eq!(latency(&cx, "host"), None);
    assert_eq!(latency(&cx, "other"), None);
}

#[test]
fn a_host_is_only_known_to_lack_ping_after_its_features_are_discovered() {
    for (features, lacks) in [(vec!["overview", "ping"], false), (vec!["overview"], true)] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, clients) = fixture(&mut cx, temp.path());
        fake.set_features(&features);
        let window = add_window(&mut cx, &store, temp.path(), "ping-features");
        assert!(!clients.read_with(&cx, |clients, _| clients.lacks_ping("host")));
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        assert_eq!(
            clients.read_with(&cx, |clients, _| clients.lacks_ping("host")),
            lacks,
            "{features:?}"
        );
    }
}

#[test]
fn popup_latency_mark_leads_the_status_line_at_the_designed_size() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    fake.set_features(&["overview", "ping"]);
    let window = add_window(&mut cx, &store, temp.path(), "popup-mark");
    window.update(&mut cx, choose_host).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.run_until_parked();
    fake.emit(
        "remote/latency",
        serde_json::json!({"computer":"host","ms":42}),
    );
    window
        .update(&mut visual, |root, _, cx| {
            root.remote_client.ui.picker_open = true;
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    let bars = visual.debug_bounds("COMPUTER_PICKER_LATENCY_BARS").unwrap();
    let status = visual.debug_bounds("COMPUTER_PICKER_CONNECTION").unwrap();
    assert_eq!(bars.size, size(px(18.), px(9.)));
    assert_eq!(bars.left(), status.left());
    assert_eq!(
        visual
            .debug_bounds("COMPUTER_PICKER_STATUS_TEXT")
            .unwrap()
            .left()
            - bars.right(),
        px(5.)
    );
    assert!(visual.debug_bounds("SIDEBAR_LINK_STATUS").is_none());
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"reconnecting"}),
    );
    visual.run_until_parked();
    assert!(visual
        .debug_bounds("COMPUTER_PICKER_LATENCY_BARS")
        .is_none());
}

#[test]
fn signing_out_closes_a_popup_opened_while_signed_in() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    let window = add_window(&mut cx, &store, temp.path(), "sign-out-closes-popup");
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.run_until_parked();
    let button = visual.debug_bounds("SIDEBAR_HOST_BUTTON").unwrap();
    visual.simulate_click(button.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("COMPUTER_PICKER_REMOTE_ROW").is_some());
    *fake.status.lock().unwrap() = RemoteStatus::default();
    store.update(&mut visual, |store, cx| {
        store.remote_status = Some(RemoteStatus::default());
        cx.notify();
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
    visual.simulate_click(button.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("COMPUTER_PICKER_SIGN_IN").is_some());
    assert!(visual.debug_bounds("COMPUTER_PICKER_REMOTE_ROW").is_none());
}

#[test]
fn every_connection_state_has_a_host_tone_and_a_row_label() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for (connection, tone, color) in [
        (Connection::Connected, theme::accent(), theme::faint()),
        (Connection::Opening, theme::accent(), theme::faint()),
        (Connection::Reconnecting, theme::warning(), theme::warning()),
        (Connection::Unreachable, theme::danger(), theme::danger()),
        (Connection::Revoked, theme::danger(), theme::danger()),
        (Connection::Forgotten, theme::muted(), theme::muted()),
    ] {
        assert_eq!(picker::host_tone(connection, false), tone);
        assert_eq!(
            picker::connection_subtitle(connection, None, false, "Studio").1,
            color
        );
    }
    for (locale, expected) in [
        (
            Locale::English,
            [
                "Connected · relay · encrypted",
                "Connecting through the relay…",
                "Reconnecting to Studio…",
                "Can’t reach Studio",
                "Access not recognized · pair again",
                "Forgotten here · pair to open",
            ],
        ),
        (
            Locale::SimplifiedChinese,
            [
                "已连接 · 中继 · 已加密",
                "正在通过中继连接…",
                "正在重新连接 Studio…",
                "无法连接 Studio",
                "访问未被识别 · 请重新配对",
                "本机已忘记 · 配对后打开",
            ],
        ),
    ] {
        with_locale(locale, || {
            let labels = [
                Connection::Connected,
                Connection::Opening,
                Connection::Reconnecting,
                Connection::Unreachable,
                Connection::Revoked,
                Connection::Forgotten,
            ]
            .map(|connection| picker::connection_subtitle(connection, None, false, "Studio").0);
            assert_eq!(labels, expected);
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
                            let trigger = visual.debug_bounds("SIDEBAR_HOST_BUTTON").unwrap();
                            let footer = visual.debug_bounds("SIDEBAR_FOOTER_ROW").unwrap();
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
                            assert!(panel.bottom() < trigger.top());
                            assert_eq!(panel.left(), footer.left());
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
            visual.debug_bounds("HOST_BUTTON_ATTENTION").is_some(),
            !remote
        );
        let trigger = visual.debug_bounds("SIDEBAR_HOST_BUTTON").unwrap();
        assert!(visual.debug_bounds("HOST_BUTTON_BADGE").is_none());
        visual.simulate_click(trigger.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("COMPUTER_PICKER_REMOTE_ROW").is_some());
        assert_eq!(fake.overview_calls.load(Ordering::SeqCst), 1);
        assert!(visual.debug_bounds("COMPUTER_PICKER_ROW_BADGE").is_none());
        if remote {
            assert!(visual.debug_bounds("COMPUTER_ROW_ATTENTION").is_none());
            assert!(visual.debug_bounds("COMPUTER_PICKER_CHECK").is_some());
        } else {
            let dot = visual.debug_bounds("COMPUTER_ROW_ATTENTION").unwrap();
            assert_eq!(dot.size, size(px(7.), px(7.)));
            let host_dot = visual.debug_bounds("HOST_BUTTON_ATTENTION").unwrap();
            assert_eq!(host_dot.size, size(px(7.), px(7.)));
            assert!(trigger.contains(&host_dot.origin));
            assert!(host_dot.center().x > trigger.center().x);
            assert!(host_dot.center().y < trigger.center().y);
        }
        fake.emit("remote/session-status", serde_json::json!({"computer":"host","status":{"session":"s","status":"idle","unread":true}}));
        visual.run_until_parked();
        assert!(visual.debug_bounds("COMPUTER_ROW_ATTENTION").is_none());
        assert!(visual.debug_bounds("HOST_BUTTON_ATTENTION").is_none());
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
    }
}

#[test]
fn signed_out_host_popup_lists_only_this_computer_and_opens_account() {
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
                let window = add_window(&mut cx, &store, temp.path(), "signed-out-host-popup");
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                visual.run_until_parked();
                clients.update(&mut visual, |clients, cx| {
                    clients.computers = computers();
                    clients.overviews.insert("host".into(), overview());
                    clients.directory_error = true;
                    clients.action_failed = true;
                    cx.notify();
                });
                visual.run_until_parked();
                let footer = visual.debug_bounds("SIDEBAR_FOOTER_ROW").unwrap();
                let button = visual.debug_bounds("SIDEBAR_HOST_BUTTON").unwrap();
                let gear = visual.debug_bounds("SIDEBAR_SETTINGS").unwrap();
                assert!(button.top() >= footer.top() && button.right() <= gear.left());
                assert_eq!(button.size, gear.size);
                assert!(visual.debug_bounds("HOST_BUTTON_ATTENTION").is_none());
                assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
                visual.simulate_click(button.center(), Default::default());
                visual.run_until_parked();
                let panel = visual.debug_bounds("COMPUTER_PICKER_PANEL").unwrap();
                assert!(panel.bottom() < button.top());
                let this_row = visual.debug_bounds("COMPUTER_PICKER_ROW").unwrap();
                let divider = visual.debug_bounds("COMPUTER_PICKER_DIVIDER").unwrap();
                let sign_in = visual.debug_bounds("COMPUTER_PICKER_SIGN_IN").unwrap();
                assert!(this_row.bottom() <= divider.top());
                assert!(divider.bottom() <= sign_in.top());
                assert!(this_row.contains(
                    &visual
                        .debug_bounds("COMPUTER_PICKER_CHECK")
                        .unwrap()
                        .center()
                ));
                crate::theme_snapshot::assert_fill_color(
                    &mut visual,
                    "COMPUTER_PICKER_SELF_TILE",
                    Some(theme::with_alpha(theme::accent(), 0.14)),
                );
                for absent in [
                    "COMPUTER_PICKER_REMOTE_ROW",
                    "COMPUTER_ROW_ATTENTION",
                    "COMPUTER_PICKER_ERROR",
                    "COMPUTER_PICKER_ACTION_ERROR",
                    "COMPUTER_PICKER_SETTINGS",
                    "COMPUTER_PICKER_SETTINGS_DIVIDER",
                ] {
                    assert!(visual.debug_bounds(absent).is_none(), "{absent}");
                }
                let detail = visual.debug_bounds("COMPUTER_PICKER_DETAIL").unwrap();
                let name = visual.debug_bounds("COMPUTER_PICKER_NAME").unwrap();
                assert!(this_row.contains(&detail.center()) && name.bottom() <= detail.top());
                assert_eq!(fake.directory_calls.load(Ordering::SeqCst), calls);
                store.update(&mut visual, |store, cx| {
                    store.revisions.shell_settings += 1;
                    cx.notify();
                });
                visual.run_until_parked();
                assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_some());
                visual.simulate_keystrokes("tab");
                visual.run_until_parked();
                window
                    .update(&mut visual, |root, window, cx| {
                        assert!(root.remote_client.ui.picker_open);
                        assert!(root
                            .remote_client
                            .ui
                            .picker_focus
                            .contains_focused(window, cx));
                    })
                    .unwrap();
                assert_eq!(fake.directory_calls.load(Ordering::SeqCst), calls);
                visual.simulate_click(sign_in.center(), Default::default());
                visual.run_until_parked();
                assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
                assert!(visual.debug_bounds("ACCOUNT_STATUS_CARD").is_some());
                window
                    .read_with(&visual, |root, _| {
                        assert_eq!(root.route, AppRoute::Settings);
                        assert_eq!(
                            root.settings_page.pane,
                            crate::surfaces::settings::page::SettingsPane::Account
                        );
                    })
                    .unwrap();
                assert_eq!(fake.directory_calls.load(Ordering::SeqCst), calls);
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
fn picker_dismisses_at_tab_boundaries_and_escape_and_returns_focus_to_the_host_button() {
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
        let host_focused = |visual: &mut VisualTestContext| {
            window
                .update(visual, |root, window, _| {
                    root.remote_client.ui.host_focus.is_focused(window)
                })
                .unwrap()
        };
        let trigger = visual.debug_bounds("SIDEBAR_HOST_BUTTON").unwrap();
        visual.simulate_click(trigger.center(), Default::default());
        visual.run_until_parked();
        visual.simulate_keystrokes("tab escape");
        visual.run_until_parked();
        assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
        assert!(host_focused(&mut visual));
        for key in ["enter", "space", "down"] {
            visual.simulate_keystrokes(key);
            visual.run_until_parked();
            assert!(
                visual.debug_bounds("COMPUTER_PICKER_PANEL").is_some(),
                "{key} did not open the popup"
            );
            visual.simulate_keystrokes("escape");
            visual.run_until_parked();
            assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
            assert!(host_focused(&mut visual));
        }
        for direction in ["tab", "shift-tab"] {
            visual.simulate_click(trigger.center(), Default::default());
            visual.run_until_parked();
            let mut dismissed = false;
            for _ in 0..16 {
                visual.simulate_keystrokes(direction);
                visual.run_until_parked();
                dismissed = window
                    .update(&mut visual, |root, window, cx| {
                        if root.remote_client.ui.picker_open {
                            assert!(
                                root.remote_client
                                    .ui
                                    .picker_focus
                                    .contains_focused(window, cx),
                                "focus left an open computer picker"
                            );
                            false
                        } else {
                            assert!(root.remote_client.ui.host_focus.is_focused(window));
                            true
                        }
                    })
                    .unwrap();
                if dismissed {
                    break;
                }
            }
            assert!(dismissed, "Tab traversal did not reach the picker boundary");
            visual.simulate_keystrokes("escape");
            visual.run_until_parked();
            assert!(visual.debug_bounds("COMPUTER_PICKER_PANEL").is_none());
            assert!(host_focused(&mut visual));
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
        .read_with(&cx, |root, _| assert!(root.remote_client.is_local()))
        .unwrap();
    let mut remote_count = 0;
    cx.update(|cx| {
        for window in cx.windows() {
            if let Some(window) = window.downcast::<NativeRoot>() {
                if window.read(cx).unwrap().remote_client.is_active() {
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
                    assert_eq!(visual.debug_bounds("SESSION_ENDED_CARD").is_some(), !shell);
                    assert!(visual.debug_bounds("REMOTE_TERMINAL_FAILED").is_none());
                    assert!(fake.attaches.lock().unwrap().is_empty());
                    if shell {
                        continue;
                    }
                    let size = window
                        .read_with(&visual, |root, _| {
                            root.remote_client.remote_terminal_size("s")
                        })
                        .unwrap()
                        .unwrap();
                    assert!(size.0 >= 20 && size.1 >= 8);
                    window
                        .read_with(&visual, |root, cx| {
                            assert!(root.front_page_client(cx).supports(&["resume"]));
                            assert!(matches!(
                                root.remote_client.workspace().unwrap().connection,
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
                        .read_with(&visual, |root, _| {
                            root.remote_client.remote_terminal_size("s")
                        })
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
                            root.remote_client.workspace().unwrap().terminals["s"]
                                .attachment
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
                                &root.remote_client.workspace().unwrap().terminals["s"]
                                    .attachment
                                    .terminal
                            ));
                            assert!(!root.remote_client.remote_terminal_interactive("s", cx));
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
                assert!(root.remote_client.workspace().unwrap().terminals.is_empty())
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
                root.remote_client.workspace().unwrap().terminals["s"]
                    .attachment
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
            root.remote_client.workspace().unwrap().terminals["s"]
                .attachment
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
            root.remote_client.workspace().unwrap().terminals["s"]
                .attachment
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
                .remote_client
                .workspace()
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
                root.remote_client.workspace().unwrap().computer.fingerprint,
                "identity-B"
            );
            root.remote_client.open_computer_pair(
                computers()[1].clone(),
                pairing::Purpose::Open { new_window: false },
                window,
                cx,
            );
            let dialog = root.remote_client.ui.pair.as_ref().unwrap();
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
            let dialog = root.remote_client.ui.pair.as_ref().unwrap();
            assert_eq!(dialog.computer.fingerprint, "identity-C");
            assert!(dialog.field.read(cx).value().0.is_empty());
            assert!(dialog.state == pairing::PairState::Entry);
            let field = dialog.field.clone();
            field.update(cx, |field, cx| {
                field.replace_text_in_range(None, "001234", window, cx)
            });
            root.remote_client.submit_computer_pin(window, cx);
        })
        .unwrap();
    fake.computers.lock().unwrap()[1].fingerprint = "identity-D".into();
    cx.run_until_parked();
    assert_eq!(fake.pairs.load(Ordering::SeqCst), 0);
    window
        .read_with(&cx, |root, cx| {
            let dialog = root.remote_client.ui.pair.as_ref().unwrap();
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
    window
        .update(&mut cx, |root, _, cx| {
            root.remote_client
                .workspace()
                .unwrap()
                .sidebar
                .update(cx, |sidebar, cx| sidebar.toggle_active_only(cx));
        })
        .unwrap();
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
            assert!(root.remote_client.is_local());
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
    window
        .read_with(&cx, |root, cx| {
            assert!(root
                .remote_client
                .workspace()
                .unwrap()
                .sidebar
                .read(cx)
                .active_only(cx));
        })
        .unwrap();
    cx.update(|cx| crate::notifications::open("mission-agent", cx));
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| {
            assert!(root.remote_client.is_local());
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
    for surface in ["chat", "mission", "mission-drawer"] {
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
                        root.remote_client
                            .workspace()
                            .unwrap()
                            .mission_workspace
                            .update(cx, |workspace, cx| {
                                workspace.focus_status_session("remote-mission", "s", window, cx)
                            });
                        root.remote_client.attach_remote_panes(cx);
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
                        .remote_client
                        .workspace()
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
                        root.remote_client
                            .workspace()
                            .unwrap()
                            .action_error
                            .as_deref(),
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
                assert!(root
                    .remote_client
                    .workspace()
                    .unwrap()
                    .action_error
                    .is_none())
            })
            .unwrap();
    }
    let old_input = window
        .read_with(&cx, |root, _| {
            root.remote_client.workspace().unwrap().terminals["s"]
                .attachment
                .terminal_input
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
                root.remote_client
                    .workspace()
                    .unwrap()
                    .action_error
                    .as_deref(),
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
            assert!(root.remote_client.is_local());
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
            assert!(root
                .remote_client
                .workspace()
                .unwrap()
                .action_error
                .is_none())
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
            root.remote_client.open_computer_pair(
                computer,
                pairing::Purpose::Open { new_window: false },
                window,
                cx,
            );
            let field = root.remote_client.ui.pair.as_ref().unwrap().field.clone();
            field.update(cx, |field, cx| {
                field.replace_text_in_range(None, "001234", window, cx)
            });
            root.remote_client.submit_computer_pin(window, cx);
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
            let remote = root.remote_client.workspace().unwrap();
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
            assert!(root.remote_client.workspace().unwrap().connection == Connection::Connected)
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
                assert!(root
                    .remote_client
                    .computer_menu(row.computer, &row.devices, cx)
                    .is_some());
            }
            let menus = root.remote_client.ui.menus.borrow();
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
                    let remote = root.remote_client.workspace().unwrap();
                    assert!(remote.connection == Connection::Forgotten);
                    assert!(remote.terminals.is_empty());
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            assert!(visual.debug_bounds("REMOTE_CONNECTION_CARD").is_some());
            window
                .update(&mut cx, |root, window, cx| {
                    root.remote_client.open_computer_pair(
                        computers()[1].clone(),
                        pairing::Purpose::Open { new_window: false },
                        window,
                        cx,
                    );
                    assert!(!root.remote_client.ui.pair.as_ref().unwrap().again);
                })
                .unwrap();
        });
    }
}

fn two_tab_overview() -> Overview {
    let mut data = overview();
    let mut tab = data.tabs[0].clone();
    tab.id = "tab-b".into();
    tab.name = "Tab B".into();
    tab.panes = runner_core::companion::Pane::Leaf {
        session: Some("b".into()),
    };
    data.tabs.push(tab);
    let mut session = data.sessions["s"].clone();
    session.title = "Session B".into();
    data.sessions.insert("b".into(), session);
    data
}

#[test]
fn switching_remote_tabs_keeps_hidden_chats_attached_until_they_close_stop_or_disconnect() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    fake.set_overview(two_tab_overview());
    cx.run_until_parked();
    let window = add_window(&mut cx, &store, temp.path(), "remote-tab-switch");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let select = |cx: &mut TestAppContext, item: Item| {
        window
            .update(cx, |root, _, cx| root.select_remote_item(item, cx))
            .unwrap();
        cx.run_until_parked();
    };
    let mirror = |cx: &mut TestAppContext, id: &str| {
        window
            .read_with(cx, |root, _| {
                root.remote_client
                    .workspace()
                    .unwrap()
                    .terminals
                    .get(id)
                    .map(|pane| pane.attachment.terminal.clone())
            })
            .unwrap()
    };
    let attached = |fake: &Fake| {
        fake.attaches
            .lock()
            .unwrap()
            .iter()
            .map(|config| (config.session.clone(), config.take_control))
            .collect::<Vec<_>>()
    };
    // The host answers each size push with a Resized frame; until it does, a
    // drawn pane keeps pushing. Let the window's sidebar transition finish so
    // the measured size is final, then acknowledge it like the host would.
    let mut seq = 100;
    let mut settle = |cx: &mut TestAppContext| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        window.update(cx, |_, window, _| window.refresh()).unwrap();
        cx.run_until_parked();
        let attaches = fake.attaches.lock().unwrap().clone();
        let sizes = fake.sizes.lock().unwrap().clone();
        for (index, config) in attaches.iter().enumerate() {
            let address = config.address();
            let Some(&(_, cols, rows)) = sizes.iter().rev().find(|size| size.0 == address) else {
                continue;
            };
            seq += 1;
            fake.frame_senders.lock().unwrap()[index]
                .send(Ok(TerminalFrame::Resized {
                    seq,
                    cols,
                    rows,
                    controller: TerminalController::Desktop,
                }))
                .unwrap();
            let terminal = mirror(cx, &config.session).unwrap();
            for _ in 0..400 {
                if terminal.size() == (cols, rows) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert_eq!(terminal.size(), (cols, rows));
        }
        cx.run_until_parked();
    };
    let a = mirror(&mut cx, "s").expect("the first tab attaches");
    settle(&mut cx);
    select(&mut cx, Item::Tab("tab-b".into()));
    let b = mirror(&mut cx, "b").expect("switching attaches the second tab");
    settle(&mut cx);
    assert_eq!(
        attached(&fake),
        vec![("s".to_owned(), true), ("b".to_owned(), true)]
    );
    let sizes = fake.sizes.lock().unwrap().len();

    for item in [
        Item::Tab("remote-tab".into()),
        Item::Tab("tab-b".into()),
        Item::Mission("remote-mission".into()),
        Item::Tab("remote-tab".into()),
    ] {
        select(&mut cx, item);
    }
    assert_eq!(attached(&fake).len(), 2, "switching back never re-attaches");
    assert!(fake.takes.lock().unwrap().is_empty());
    assert_eq!(
        fake.sizes.lock().unwrap().len(),
        sizes,
        "a hidden or re-shown pane sends no size"
    );
    assert_eq!(fake.cancels.load(Ordering::SeqCst), 0);
    assert!(Arc::ptr_eq(&a, &mirror(&mut cx, "s").unwrap()));
    assert!(Arc::ptr_eq(&b, &mirror(&mut cx, "b").unwrap()));

    fake.set_overview(overview());
    cx.run_until_parked();
    assert!(
        mirror(&mut cx, "b").is_none(),
        "closing a tab detaches its chat"
    );
    assert_eq!(fake.cancels.load(Ordering::SeqCst), 1);

    fake.set_overview(two_tab_overview());
    cx.run_until_parked();
    assert!(
        mirror(&mut cx, "b").is_none(),
        "an unvisited tab stays detached"
    );
    select(&mut cx, Item::Tab("tab-b".into()));
    assert!(mirror(&mut cx, "b").is_some());
    let mut stopped = two_tab_overview();
    stopped.sessions.get_mut("s").unwrap().status.status = Status::Stopped;
    fake.set_overview(stopped.clone());
    cx.run_until_parked();
    assert!(
        mirror(&mut cx, "s").is_none(),
        "a stopped hidden chat detaches"
    );
    assert_eq!(fake.cancels.load(Ordering::SeqCst), 2);

    select(&mut cx, Item::Mission("remote-mission".into()));
    assert!(mirror(&mut cx, "b").is_some());
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"disconnected","error":"route closed"}),
    );
    cx.run_until_parked();
    assert!(
        mirror(&mut cx, "b").is_none(),
        "a dropped connection detaches hidden chats"
    );
    assert_eq!(fake.cancels.load(Ordering::SeqCst), 3);
    assert_eq!(attached(&fake).len(), 3);
}

#[test]
fn hidden_remote_chats_recover_without_taking_control() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    fake.set_overview(two_tab_overview());
    cx.run_until_parked();
    let window = add_window(&mut cx, &store, temp.path(), "remote-tab-recovery");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let select = |cx: &mut TestAppContext, item: Item| {
        window
            .update(cx, |root, _, cx| root.select_remote_item(item, cx))
            .unwrap();
        cx.run_until_parked();
    };
    let attached = |fake: &Fake| {
        fake.attaches
            .lock()
            .unwrap()
            .iter()
            .map(|config| (config.session.clone(), config.take_control))
            .collect::<Vec<_>>()
    };
    let has_mirror = |cx: &mut TestAppContext, id: &str| {
        window
            .read_with(cx, |root, _| {
                root.remote_client
                    .workspace()
                    .unwrap()
                    .terminals
                    .contains_key(id)
            })
            .unwrap()
    };
    select(&mut cx, Item::Tab("tab-b".into()));
    assert_eq!(
        attached(&fake),
        vec![("s".to_owned(), true), ("b".to_owned(), true)]
    );

    fake.emit("daemon/disconnected", serde_json::Value::Null);
    fake.emit("daemon/reconnected", serde_json::Value::Null);
    cx.run_until_parked();
    assert_eq!(attached(&fake)[2..], [("b".to_owned(), false)]);
    assert!(!has_mirror(&mut cx, "s"));
    select(&mut cx, Item::Mission("remote-mission".into()));
    select(&mut cx, Item::Tab("remote-tab".into()));
    assert_eq!(
        attached(&fake)[3..],
        [("s".to_owned(), false)],
        "a chat hidden during daemon recovery reattaches without taking control"
    );
    select(&mut cx, Item::Tab("tab-b".into()));
    assert_eq!(attached(&fake).len(), 4);

    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"disconnected","error":"route closed"}),
    );
    cx.run_until_parked();
    assert!(!has_mirror(&mut cx, "s"));
    assert!(has_mirror(&mut cx, "b"));
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"connected","error":""}),
    );
    cx.run_until_parked();
    select(&mut cx, Item::Mission("remote-mission".into()));
    select(&mut cx, Item::Tab("remote-tab".into()));
    assert_eq!(
        attached(&fake)[4..],
        [("s".to_owned(), false)],
        "a chat hidden during a route reconnect reattaches without taking control"
    );

    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"disconnected","error":"route closed"}),
    );
    cx.run_until_parked();
    assert!(!has_mirror(&mut cx, "b"));
    let mut stopped = two_tab_overview();
    stopped.sessions.get_mut("b").unwrap().status.status = Status::Stopped;
    fake.set_overview(stopped);
    cx.run_until_parked();
    fake.set_overview(two_tab_overview());
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"connected","error":""}),
    );
    cx.run_until_parked();
    select(&mut cx, Item::Tab("tab-b".into()));
    assert_eq!(
        attached(&fake)[5..],
        [("b".to_owned(), true)],
        "a chat that stopped and restarted opens fresh"
    );
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
                    root.remote_client.workspace().unwrap().terminals["s"]
                        .attachment
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
                        &root.remote_client.workspace().unwrap().terminals["s"]
                            .attachment
                            .terminal
                    ))
                })
                .unwrap();
            assert_eq!(fake.attaches.lock().unwrap().len(), attaches);
            assert_eq!(fake.follows.lock().unwrap().len(), follows);
            fake.fail_take.store(true, Ordering::SeqCst);
            window
                .update(&mut cx, |root, _, cx| {
                    root.remote_client.take_remote_control("s", cx)
                })
                .unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            assert!(visual.debug_bounds("REMOTE_CONTROL_FAILED").is_some());
            fake.fail_take.store(false, Ordering::SeqCst);
            window
                .update(&mut cx, |root, _, cx| {
                    root.remote_client.take_remote_control("s", cx)
                })
                .unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            assert!(visual.debug_bounds("REMOTE_CONTROL_FAILED").is_none());
            assert_eq!(fake.takes.lock().unwrap().len(), 2);
            fake.fail_take.store(true, Ordering::SeqCst);
            window
                .update(&mut cx, |root, _, cx| {
                    root.remote_client.take_remote_control("s", cx)
                })
                .unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            assert!(visual.debug_bounds("REMOTE_CONTROL_FAILED").is_some());
            let attaches = fake.attaches.lock().unwrap().len();
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
            assert!(
                visual.debug_bounds("REMOTE_CONTROL_FAILED").is_some(),
                "a chat hidden by a mission keeps its pane"
            );
            assert!(
                visual.debug_bounds("REMOTE_TAKEN_BACK").is_some(),
                "switching back does not take control from the host desktop"
            );
            assert_eq!(fake.attaches.lock().unwrap().len(), attaches);
            window
                .read_with(&cx, |root, _| {
                    assert!(Arc::ptr_eq(
                        &original,
                        &root.remote_client.workspace().unwrap().terminals["s"]
                            .attachment
                            .terminal
                    ))
                })
                .unwrap();
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
                    let remote = root.remote_client.workspace().unwrap();
                    assert!(remote.connection == Connection::Connected);
                    assert!(!Arc::ptr_eq(
                        &original,
                        &remote.terminals["s"].attachment.terminal
                    ));
                })
                .unwrap();
        });
    }
}

#[test]
fn switching_computers_restores_each_hosts_saved_sidebar_view() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    fake.set_overview(sidebar_overview());
    let other = Computer {
        id: "other".into(),
        name: "Mini".into(),
        ..computers()[1].clone()
    };
    fake.computers.lock().unwrap().push(other.clone());
    clients.update(&mut cx, |clients, cx| clients.refresh(cx));
    store.update(&mut cx, |store, _| {
        let settings = &mut store.settings;
        settings.sidebar_active_only = true;
        settings
            .sidebar_collapsed_projects
            .insert("local-project".into());
        settings
            .sidebar_collapsed_active_groups
            .insert("agents".into());
        settings
            .sidebar_collapsed_link_groups
            .insert("local-tab".into());
        settings.sidebar_projects_open = false;
        settings.sidebar_chats_open = false;
    });
    cx.run_until_parked();
    let view = |settings: &AppSettings| {
        (
            settings.sidebar_active_only,
            settings.sidebar_collapsed_projects.clone(),
            settings.sidebar_collapsed_active_groups.clone(),
            settings.sidebar_projects_open,
            settings.sidebar_chats_open,
            settings.sidebar_collapsed_link_groups.clone(),
        )
    };
    let local = store.read_with(&cx, |store, _| view(&store.settings));
    let defaults = (
        false,
        Default::default(),
        Default::default(),
        true,
        true,
        Default::default(),
    );
    let window = add_window(&mut cx, &store, temp.path(), "remember-remote-sidebar");
    let visit = |cx: &mut TestAppContext, computer: Computer| {
        window
            .update(cx, |root, window, cx| {
                root.choose_computer(computer, false, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
    };
    let sidebar = |cx: &TestAppContext| {
        window
            .read_with(cx, |root, _| {
                root.remote_client.workspace().unwrap().sidebar.clone()
            })
            .unwrap()
    };
    let remote_view = |cx: &TestAppContext| {
        sidebar(cx).read_with(cx, |sidebar, _| {
            view(sidebar.remote_settings.as_ref().unwrap())
        })
    };
    visit(&mut cx, computers()[1].clone());
    assert_eq!(remote_view(&cx), defaults);
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let project = visual.debug_bounds("SIDEBAR_PROJECT:p").unwrap();
    visual.simulate_click(project.center(), Default::default());
    visual.run_until_parked();
    for selector in ["SIDEBAR_SECTION:PROJECTS", "SIDEBAR_SECTION:RECENTS"] {
        let section = visual.debug_bounds(selector).unwrap();
        visual.simulate_click(
            point(section.left() + px(24.), section.center().y),
            Default::default(),
        );
        visual.run_until_parked();
    }
    sidebar(&cx).update(&mut cx, |sidebar, cx| {
        sidebar
            .remote_settings
            .as_mut()
            .unwrap()
            .sidebar_collapsed_link_groups
            .insert("remote-tab".into());
        sidebar.toggle_active_only(cx);
    });
    visual.run_until_parked();
    let group = visual.debug_bounds("ACTIVE_GROUP_TOGGLE agents").unwrap();
    visual.simulate_click(group.center(), Default::default());
    visual.run_until_parked();
    let remembered = remote_view(&cx);
    assert!(remembered.0);
    assert!(!remembered.1.is_empty());
    assert!(remembered.2.contains("agents"));
    assert!(!remembered.3 && !remembered.4);
    assert!(remembered.5.contains("remote-tab"));

    visit(&mut cx, computers()[0].clone());
    assert_eq!(
        store.read_with(&cx, |store, _| view(&store.settings)),
        local
    );
    let overrides = store.update(&mut cx, |store, _| {
        let mut shortcut = crate::keymap::entry("new-chat").unwrap().default.clone();
        shortcut.shift = !shortcut.shift;
        store
            .settings
            .keymap_overrides
            .insert("new-chat".into(), Some(shortcut));
        store.settings.keymap_overrides.clone()
    });
    visit(&mut cx, computers()[1].clone());
    assert_eq!(remote_view(&cx), remembered);
    sidebar(&cx).read_with(&cx, |sidebar, _| {
        assert_eq!(
            sidebar.remote_settings.as_ref().unwrap().keymap_overrides,
            overrides
        );
    });
    visit(&mut cx, other.clone());
    assert_eq!(remote_view(&cx), defaults);
    clients.update(&mut cx, |clients, cx| {
        clients
            .overviews
            .insert(other.id.clone(), sidebar_overview());
        cx.notify();
    });
    cx.run_until_parked();
    sidebar(&cx).update(&mut cx, |sidebar, cx| sidebar.toggle_active_only(cx));
    let other_view = remote_view(&cx);
    let saved = store.read_with(&cx, |store, _| store.settings.remote_sidebars.clone());
    assert!(saved["other"].active_only);
    assert_eq!(saved["host"].collapsed_projects, remembered.1);
    assert_eq!(saved["other"].collapsed_projects, other_view.1);
    assert_ne!(saved["host"], saved["other"]);
    visit(&mut cx, computers()[1].clone());
    assert_eq!(remote_view(&cx), remembered);
    visit(&mut cx, other);
    assert_eq!(remote_view(&cx), other_view);
    visit(&mut cx, computers()[1].clone());
    sidebar(&cx).update(&mut cx, |sidebar, cx| sidebar.toggle_active_only(cx));
    visit(&mut cx, computers()[0].clone());
    visit(&mut cx, computers()[1].clone());
    let mut show_all = remembered.clone();
    show_all.0 = false;
    assert_eq!(remote_view(&cx), show_all);
    assert_eq!(
        store.read_with(&cx, |store, _| view(&store.settings)),
        local
    );

    let second = add_window(&mut cx, &store, temp.path(), "independent-remote-sidebar");
    second.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    second
        .read_with(&cx, |root, cx| {
            let sidebar = root.remote_client.workspace().unwrap().sidebar.read(cx);
            assert_eq!(view(sidebar.remote_settings.as_ref().unwrap()), show_all);
        })
        .unwrap();
    second
        .update(&mut cx, |root, _, cx| {
            root.remote_client
                .workspace()
                .unwrap()
                .sidebar
                .update(cx, |sidebar, cx| sidebar.toggle_active_only(cx));
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        remote_view(&cx),
        show_all,
        "open windows keep their working copy"
    );

    visit(&mut cx, computers()[0].clone());
    clients.update(&mut cx, |clients, _| {
        clients.overviews.remove("host");
    });
    visit(&mut cx, computers()[1].clone());
    assert_eq!(
        remote_view(&cx),
        remembered,
        "dropping an overview does not discard the saved view"
    );
}

#[test]
fn first_remote_overview_seeds_only_empty_projects_once() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for explicit_sidebar in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, clients) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        let mut data = sidebar_overview();
        for id in ["empty", "pinned-only", "mission-only"] {
            let mut project = data.projects[0].clone();
            project.id = id.into();
            project.name = id.into();
            data.projects.push(project);
        }
        let mut pinned = data.tabs[0].clone();
        pinned.id = "only-pinned-tab".into();
        pinned.project = Some("pinned-only".into());
        data.tabs.push(pinned);
        let mut mission = data.missions[0].clone();
        mission.id = "only-mission".into();
        mission.project = Some("mission-only".into());
        data.missions.push(mission.clone());
        mission.id = "only-pinned-mission".into();
        mission.project = Some("pinned-only".into());
        mission.pinned = true;
        data.missions.push(mission);
        if explicit_sidebar {
            data.sidebar = Some(workspace::sidebar_nodes(&data));
        }
        fake.set_overview(data.clone());
        cx.run_until_parked();
        assert!(store
            .read_with(&cx, |store, _| store.settings.clone())
            .remote_sidebars
            .is_empty());
        clients.update(&mut cx, |clients, _| {
            clients.overviews.remove("host");
        });
        let window = add_window(&mut cx, &store, temp.path(), "seed-remote-sidebar");
        window
            .update(&mut cx, |root, window, cx| {
                root.visit_computer(computers()[1].clone(), window, cx);
            })
            .unwrap();
        assert!(
            store
                .read_with(&cx, |store, _| store.settings.clone())
                .remote_sidebars
                .is_empty(),
            "seeding waits for the overview"
        );
        fake.set_overview(data.clone());
        cx.run_until_parked();
        let saved = store
            .read_with(&cx, |store, _| store.settings.clone())
            .remote_sidebars["host"]
            .clone();
        assert_eq!(
            saved.collapsed_projects,
            ["empty".into(), "pinned-only".into()].into()
        );
        assert!(saved.projects_open && saved.chats_open);
        assert!(!saved.active_only);
        assert!(saved.collapsed_active_groups.is_empty() && saved.collapsed_link_groups.is_empty());

        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        let project = visual.debug_bounds("SIDEBAR_PROJECT:empty").unwrap();
        visual.simulate_click(project.center(), Default::default());
        visual.run_until_parked();
        let saved = store
            .read_with(&cx, |store, _| store.settings.clone())
            .remote_sidebars["host"]
            .clone();
        assert_eq!(saved.collapsed_projects, ["pinned-only".into()].into());
        let mut tab = data.tabs[1].clone();
        tab.id = "new-chat".into();
        tab.project = Some("pinned-only".into());
        data.tabs.push(tab);
        data.sidebar = None;
        fake.set_overview(data);
        visual.run_until_parked();
        assert_eq!(
            store
                .read_with(&cx, |store, _| store.settings.clone())
                .remote_sidebars["host"],
            saved
        );
        window
            .update(&mut cx, |root, window, cx| {
                root.choose_computer(computers()[0].clone(), false, window, cx);
                root.choose_computer(computers()[1].clone(), false, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .read_with(&cx, |root, cx| {
                let sidebar = root.remote_client.workspace().unwrap().sidebar.read(cx);
                assert_eq!(
                    crate::app_settings::RemoteSidebarSettings::from_settings(
                        sidebar.remote_settings.as_ref().unwrap()
                    ),
                    saved
                );
            })
            .unwrap();
    }
}

#[test]
fn remote_sidebar_waits_for_the_seed_before_persisting_initial_window_views() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for window_count in [1, 2] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, clients) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        clients.update(&mut cx, |clients, _| {
            clients.overviews.remove("host");
        });
        let windows = (0..window_count)
            .map(|index| {
                let window = add_window(
                    &mut cx,
                    &store,
                    temp.path(),
                    &format!("pending-remote-sidebar-{index}"),
                );
                window
                    .update(&mut cx, |root, window, cx| {
                        root.visit_computer(computers()[1].clone(), window, cx);
                        if window_count == 1 {
                            root.remote_client.workspace().unwrap().sidebar.update(
                                cx,
                                |sidebar, cx| {
                                    let settings = sidebar.remote_settings.as_mut().unwrap();
                                    settings.sidebar_projects_open = false;
                                    settings.sidebar_chats_open = false;
                                    sidebar.toggle_active_only(cx);
                                },
                            );
                        }
                    })
                    .unwrap();
                window
            })
            .collect::<Vec<_>>();
        assert!(store.read_with(&cx, |store, _| store.settings.remote_sidebars.is_empty()));
        let targets = windows
            .iter()
            .map(|window| {
                window
                    .read_with(&cx, |root, _| {
                        root.remote_client
                            .workspace()
                            .unwrap()
                            .front_page
                            .target
                            .clone()
                    })
                    .unwrap()
            })
            .collect::<Vec<_>>();
        clients.update(&mut cx, |clients, _| {
            clients.invalidate_capabilities("host")
        });
        for (window, target) in windows.iter().zip(targets) {
            window
                .update(&mut cx, |root, _, cx| root.sync_remote_state(cx))
                .unwrap();
            window
                .read_with(&cx, |root, cx| {
                    let remote = root.remote_client.workspace().unwrap();
                    assert_ne!(remote.front_page.target, target);
                    let settings = remote.sidebar.read(cx).remote_settings.as_ref().unwrap();
                    assert_eq!(settings.sidebar_active_only, window_count == 1);
                    assert_eq!(settings.sidebar_projects_open, window_count != 1);
                    assert_eq!(settings.sidebar_chats_open, window_count != 1);
                })
                .unwrap();
        }
        assert!(store.read_with(&cx, |store, _| store.settings.remote_sidebars.is_empty()));
        fake.set_overview(overview());
        cx.run_until_parked();
        let saved = store.read_with(&cx, |store, _| {
            store.settings.remote_sidebars["host"].clone()
        });
        assert_eq!(saved.collapsed_projects, ["p".into()].into());
        assert_eq!(saved.active_only, window_count == 1);
        assert_eq!(saved.projects_open, window_count != 1);
        assert_eq!(saved.chats_open, window_count != 1);
        for window in &windows {
            window
                .read_with(&cx, |root, cx| {
                    let sidebar = root.remote_client.workspace().unwrap().sidebar.read(cx);
                    assert_eq!(
                        crate::app_settings::RemoteSidebarSettings::from_settings(
                            sidebar.remote_settings.as_ref().unwrap()
                        ),
                        saved
                    );
                })
                .unwrap();
        }
        windows
            .last()
            .unwrap()
            .update(&mut cx, |root, _, cx| {
                root.remote_client
                    .workspace()
                    .unwrap()
                    .sidebar
                    .update(cx, |sidebar, cx| sidebar.toggle_active_only(cx));
            })
            .unwrap();
        let changed = store.read_with(&cx, |store, _| {
            store.settings.remote_sidebars["host"].clone()
        });
        assert_eq!(changed.collapsed_projects, saved.collapsed_projects);
        assert_eq!(changed.active_only, !saved.active_only);
    }
}

#[test]
fn remote_sidebar_view_survives_a_fresh_store_loading_the_settings_file() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let window = add_window(&mut cx, &store, temp.path(), "save-remote-sidebar");
    window.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    assert_eq!(
        store
            .read_with(&cx, |store, _| store.settings.clone())
            .remote_sidebars["host"]
            .collapsed_projects,
        ["p".into()].into()
    );
    window
        .update(&mut cx, |root, _, cx| {
            root.remote_client
                .workspace()
                .unwrap()
                .sidebar
                .update(cx, |sidebar, cx| {
                    let settings = sidebar.remote_settings.as_mut().unwrap();
                    settings.sidebar_collapsed_projects.clear();
                    settings
                        .sidebar_collapsed_active_groups
                        .insert("agents".into());
                    settings
                        .sidebar_collapsed_link_groups
                        .insert("remote-tab".into());
                    settings.sidebar_projects_open = false;
                    settings.sidebar_chats_open = false;
                    sidebar.toggle_active_only(cx);
                });
        })
        .unwrap();
    cx.run_until_parked();
    let saved = store
        .read_with(&cx, |store, _| store.settings.clone())
        .remote_sidebars["host"]
        .clone();
    let path = temp.path().join("settings.json");
    let settings = AppSettings::load(&path).unwrap();
    assert_eq!(settings.remote_sidebars["host"], saved);
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    let restarted = cx.new(|cx| AppStore::new(core, None, None, path, settings, None, cx));
    cx.run_until_parked();
    restarted.update(&mut cx, |store, _| {
        store.client = fake.client();
        store.remote_clients = clients;
        store.remote_status = Some(fake.status.lock().unwrap().clone());
    });
    let second = add_window(&mut cx, &restarted, temp.path(), "reload-remote-sidebar");
    second.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    second
        .read_with(&cx, |root, cx| {
            let sidebar = root.remote_client.workspace().unwrap().sidebar.read(cx);
            assert_eq!(
                crate::app_settings::RemoteSidebarSettings::from_settings(
                    sidebar.remote_settings.as_ref().unwrap()
                ),
                saved
            );
        })
        .unwrap();
    assert_eq!(
        restarted
            .read_with(&cx, |store, _| store.settings.clone())
            .remote_sidebars["host"],
        saved
    );
}

#[test]
fn switching_back_to_a_computer_restores_its_selection_and_split_focus() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let other = Computer {
        id: "other".into(),
        name: "Mini".into(),
        online: true,
        paired: true,
        fingerprint: "other-key".into(),
        ..Default::default()
    };
    fake.computers.lock().unwrap().push(other.clone());
    clients.update(&mut cx, |clients, cx| clients.refresh(cx));
    let mut data = sidebar_overview();
    data.sessions
        .insert("s2".into(), data.sessions["s"].clone());
    data.tabs[1].panes = serde_json::from_value(serde_json::json!({
        "split":"row", "sizes":[0.5, 0.5], "a":{"session":"s"}, "b":{"session":"s2"}
    }))
    .unwrap();
    fake.set_overview(data.clone());
    cx.run_until_parked();
    let window = add_window(&mut cx, &store, temp.path(), "remember-remote-selection");
    let visit = |cx: &mut TestAppContext, computer: Computer| {
        window
            .update(cx, |root, window, cx| {
                root.choose_computer(computer, false, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
    };
    let active = |cx: &mut TestAppContext| {
        window
            .read_with(cx, |root, _| {
                let remote = root.remote_client.workspace().unwrap();
                match &remote.active {
                    Some(Item::Tab(id)) => (remote.computer.id.clone(), id.clone()),
                    _ => panic!("expected a remote tab"),
                }
            })
            .unwrap()
    };

    visit(&mut cx, computers()[1].clone());
    assert_eq!(active(&mut cx), ("host".into(), "remote-tab".into()));
    let focused = window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(Item::Tab("project-tab".into()), cx);
            let layout = root.front_page_layout_mut().unwrap();
            let second = layout.root.leaves()[1].id.clone();
            layout.focus_pane(&second);
            second
        })
        .unwrap();

    visit(&mut cx, other.clone());
    assert_eq!(active(&mut cx), ("other".into(), "remote-tab".into()));
    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(Item::Tab("recent-tab".into()), cx)
        })
        .unwrap();

    visit(&mut cx, computers()[0].clone());
    assert!(window
        .read_with(&cx, |root, _| root.remote_client.is_local())
        .unwrap());
    visit(&mut cx, computers()[1].clone());
    assert_eq!(active(&mut cx), ("host".into(), "project-tab".into()));
    window
        .read_with(&cx, |root, _| {
            let layout = root.front_page_layout().unwrap();
            assert_eq!(layout.focused_pane_id, focused);
            assert_eq!(layout.session_ids(), ["s", "s2"]);
        })
        .unwrap();
    visit(&mut cx, other);
    assert_eq!(active(&mut cx), ("other".into(), "recent-tab".into()));

    data.tabs.retain(|tab| tab.id != "project-tab");
    fake.set_overview(data);
    cx.run_until_parked();
    visit(&mut cx, computers()[1].clone());
    assert_eq!(active(&mut cx), ("host".into(), "remote-tab".into()));
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
                root.remote_client.workspace().unwrap().terminals["s"]
                    .attachment
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
                root.remote_client.open_computer_pair(
                    computers()[1].clone(),
                    pairing::Purpose::Open { new_window: false },
                    window,
                    cx,
                );
                let field = root.remote_client.ui.pair.as_ref().unwrap().field.clone();
                field.update(cx, |field, cx| {
                    field.replace_text_in_range(None, "001234", window, cx)
                });
                root.remote_client.submit_computer_pin(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(fake.pairs.load(Ordering::SeqCst), 1);
        assert!(fake.follows.lock().unwrap()[follows_before..].contains(&("host".into(), true)));
        window
            .read_with(&cx, |root, _| {
                assert!(root.remote_client.ui.pair.is_none());
                let remote = root.remote_client.workspace().unwrap();
                assert!(remote.connection == Connection::Connected);
                assert!(!Arc::ptr_eq(
                    &original,
                    &remote.terminals["s"].attachment.terminal
                ));
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
        "linkMission",
        "unlinkMission",
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
            assert_eq!(
                root.remote_client.workspace().unwrap().layout_hints.len(),
                1
            )
        })
        .unwrap();
    pushed.tabs[0].panes = serde_json::from_value(serde_json::json!({"split":"column","sizes":[0.5,0.5],"a":{"session":"s"},"b":{"session":null}})).unwrap();
    fake.set_overview(pushed.clone());
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| {
            let layout = root.front_page_layout().unwrap();
            assert_eq!(layout.root.leaves()[1].id, draft.root.leaves()[1].id);
            assert!(root
                .remote_client
                .workspace()
                .unwrap()
                .layout_hints
                .is_empty());
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
            assert!(root
                .remote_client
                .workspace()
                .unwrap()
                .layout_hints
                .is_empty());
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
            assert!(root.remote_client.workspace().unwrap().connection == Connection::Revoked);
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
                .update(&mut cx, |root, window, cx| {
                    root.test_start_launcher(window, cx)
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
                let remote = root.remote_client.workspace().unwrap();
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
                let remote = root.remote_client.workspace().unwrap();
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
                root.remote_client.workspace_mut().unwrap().active =
                    Some(Item::Tab("removed-tab".into()));
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
        .update(&mut cx, |root, window, cx| {
            root.test_start_launcher(window, cx)
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
            let remote = root.remote_client.workspace().unwrap();
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
                        crate::keymap::TerminalCtrlKeys::RunnerFirst,
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
                        root.remote_client.focus_remote_terminal("s", window, cx);
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
                root.remote_client
                    .workspace_mut()
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
                let remote = root.remote_client.workspace().unwrap();
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
                let remote = root.remote_client.workspace().unwrap();
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
                    "chat-fill" => root.test_start_launcher(window, cx),
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
                                .remote_client
                                .workspace()
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
                    let remote = root.remote_client.workspace().unwrap();
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
                    let remote = root.remote_client.workspace().unwrap();
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
            let remote = root.remote_client.workspace().unwrap();
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
            assert!(root.remote_client.workspace().unwrap().active == Some(Item::Roles));
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
            assert!(!root.launcher_page_visible());
            assert_eq!(root.route, original_route);
            assert!(root.remote_client.workspace().unwrap().active == Some(Item::Crews));
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
            assert!(root.remote_client.is_local());
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
        assert!(visual.debug_bounds("SIDEBAR_WORKSPACE_NEW_TAB").is_none());
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
fn remote_libraries_render_on_the_work_card_not_the_island_area() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let window = add_window(&mut cx, &store, temp.path(), "library-card");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    crate::theme_snapshot::assert_fill_color(&mut visual, "APP_CONTENT_COLUMN", None);
    fake.queue_result("catalog", Ok(library_catalog(2)));
    window
        .update(&mut visual, |root, window, cx| root.open_roles(window, cx))
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("ROLE_ROW:host-role-0").is_some());
    crate::theme_snapshot::assert_fill_color(
        &mut visual,
        "APP_CONTENT_COLUMN",
        Some(crate::theme::bg()),
    );
    fake.queue_result("catalog", Ok(library_catalog(2)));
    window
        .update(&mut visual, |root, window, cx| root.open_crews(window, cx))
        .unwrap();
    visual.run_until_parked();
    crate::theme_snapshot::assert_fill_color(
        &mut visual,
        "APP_CONTENT_COLUMN",
        Some(crate::theme::bg()),
    );
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
        .update(&mut visual, |root, _, cx| {
            root.remote_client.refresh_remote_library(cx)
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("CREW_ROW:host-crew").is_none());
    assert!(visual.debug_bounds("EMPTY_NEW_CREW").is_none());
}

#[test]
fn remote_role_library_chat_starts_the_clicked_host_role_directly() {
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
    assert!(visual.debug_bounds("START_CHAT_FORM").is_none());
    let starts = action_trace(&fake, "startChat");
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0]["role"], "host-role-1");
    for field in ["runtime", "cwd", "model", "effort", "speed", "project"] {
        assert!(starts[0][field].is_null(), "{field}");
    }
    assert_eq!(action_trace(&fake, "catalog").len(), 2);
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
                    .remote_client
                    .workspace()
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
                .update(&mut visual, |root, _, cx| {
                    root.remote_client.refresh_remote_library(cx)
                })
                .unwrap();
        }
        visual.run_until_parked();
        if return_local {
            window
                .read_with(&visual, |root, _| assert!(root.remote_client.is_local()))
                .unwrap();
            assert!(visual.debug_bounds("ROLE_ROW:host-role-0").is_none());
        } else {
            assert!(visual.debug_bounds("ROLE_ROW:host-role-2").is_some());
            window
                .read_with(&visual, |root, _| {
                    assert_eq!(
                        root.remote_client
                            .workspace()
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
    fake.queue_result(
        "startChat",
        Err(runner_core::protocol::ClientError::msg("Role unavailable")),
    );
    let chat = visual.debug_bounds("ROLE_ROW_CHAT:host-role-1").unwrap();
    visual.simulate_click(chat.center(), Default::default());
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| {
            assert!(root.start_chat_modal.is_none());
        })
        .unwrap();
    assert_eq!(action_trace(&fake, "startChat")[0]["role"], "host-role-1");
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
                root.remote_client
                    .workspace()
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
                let remote = root.remote_client.workspace().unwrap();
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
fn remote_terminals_follow_the_viewer_cursor_and_selection_settings() {
    use crate::app_settings::TerminalCursorStyle;
    use alacritty_terminal::vte::ansi::CursorShape;
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, _, _) = fixture(&mut cx, temp.path());
    let set_cursor = |cx: &mut TestAppContext, style| {
        store.update(cx, |store, cx| {
            store.update_settings(
                |settings| {
                    settings.terminal_cursor_style = style;
                    true
                },
                false,
                cx,
            )
        });
        cx.run_until_parked();
    };
    set_cursor(&mut cx, TerminalCursorStyle::Bar);
    let window = add_window(&mut cx, &store, temp.path(), "remote-cursor");
    window.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    let terminal = |cx: &TestAppContext| {
        window
            .read_with(cx, |root, _| {
                let term = root.remote_client.workspace().unwrap().terminals["s"]
                    .attachment
                    .terminal
                    .term
                    .lock();
                (
                    term.cursor_style().shape,
                    term.semantic_escape_chars().to_owned(),
                )
            })
            .unwrap()
    };
    assert_eq!(
        terminal(&cx),
        (
            CursorShape::Beam,
            runner_terminal::terminal::XTERM_WORD_SEPARATORS.to_owned()
        )
    );
    set_cursor(&mut cx, TerminalCursorStyle::Underline);
    assert_eq!(terminal(&cx).0, CursorShape::Underline);
}

#[test]
fn remote_terminals_draw_with_the_host_palette_and_fall_back_to_the_viewer() {
    use runner_terminal::palette::{CATPPUCCIN_MOCHA, RUNNER};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    let window = add_window(&mut cx, &store, temp.path(), "remote-palette");
    window.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    let drawn = |cx: &TestAppContext| {
        VisualTestContext::from_window(window.into(), cx).run_until_parked();
        window
            .read_with(cx, |root, cx| {
                (
                    root.remote_client.workspace().unwrap().terminals["s"]
                        .attachment
                        .terminal
                        .palette(),
                    root.terminal_style(cx).palette,
                )
            })
            .unwrap()
    };
    let viewer = store.read_with(&cx, |store, _| {
        crate::app_settings::terminal_palette(&store.settings, crate::theme::active_variant())
    });
    assert!(![CATPPUCCIN_MOCHA, RUNNER].contains(&viewer));
    assert_eq!(drawn(&cx), (viewer, viewer));
    for host in [CATPPUCCIN_MOCHA, RUNNER] {
        let mut data = overview();
        data.terminal_palette = Some(runner_terminal::terminal::palette_to_wire(host).into());
        fake.set_overview(data);
        cx.run_until_parked();
        assert_eq!(drawn(&cx), (host, host));
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
        crate::keymap::install_bindings(
            cx,
            &crate::keymap::KeymapOverrides::new(),
            crate::keymap::TerminalCtrlKeys::RunnerFirst,
            false,
        )
    });
    let (store, fake, _) = fixture(&mut cx, temp.path());
    let window = add_window(&mut cx, &store, temp.path(), "remote-clipboard");
    window.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let terminal = window
        .update(&mut visual, |root, window, cx| {
            window.activate_window();
            root.remote_client.focus_remote_terminal("s", window, cx);
            root.remote_client.workspace().unwrap().terminals["s"]
                .attachment
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
        Some(selection.clone())
    );
    assert!(fake.input.lock().unwrap().is_empty());
    if cfg!(any(windows, target_os = "linux")) {
        assert_eq!(terminal.selection_text(), None);
    } else {
        assert_eq!(terminal.selection_text(), Some(selection));
    }
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

#[test]
fn remote_terminal_pane_shortcut_and_sidebar_close_dispatch_without_confirmation() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for entry in ["pane", "shortcut", "sidebar"] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        let mut data = overview();
        data.sessions.get_mut("s").unwrap().kind = runner_core::companion::SessionKind::Shell;
        data.sessions.get_mut("s").unwrap().runtime = Some("shell".into());
        fake.set_overview(data);
        let window = add_window(&mut cx, &store, temp.path(), "immediate-terminal-close");
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, window, cx| {
                let layout = root.front_page_layout().unwrap().clone();
                match entry {
                    "pane" => {
                        root.request_close_terminal_pane(&layout.focused_pane_id, "s", window, cx)
                    }
                    "shortcut" => root.close_remote_front_page(window, cx),
                    "sidebar" => root.request_close_terminal_tab(&layout.id, "s", window, cx),
                    _ => unreachable!(),
                }
                assert!(root.terminal_close_confirm.is_none());
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(fake.request_count("close"), 1, "{entry}");
        assert_eq!(fake.request_count("archive"), 0, "{entry}");
        window
            .read_with(&cx, |root, _| {
                assert!(root.terminal_close_confirm.is_none())
            })
            .unwrap();
    }
}

#[test]
fn remote_single_chat_tab_shortcut_archives_only_after_confirmation() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let window = add_window(&mut cx, &store, temp.path(), "confirm-chat-tab-close");
    window.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    for key in ["escape", "enter"] {
        window
            .update(&mut visual, |root, window, cx| {
                window.activate_window();
                root.remote_client.workspace.as_mut().unwrap().landed_at = None;
                let layout = root.front_page_layout().unwrap();
                assert_eq!(layout.root.leaves().len(), 1);
                assert_eq!(layout.all_session_ids(), ["s"]);
                crate::surfaces::windowing::close_tab(root, window, cx);
            })
            .unwrap();
        visual.run_until_parked();
        window
            .read_with(&visual, |root, _| {
                let Some(TerminalCloseTarget::ArchiveChatPane { session_id, close }) = root
                    .terminal_close_confirm
                    .as_ref()
                    .map(|confirm| &confirm.target)
                else {
                    panic!("{key}: the shortcut opens the archive confirmation");
                };
                assert_eq!(session_id, "s");
                assert_eq!(close.tab_id, "remote-tab");
            })
            .unwrap();
        assert_eq!(fake.request_count("archive"), 0, "{key}");
        visual.simulate_keystrokes(key);
        visual.run_until_parked();
        window
            .read_with(&visual, |root, _| {
                assert!(root.terminal_close_confirm.is_none(), "{key}")
            })
            .unwrap();
        assert_eq!(
            fake.request_count("archive"),
            usize::from(key == "enter"),
            "{key}"
        );
    }
    let archives = action_trace(&fake, "archive");
    assert_eq!(archives[0]["target"]["kind"], "session");
    assert_eq!(archives[0]["target"]["id"], "s");
}

#[cfg(target_os = "macos")]
#[test]
fn remote_library_pages_keep_the_lower_titlebar_band_draggable() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let window = add_window(&mut cx, &store, temp.path(), "library-titlebar");
    window.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    for item in [Item::Roles, Item::Crews] {
        window
            .update(&mut visual, |root, _, cx| {
                root.select_remote_item(item.clone(), cx)
            })
            .unwrap();
        visual.run_until_parked();
        let drag = visual.debug_bounds("MAIN_TITLEBAR_DRAG").unwrap();
        let chrome = visual.debug_bounds("APP_CHROME").unwrap();
        let at = gpui::point(drag.right() - px(120.), chrome.top() + px(40.));
        assert!(drag.contains(&at));
        visual.simulate_mouse_down(at, MouseButton::Left, gpui::Modifiers::default());
        window
            .read_with(&visual, |root, _| assert!(root.titlebar_drag_armed))
            .unwrap();
        visual.simulate_mouse_up(at, MouseButton::Left, gpui::Modifiers::default());
        visual.run_until_parked();
    }
}

#[test]
fn remote_title_menu_pin_runs_after_the_root_update() {
    use gpui::Focusable;
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let window = add_window(&mut cx, &store, temp.path(), "title-pin");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(workspace::Item::Tab("remote-tab".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.run_until_parked();
    let menu = window
        .read_with(&visual, |root, _| root.pane_action_menus.get("s").cloned())
        .unwrap()
        .expect("the remote chat's title menu");
    visual.update(|window, cx| menu.read(cx).focus_handle(cx).focus(window, cx));
    visual.simulate_keystrokes("down end enter");
    visual.run_until_parked();
    let pins = action_trace(&fake, "pin");
    assert_eq!(pins.len(), 1, "{pins:?}");
    assert_eq!(
        pins[0]["pinned"], false,
        "the pinned remote tab is unpinned"
    );
}

#[test]
fn reconnecting_to_or_leaving_a_host_restores_a_maximized_split() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let window = add_window(&mut cx, &store, temp.path(), "maximize-reconnect");
    window
        .update(&mut cx, |root, window, cx| {
            root.choose_computer(computers()[1].clone(), false, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let mut pushed = overview();
    pushed.tabs[0].panes = serde_json::from_value(serde_json::json!({"split":"row","sizes":[0.5,0.5],"a":{"session":"s"},"b":{"session":null}})).unwrap();
    fake.set_overview(pushed);
    cx.run_until_parked();
    let maximize = |cx: &mut TestAppContext| {
        window
            .update(cx, |root, window, cx| {
                let pane = root.front_page_layout().unwrap().focused_pane_id.clone();
                root.toggle_maximize_pane(&pane, window, cx);
                let layout = root.front_page_layout().unwrap();
                assert_eq!(layout.root.leaves().len(), 2);
                assert_eq!(
                    crate::surfaces::panes::maximized_leaf(&root.maximized_panes, layout)
                        .map(|leaf| leaf.id.clone()),
                    Some(pane.clone())
                );
                pane
            })
            .unwrap()
    };
    let pane = maximize(&mut cx);
    window
        .update(&mut cx, |root, _, cx| {
            let clients = root.app_store.read(cx).remote_clients.clone();
            clients.update(cx, |clients, cx| {
                clients.apply_event(ClientEvent {
                    name: "daemon/reconnected".into(),
                    payload: serde_json::json!({}),
                });
                cx.notify();
            });
            root.sync_remote_state(cx);
            let layout = root.front_page_layout().unwrap();
            assert_eq!(layout.root.leaves().len(), 2);
            assert_eq!(
                layout.focused_pane_id, pane,
                "the split and its focus are intact"
            );
            assert!(root.maximized_panes.is_empty());
        })
        .unwrap();
    cx.run_until_parked();

    maximize(&mut cx);
    window
        .update(&mut cx, |root, window, cx| {
            root.return_to_this_computer(window, cx);
            assert!(root.maximized_panes.is_empty());
        })
        .unwrap();
}

fn host_usage() -> serde_json::Value {
    serde_json::json!({"agents":[
        {"name":"claude-code","value":{"windows":[{"name":"Week","usedPercent":41.,"resetsAt":null}],"updatedAt":"2026-11-09T06:00:00Z"},"error":null},
        {"name":"codex","value":{"windows":[{"name":"Week","usedPercent":12.,"resetsAt":null}],"credits":{"balance":"1250.75","hasCredits":true,"unlimited":false},"updatedAt":"2026-11-09T06:00:00Z"},"error":null},
        {"name":"future-agent","value":null,"error":"futureReason"}
    ],"refreshing":false,"activeClaudeAccount":"Host Work"})
}

#[test]
fn remote_footer_and_readonly_popover_use_the_hosts_usage_and_keep_it_while_disconnected() {
    use runner_core::protocol::model::Runtime;
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    fake.set_features(&["overview", "usage", "ping"]);
    store.update(&mut cx, |store, _| {
        store.usage.runtimes.insert(
            Runtime::Codex,
            runner_core::protocol::usage::RuntimeUsage {
                value: Some(runner_core::protocol::usage::AgentUsage {
                    credits: Some(runner_core::protocol::UsageCredits {
                        balance: Some("9.5".into()),
                        has_credits: true,
                        unlimited: false,
                    }),
                    windows: vec![runner_core::protocol::usage::UsageWindow {
                        name: "Week".into(),
                        used_percent: 88.,
                        resets_at: None,
                    }],
                    updated_at: chrono::Utc::now(),
                }),
                error: None,
            },
        );
        *fake.local_usage.lock().unwrap() = store.usage.clone();
    });
    let window = add_window(&mut cx, &store, temp.path(), "remote-usage");
    window
        .update(&mut cx, |root, _, cx| {
            root.usage_installed = vec![Runtime::Codex];
            cx.notify();
        })
        .unwrap();
    window
        .read_with(&cx, |root, cx| {
            assert_eq!(
                root.sidebar_usage(cx)
                    .0
                    .get(Runtime::Codex)
                    .unwrap()
                    .windows[0]
                    .used_percent,
                88.
            )
        })
        .unwrap();
    window.update(&mut cx, choose_host).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    visual.run_until_parked();
    assert_eq!(fake.request_count("usage"), 1);
    window
        .read_with(&visual, |root, cx| {
            let (snapshot, agents, dimmed) = root.sidebar_usage(cx);
            assert_eq!(agents, vec![Runtime::Codex, Runtime::ClaudeCode]);
            assert_eq!(
                snapshot.get(Runtime::Codex).unwrap().windows[0].used_percent,
                12.
            );
            assert_eq!(
                snapshot.get(Runtime::ClaudeCode).unwrap().windows[0].used_percent,
                41.
            );
            assert_eq!(
                snapshot
                    .get(Runtime::Codex)
                    .unwrap()
                    .credits
                    .as_ref()
                    .unwrap()
                    .balance
                    .as_deref(),
                Some("1250.75")
            );
            assert!(!dimmed);
            assert_eq!(root.remote_client.remote_usage(cx).unwrap().2, "Studio");
        })
        .unwrap();
    let usage = visual.debug_bounds("SIDEBAR_USAGE").unwrap();
    visual.simulate_click(usage.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("USAGE_POPOVER_PANEL").is_some());
    assert!(visual.debug_bounds("USAGE_CREDITS").is_some());
    assert!(visual.debug_bounds("USAGE_AGENT_SETTINGS").is_none());
    assert!(visual.debug_bounds("USAGE_REFRESH").is_none());
    assert!(visual.debug_bounds("USAGE_REMOTE_ACCOUNT").is_some());
    assert!(visual.debug_bounds("USAGE_SECTION_claude-code").is_some());
    assert_eq!(fake.request_count("usage"), 2);
    assert_eq!(action_trace(&fake, "usage")[1]["popover"], true);
    for state in ["reconnecting", "unreachable"] {
        fake.emit(
            "remote/connection",
            serde_json::json!({"computer":"host","state":state}),
        );
        visual.run_until_parked();
        window
            .read_with(&visual, |root, cx| {
                let (snapshot, _, dimmed) = root.sidebar_usage(cx);
                assert!(dimmed);
                assert_eq!(
                    snapshot
                        .get(Runtime::Codex)
                        .unwrap()
                        .credits
                        .as_ref()
                        .unwrap()
                        .balance
                        .as_deref(),
                    Some("1250.75")
                );
                assert_eq!(
                    snapshot.get(Runtime::Codex).unwrap().windows[0].used_percent,
                    12.
                );
            })
            .unwrap();
    }
    assert!(clients.read_with(&visual, |clients, _| clients.usage.contains_key("host")));
    window
        .update(&mut visual, |root, window, cx| {
            root.return_to_this_computer(window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    window
        .read_with(&visual, |root, cx| {
            assert_eq!(
                root.sidebar_usage(cx)
                    .0
                    .get(Runtime::Codex)
                    .unwrap()
                    .windows[0]
                    .used_percent,
                88.
            )
        })
        .unwrap();
    assert!(visual.debug_bounds("USAGE_POPOVER_PANEL").is_none());
}

#[test]
fn usage_requests_are_shared_by_windows_and_follow_the_periodic_and_popover_intervals() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    fake.set_features(&["overview", "usage"]);
    let first = add_window(&mut cx, &store, temp.path(), "usage-first");
    let second = add_window(&mut cx, &store, temp.path(), "usage-second");
    first.update(&mut cx, choose_host).unwrap();
    second.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    assert_eq!(fake.request_count("usage"), 1);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(58));
    cx.run_until_parked();
    assert_eq!(fake.request_count("usage"), 1);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(fake.request_count("usage"), 2);
    let mut refreshing = host_usage();
    refreshing["refreshing"] = true.into();
    fake.queue_result("usage", Ok(refreshing));
    let mut visual = VisualTestContext::from_window(first.into(), &cx);
    visual.run_until_parked();
    let trigger = visual.debug_bounds("SIDEBAR_USAGE").unwrap();
    visual.simulate_click(trigger.center(), Default::default());
    visual.run_until_parked();
    assert_eq!(fake.request_count("usage"), 3);
    visual
        .executor()
        .advance_clock(std::time::Duration::from_secs(2));
    visual.run_until_parked();
    assert_eq!(fake.request_count("usage"), 4);
    visual
        .executor()
        .advance_clock(std::time::Duration::from_secs(2));
    visual.run_until_parked();
    assert_eq!(fake.request_count("usage"), 4, "refresh completed");
    for window in [first, second] {
        window
            .update(&mut visual, |root, window, cx| {
                root.return_to_this_computer(window, cx)
            })
            .unwrap();
    }
    visual.run_until_parked();
    visual
        .executor()
        .advance_clock(std::time::Duration::from_secs(60));
    visual.run_until_parked();
    assert_eq!(
        fake.request_count("usage"),
        4,
        "no remote window showing the host"
    );
}

#[test]
fn a_host_without_usage_shows_the_update_hint_and_receives_no_usage_request() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    fake.set_features(&["overview"]);
    let window = add_window(&mut cx, &store, temp.path(), "no-usage");
    window
        .update(&mut cx, |root, window, cx| {
            root.usage_installed = vec![runner_core::protocol::model::Runtime::Codex];
            choose_host(root, window, cx);
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    visual.run_until_parked();
    assert!(visual.debug_bounds("SIDEBAR_USAGE").is_none());
    let hint = visual.debug_bounds("SIDEBAR_USAGE_UPDATE").unwrap();
    visual.simulate_click(hint.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("USAGE_OLDER_HOST").is_some());
    assert!(visual.debug_bounds("USAGE_SECTION_codex").is_none());
    assert_eq!(fake.request_count("usage"), 0);
    window
        .update(&mut visual, |root, window, cx| {
            root.return_to_this_computer(window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SIDEBAR_USAGE_UPDATE").is_none());
    assert!(visual.debug_bounds("SIDEBAR_USAGE").is_some());
}

#[test]
fn the_update_hint_fits_beside_the_footer_buttons_at_every_width_zoom_and_language() {
    use crate::i18n::{testing::with_locale, Locale};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            for width in [
                crate::app_settings::SIDEBAR_MIN,
                crate::app_settings::SIDEBAR_DEFAULT,
            ] {
                for zoom in [1., 1.5, 2.] {
                    let temp = tempfile::tempdir().unwrap();
                    let mut cx = TestAppContext::single();
                    let (store, fake, _) = fixture(&mut cx, temp.path());
                    fake.set_features(&["overview"]);
                    store.update(&mut cx, |store, _| {
                        store.settings.sidebar_width = width;
                        store.settings.app_zoom = zoom;
                    });
                    let window = add_window(&mut cx, &store, temp.path(), "hint-width");
                    window.update(&mut cx, choose_host).unwrap();
                    let mut visual = VisualTestContext::from_window(window.into(), &cx);
                    visual.simulate_resize(size(px(1200.), px(900.)));
                    visual.run_until_parked();
                    let footer = visual.debug_bounds("SIDEBAR_FOOTER").unwrap();
                    let hint = visual.debug_bounds("SIDEBAR_USAGE_UPDATE").unwrap();
                    let text = visual.debug_bounds("SIDEBAR_USAGE_UPDATE_TEXT").unwrap();
                    let gear = visual.debug_bounds("SIDEBAR_SETTINGS").unwrap();
                    let case = format!("{locale:?} {width} {zoom}");
                    assert!(hint.left() >= footer.left(), "{case}");
                    assert!(hint.right() <= gear.left(), "{case}: {hint:?} {gear:?}");
                    assert!(
                        gear.right() <= footer.right(),
                        "{case}: {gear:?} {footer:?}"
                    );
                    assert!(text.right() <= hint.right(), "{case}: {text:?} {hint:?}");
                    assert!(
                        (hint.size.height - px(32. * zoom)).abs() <= px(1.),
                        "{case}"
                    );
                }
            }
        });
    }
}

#[test]
fn a_host_with_usage_shows_no_update_hint() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    fake.set_features(&["overview", "usage"]);
    let window = add_window(&mut cx, &store, temp.path(), "with-usage");
    window.update(&mut cx, choose_host).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.run_until_parked();
    assert!(visual.debug_bounds("SIDEBAR_USAGE_UPDATE").is_none());
}

#[test]
fn usage_keeps_the_last_reply_after_failures_and_reentry_but_forget_removes_it() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = fixture(&mut cx, temp.path());
    fake.set_features(&["overview", "usage"]);
    let window = add_window(&mut cx, &store, temp.path(), "usage-cache");
    window.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    assert_eq!(fake.request_count("usage"), 1);
    fake.queue_result("usage", Err(ClientError::msg("temporary failure")));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(fake.request_count("usage"), 2);
    assert!(clients.read_with(&cx, |clients, _| clients.usage.contains_key("host")));
    window
        .update(&mut cx, |root, window, cx| {
            root.return_to_this_computer(window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert!(clients.read_with(&cx, |clients, _| clients.usage.contains_key("host")));
    window.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    assert_eq!(fake.request_count("usage"), 3);
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"reconnecting"}),
    );
    cx.run_until_parked();
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"connected"}),
    );
    cx.run_until_parked();
    assert_eq!(
        fake.request_count("usage"),
        4,
        "reconnection requests usage immediately"
    );
    clients.update(&mut cx, |clients, cx| {
        clients.apply(model::Update::Forgotten("host".into(), true));
        cx.notify();
    });
    cx.run_until_parked();
    assert!(!clients.read_with(&cx, |clients, _| clients.usage.contains_key("host")));
}

pub(crate) fn update_gate_error() -> ClientError {
    ClientError {
        message: "Codex needs an update".into(),
        version_gate: Some(runner_core::protocol::RuntimeVersionGate {
            runtime: runner_core::protocol::Runtime::Codex,
            installed_version: "0.147.0".into(),
            min_version: "0.149.0".into(),
        }),
    }
}

pub(crate) fn completed_update() -> serde_json::Value {
    serde_json::to_value(runner_core::protocol::RuntimeUpdateTask {
        id: "host-update".into(),
        runtime: runner_core::protocol::Runtime::Codex,
        state: runner_core::protocol::RuntimeUpdateState::Succeeded,
        from_version: Some("0.147.0".into()),
        to_version: Some("0.167.0".into()),
        installed_version: Some("0.167.0".into()),
        step: "Done".into(),
        message: None,
        exit_code: Some(0),
        running_sessions: 0,
    })
    .unwrap()
}

/// Presses and releases where the element is drawn at each step: a frame landing between
/// reading its bounds and the press can move it, and GPUI drops a click whose press or
/// release misses its hitbox.
fn click_drawn(visual: &mut VisualTestContext, selector: &'static str) {
    let center = |visual: &mut VisualTestContext| {
        visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"))
            .center()
    };
    let at = center(visual);
    visual.simulate_mouse_move(at, None, Default::default());
    visual.run_until_parked();
    let at = center(visual);
    visual.simulate_mouse_down(at, MouseButton::Left, Default::default());
    visual.run_until_parked();
    let at = center(visual);
    visual.simulate_mouse_up(at, MouseButton::Left, Default::default());
    visual.run_until_parked();
}

#[test]
fn remote_resume_and_fork_refusals_offer_host_scoped_update_recovery() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for action in ["resume", "fork"] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        fake.features
            .lock()
            .unwrap()
            .extend(["runtimeTaskStart".into(), "runtimeTask".into()]);
        let mut data = overview();
        data.sessions.get_mut("s").unwrap().status.status = runner_core::companion::Status::Stopped;
        fake.set_overview(data);
        fake.queue_result(action, Err(update_gate_error()));
        fake.queue_result("runtimeTaskStart", Ok(completed_update()));
        let window = add_window(&mut cx, &store, temp.path(), action);
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, window, cx| {
                if action == "resume" {
                    let pane = root.front_page_layout().unwrap().focused_pane_id.clone();
                    root.resume_chat(&pane, "s", window, cx);
                } else {
                    root.fork_chat("s", window, cx);
                    root.confirm_fork_chat(window, cx);
                }
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.run_until_parked();
        let selector = if action == "resume" {
            "AGENT_RECOVERY_UPDATE:s"
        } else {
            "launcher-update-wait-primary"
        };
        if action == "resume" {
            assert!(
                visual.debug_bounds(selector).is_some(),
                "remote resume must render its update card"
            );
            click_drawn(&mut visual, selector);
        } else {
            // The fork refusal is a launcher card using the same stopped-card action.
            click_drawn(&mut visual, "LAUNCHER_UPDATE_ACTION");
        }
        let chat_error = window
            .read_with(&visual, |root, _| root.chat_error.clone())
            .unwrap();
        assert_eq!(
            fake.request_count("runtimeTaskStart"),
            1,
            "{action}: chat error {chat_error:?}"
        );
        assert_eq!(
            fake.request_count(action),
            2,
            "{action} retries the original host operation"
        );
        window
            .read_with(&visual, |root, _| {
                assert!(
                    root.agent_tasks.tasks.is_empty(),
                    "host tasks never enter the local list"
                );
                assert!(root.agent_update.is_none(), "no automatic dialog");
            })
            .unwrap();
    }
}

#[test]
fn split_close_completion_restores_remote_keyboard_input_without_stealing_navigation() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for kind in ["shell", "chat", "empty"] {
        for navigation in ["stay", "sibling", "tab", "input", "late-tab", "late-input"] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, _) = fixture(&mut cx, temp.path());
            enable_front_page(&fake);
            fake.model_session_removal.store(true, Ordering::SeqCst);
            let mut data = overview();
            let mut survivor = data.sessions["s"].clone();
            survivor.runtime = Some("shell".into());
            data.sessions.insert("survivor".into(), survivor);
            if kind == "shell" {
                data.sessions.get_mut("s").unwrap().runtime = Some("shell".into());
            }
            data.tabs[0].panes = serde_json::from_value(serde_json::json!({
                "split":"row","sizes":[0.5,0.5],
                "a":{"id":"closing", "session": if kind == "empty" { None } else { Some("s") }},
                "b":{"id":"surviving", "session":"survivor"}
            }))
            .unwrap();
            let mut other = data.tabs[0].clone();
            other.id = "other-tab".into();
            other.panes =
                serde_json::from_value(serde_json::json!({"id":"other", "session":"survivor"}))
                    .unwrap();
            data.tabs.push(other);
            fake.set_overview(data);
            let window = add_window(&mut cx, &store, temp.path(), "remote-close-focus");
            window.update(&mut cx, choose_host).unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            let (closing, surviving) = window
                .update(&mut visual, |root, _, _| {
                    let leaves = root.front_page_layout().unwrap().root.leaves();
                    (leaves[0].id.clone(), leaves[1].id.clone())
                })
                .unwrap();
            window
                .update(&mut visual, |root, window, cx| {
                    root.focus_pane(
                        if navigation == "sibling" {
                            &surviving
                        } else {
                            &closing
                        },
                        cx,
                    );
                    root.focus_active_terminal(window, cx);
                })
                .unwrap();
            visual.run_until_parked();
            let mut input_focus = None;
            window
                .update(&mut visual, |root, window, cx| {
                    match kind {
                        "empty" => root.close_pane(&closing, window, cx),
                        "shell" => root.close_terminal_pane(&closing, "s", window, cx),
                        _ => {
                            root.request_close_chat_pane(&closing, "s", window, cx);
                            root.confirm_terminal_close(window, cx);
                        }
                    }
                    match navigation {
                        "tab" => {
                            root.select_remote_item(RemoteItem::Tab("other-tab".into()), cx);
                            root.focus_active_terminal(window, cx);
                        }
                        "input" => {
                            let find = root.remote_client.workspace().unwrap().terminals
                                ["survivor"]
                                .attachment
                                .terminal_find
                                .clone();
                            find.update(cx, |find, cx| find.open(window, cx));
                            input_focus = window.focused(cx);
                        }
                        _ => {}
                    }
                })
                .unwrap();
            visual.run_until_parked();
            window
                .update(&mut visual, |root, window, cx| match navigation {
                    "late-tab" => {
                        root.select_remote_item(RemoteItem::Tab("other-tab".into()), cx);
                        root.focus_active_terminal(window, cx);
                    }
                    "late-input" => {
                        let find = root.remote_client.workspace().unwrap().terminals["survivor"]
                            .attachment
                            .terminal_find
                            .clone();
                        find.update(cx, |find, cx| find.open(window, cx));
                        input_focus = window.focused(cx);
                    }
                    _ => {}
                })
                .unwrap();
            visual.run_until_parked();
            let completed_overview = fake.overview.lock().unwrap().clone();
            fake.set_overview(completed_overview);
            visual.run_until_parked();
            window
                .update(&mut visual, |root, window, cx| {
                    let layout = root.front_page_layout().unwrap();
                    assert_eq!(
                        layout.id,
                        if matches!(navigation, "tab" | "late-tab") {
                            "other-tab"
                        } else {
                            "remote-tab"
                        }
                    );
                    assert_eq!(layout.root.leaves().len(), 1, "{kind}/{navigation}");
                    if matches!(navigation, "input" | "late-input") {
                        assert_eq!(window.focused(cx), input_focus, "{kind}/{navigation}");
                    } else {
                        assert!(
                            root.remote_client.workspace().unwrap().terminals["survivor"]
                                .attachment
                                .terminal_focus
                                .is_focused(window),
                            "{kind}/{navigation}"
                        );
                    }
                })
                .unwrap();
            if !matches!(navigation, "input" | "late-input") {
                fake.input.lock().unwrap().clear();
                visual.simulate_keystrokes("x");
                visual.run_until_parked();
                assert!(
                    fake.input
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|(address, bytes)| address.contains("survivor") && bytes == b"x"),
                    "{kind}/{navigation}"
                );
            }
        }
    }
}

#[test]
fn remote_archive_intermediate_launcher_waits_for_collapse_without_stealing_find_focus() {
    use crate::surfaces::chat::PaneCloseFocus;
    use crate::surfaces::front_page::actions::{Completion, WorkspaceAction};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for navigation in ["stay", "find-before", "find-after"] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        fake.model_session_removal.store(true, Ordering::SeqCst);
        let mut data = overview();
        let mut survivor = data.sessions["s"].clone();
        survivor.runtime = Some("shell".into());
        data.sessions.insert("survivor".into(), survivor);
        data.tabs[0].panes = serde_json::from_value(serde_json::json!({
            "split":"row","sizes":[0.5,0.5],
            "a":{"session":"s"},"b":{"session":"survivor"}
        }))
        .unwrap();
        fake.set_overview(data);
        let window = add_window(&mut cx, &store, temp.path(), "intermediate-remote-archive");
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        let (focus, action, final_layout, mut input_focus) = window
            .update(&mut visual, |root, window, cx| {
                let layout = root.front_page_layout().unwrap().clone();
                let pane = layout.root.leaves()[0].id.clone();
                root.focus_pane(&pane, cx);
                root.focus_active_terminal(window, cx);
                let focus = PaneCloseFocus::capture(root, window, cx).unwrap();
                let (action, _) = WorkspaceAction::archive(
                    root,
                    vec!["s".into()],
                    None,
                    Some(crate::PendingPaneClose {
                        tab_id: layout.id.clone(),
                        pane_id: pane,
                    }),
                    cx,
                )
                .unwrap();
                let final_layout =
                    root.remote_client.workspace().unwrap().closing_layouts[&layout.id].clone();
                if navigation == "find-before" {
                    let find = root.remote_client.workspace().unwrap().terminals["survivor"]
                        .attachment
                        .terminal_find
                        .clone();
                    find.update(cx, |find, cx| find.open(window, cx));
                }
                (focus, action, final_layout, window.focused(cx))
            })
            .unwrap();
        action
            .client
            .archive(runner_core::companion::ActionTarget {
                kind: runner_core::companion::ActionKind::Session,
                id: "s".into(),
            })
            .unwrap();
        let intermediate_overview = fake.overview.lock().unwrap().clone();
        fake.set_overview(intermediate_overview);
        visual.run_until_parked();
        window
            .update(&mut visual, |root, window, cx| {
                let layout = root.front_page_layout().unwrap();
                assert_eq!(layout.root.leaves().len(), 2);
                assert_eq!(layout.focused_session_id(), None);
                assert!(
                    !root.launchers.is_empty(),
                    "the intermediate launcher must render"
                );
                assert_eq!(
                    window.focused(cx),
                    input_focus,
                    "intermediate launcher stole focus: {navigation}"
                );
                if navigation == "find-after" {
                    let find = root.remote_client.workspace().unwrap().terminals["survivor"]
                        .attachment
                        .terminal_find
                        .clone();
                    find.update(cx, |find, cx| find.open(window, cx));
                    input_focus = window.focused(cx);
                }
            })
            .unwrap();
        action
            .client
            .layout(&final_layout, final_layout.parent_id.clone())
            .unwrap();
        window
            .update(&mut visual, |root, window, cx| {
                WorkspaceAction::complete_observed(
                    root,
                    action,
                    Ok::<_, String>(Completion {
                        layouts: vec![final_layout],
                        ..Default::default()
                    }),
                    |done| done,
                    |_, _, succeeded, cx| {
                        assert!(succeeded);
                        focus.defer_restore(window, cx);
                    },
                    cx,
                );
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(&mut visual, |root, window, cx| {
                assert!(root.remote_client.pane_close_pending("remote-tab"));
                assert_eq!(
                    window.focused(cx),
                    input_focus,
                    "acknowledgement focused the interim launcher"
                );
            })
            .unwrap();
        let final_overview = fake.overview.lock().unwrap().clone();
        fake.set_overview(final_overview);
        visual.run_until_parked();
        window
            .update(&mut visual, |root, window, cx| {
                assert!(!root.remote_client.pane_close_pending("remote-tab"));
                assert_eq!(root.front_page_layout().unwrap().root.leaves().len(), 1);
                if navigation == "stay" {
                    assert!(
                        root.remote_client.workspace().unwrap().terminals["survivor"]
                            .attachment
                            .terminal_focus
                            .is_focused(window)
                    );
                } else {
                    assert_eq!(window.focused(cx), input_focus);
                }
            })
            .unwrap();
        fake.input.lock().unwrap().clear();
        visual.simulate_keystrokes("x");
        visual.run_until_parked();
        if navigation == "stay" {
            assert!(fake
                .input
                .lock()
                .unwrap()
                .iter()
                .any(|(address, bytes)| address.contains("survivor") && bytes == b"x"));
        } else {
            assert!(
                fake.input.lock().unwrap().is_empty(),
                "Find input reached the terminal"
            );
        }
    }
}

#[test]
fn remote_chat_archive_prompts_for_linked_missions_and_supports_new_and_legacy_hosts() {
    for legacy_host in [true, false] {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);

        let mut data = fake.overview.lock().unwrap().clone();
        data.missions[0].linked_chat = Some(runner_core::companion::LinkedChat {
            session_id: "s".into(),
            title: "Remote chat".into(),
            archived: false,
            reports: true,
        });
        let mut other = data.missions[0].clone();
        other.id = "other-mission".into();
        other.title = "Other mission".into();
        other.linked_chat = None;
        data.missions.push(other);
        fake.set_overview(data);
        cx.run_until_parked();

        let window = add_window(&mut cx, &store, temp.path(), "remote-linked-archive");
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);

        // Sidebar/tab archive on a remote chat with a linked mission opens Option A instead of archiving silently.
        window
            .update(&mut visual, |root, window, cx| {
                window.activate_window();
                root.request_archive_all(
                    Some("remote-tab"),
                    vec!["s".into()],
                    crate::ArchiveAllSource::Sidebar,
                    window,
                    cx,
                );
                assert!(
                    root.terminal_close_confirm.is_some(),
                    "legacy_host={legacy_host}: linked remote chat opens confirmation"
                );
            })
            .unwrap();
        visual.update(|window, _| window.refresh());
        visual.run_until_parked();
        assert!(visual
            .debug_bounds("CONFIRM_LINKED_MISSION remote-mission")
            .is_some());
        assert!(visual
            .debug_bounds("CONFIRM_LINKED_MISSION other-mission")
            .is_none());

        // Escape cancels with zero archive requests.
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        assert_eq!(fake.request_count("archive"), 0);

        if !legacy_host {
            fake.queue_result(
                "archive",
                Ok(serde_json::json!({"archivedMissions": ["remote-mission"]})),
            );
        }

        window
            .update(&mut visual, |root, window, cx| {
                root.request_archive_all(
                    Some("remote-tab"),
                    vec!["s".into()],
                    crate::ArchiveAllSource::Sidebar,
                    window,
                    cx,
                );
            })
            .unwrap();
        visual.run_until_parked();
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();

        let archives = action_trace(&fake, "archive");
        assert_eq!(archives[0]["target"]["kind"], "session");
        assert_eq!(archives[0]["target"]["id"], "s");
        assert_eq!(archives[0]["withMissions"], true);
        if legacy_host {
            assert_eq!(
                archives.len(),
                2,
                "legacy host returning null triggers explicit mission archive fallback"
            );
            assert_eq!(archives[1]["target"]["kind"], "mission");
            assert_eq!(archives[1]["target"]["id"], "remote-mission");
            assert!(archives[1].get("withMissions").is_none());
        } else {
            assert_eq!(
                archives.len(),
                1,
                "withMissions-aware host archives chat and linked missions in one request"
            );
        }
    }
}

#[test]
fn remote_chat_archive_tracks_linked_mission_pending_state_across_hosts_and_errors() {
    use crate::surfaces::sidebar::AttentionState;
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);

    let host_a = computers()[1].clone();
    let host_b = Computer {
        id: "host-b".into(),
        name: "Backup".into(),
        online: true,
        paired: true,
        fingerprint: "host-b-key".into(),
        ..Default::default()
    };
    fake.computers.lock().unwrap().push(host_b.clone());

    let mut data: Overview = serde_json::from_value(serde_json::json!({
        "projects": [{"id": "p", "name": "Remote project"}],
        "tabs": [{"id": "remote-tab", "project": "p", "name": "Remote tab", "pinned": true, "panes": {"session": "s"}, "shells": []}],
        "missions": [{"id": "remote-mission", "title": "Remote mission", "project": null, "crew": "Duo", "state": "stopped", "pinned": false, "members": [], "asks": [], "shells": []}],
        "sessions": {"s": {"kind": "chat", "title": "Remote session", "runtime": "codex", "status": "stopped", "unread": false}}
    }))
    .unwrap();
    data.missions[0].linked_chat = Some(runner_core::companion::LinkedChat {
        session_id: "s".into(),
        title: "Remote session".into(),
        archived: false,
        reports: true,
    });
    fake.set_overview(data.clone());
    fake.emit(
        "remote/overview",
        serde_json::json!({"computer": "host-b", "overview": data}),
    );
    cx.run_until_parked();

    let window = add_window(&mut cx, &store, temp.path(), "remote-pending-archive");
    window.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();

    // Case 1: Staying on Host A: in-flight state marks both chat and linked mission and blocks
    // duplicates, then an error clears both marks and surfaces action_error for retry.
    fake.queue_result("archive", Err(ClientError::msg("archive refused")));
    window
        .update(&mut cx, |root, window, cx| {
            let attentions = root.front_page_sidebar().read(cx).remote_row_attentions(cx);
            assert_eq!(attentions.get("remote-tab"), Some(&AttentionState::None));
            assert_eq!(
                attentions.get("remote-mission"),
                Some(&AttentionState::None)
            );
            root.request_archive_all(
                Some("remote-tab"),
                vec!["s".into()],
                crate::ArchiveAllSource::Sidebar,
                window,
                cx,
            );
            assert!(root.terminal_close_confirm.is_some());
            root.confirm_terminal_close(window, cx);

            assert!(root.sidebar_archiving_session("s", cx));
            assert!(root.sidebar_archiving_mission("remote-mission", cx));
            let attentions = root.front_page_sidebar().read(cx).remote_row_attentions(cx);
            assert_eq!(attentions.get("remote-tab"), Some(&AttentionState::Working));
            assert_eq!(
                attentions.get("remote-mission"),
                Some(&AttentionState::Working)
            );
            root.request_archive_all(
                Some("remote-tab"),
                vec!["s".into()],
                crate::ArchiveAllSource::Sidebar,
                window,
                cx,
            );
            assert!(root.terminal_close_confirm.is_none());
            root.remote_archive_sessions(vec!["s".into()], cx);
        })
        .unwrap();

    cx.run_until_parked();
    assert_eq!(fake.request_count("archive"), 1);
    window
        .read_with(&cx, |root, cx| {
            assert!(root
                .remote_client
                .workspace()
                .unwrap()
                .action_error
                .is_some());
            assert!(!root.sidebar_archiving_session("s", cx));
            assert!(!root.sidebar_archiving_mission("remote-mission", cx));
            let attentions = root.front_page_sidebar().read(cx).remote_row_attentions(cx);
            assert_eq!(attentions.get("remote-tab"), Some(&AttentionState::None));
            assert_eq!(
                attentions.get("remote-mission"),
                Some(&AttentionState::None)
            );
        })
        .unwrap();

    // Case 2: Retry succeeds even across host switches (Host A -> Host B -> Local -> Host A while in
    // flight, and finishing after epoch change).
    fake.queue_result(
        "archive",
        Ok(serde_json::json!({"archivedMissions": ["remote-mission"]})),
    );
    window
        .update(&mut cx, |root, window, cx| {
            root.request_archive_all(
                Some("remote-tab"),
                vec!["s".into()],
                crate::ArchiveAllSource::Sidebar,
                window,
                cx,
            );
            assert!(root.terminal_close_confirm.is_some());
            root.confirm_terminal_close(window, cx);
            assert!(root.sidebar_archiving_session("s", cx));
            assert!(root.sidebar_archiving_mission("remote-mission", cx));

            // Switching to another computer or This Computer never leaks Host A's pending marks.
            root.visit_computer(host_b.clone(), window, cx);
            assert!(!root.sidebar_archiving_session("s", cx));
            assert!(!root.sidebar_archiving_mission("remote-mission", cx));
            let attentions = root.front_page_sidebar().read(cx).remote_row_attentions(cx);
            assert_eq!(attentions.get("remote-tab"), Some(&AttentionState::None));
            assert_eq!(
                attentions.get("remote-mission"),
                Some(&AttentionState::None)
            );

            root.return_to_this_computer(window, cx);
            assert!(!root.sidebar_archiving_session("s", cx));
            assert!(!root.sidebar_archiving_mission("remote-mission", cx));

            // Switching back to Host A while in flight restores the archiving state.
            root.visit_computer(host_a.clone(), window, cx);
            assert!(root.sidebar_archiving_session("s", cx));
            assert!(root.sidebar_archiving_mission("remote-mission", cx));
            let attentions = root.front_page_sidebar().read(cx).remote_row_attentions(cx);
            assert_eq!(attentions.get("remote-tab"), Some(&AttentionState::Working));
            assert_eq!(
                attentions.get("remote-mission"),
                Some(&AttentionState::Working)
            );
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.request_count("archive"), 2);
    window
        .read_with(&cx, |root, cx| {
            assert!(!root.sidebar_archiving_session("s", cx));
            assert!(!root.sidebar_archiving_mission("remote-mission", cx));
            let attentions = root.front_page_sidebar().read(cx).remote_row_attentions(cx);
            assert_eq!(attentions.get("remote-tab"), Some(&AttentionState::None));
            assert_eq!(
                attentions.get("remote-mission"),
                Some(&AttentionState::None)
            );
        })
        .unwrap();

    // Case 3: Completing while viewing Host B cleans up Host A so returning shows no stale marks.
    fake.queue_result(
        "archive",
        Ok(serde_json::json!({"archivedMissions": ["remote-mission"]})),
    );
    window
        .update(&mut cx, |root, window, cx| {
            root.request_archive_all(
                Some("remote-tab"),
                vec!["s".into()],
                crate::ArchiveAllSource::Sidebar,
                window,
                cx,
            );
            root.confirm_terminal_close(window, cx);
            assert!(root.sidebar_archiving_session("s", cx));
            assert!(root.sidebar_archiving_mission("remote-mission", cx));
            root.visit_computer(host_b, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.request_count("archive"), 3);

    window
        .update(&mut cx, |root, window, cx| {
            root.visit_computer(host_a, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .read_with(&cx, |root, cx| {
            assert!(!root.sidebar_archiving_session("s", cx));
            assert!(!root.sidebar_archiving_mission("remote-mission", cx));
            let attentions = root.front_page_sidebar().read(cx).remote_row_attentions(cx);
            assert_eq!(attentions.get("remote-tab"), Some(&AttentionState::None));
            assert_eq!(
                attentions.get("remote-mission"),
                Some(&AttentionState::None)
            );
        })
        .unwrap();

    // Case 4: Clearing pending archives for a disconnected host clears both marks cleanly.
    fake.queue_result(
        "archive",
        Ok(serde_json::json!({"archivedMissions": ["remote-mission"]})),
    );
    window
        .update(&mut cx, |root, window, cx| {
            root.request_archive_all(
                Some("remote-tab"),
                vec!["s".into()],
                crate::ArchiveAllSource::Sidebar,
                window,
                cx,
            );
            root.confirm_terminal_close(window, cx);
            assert!(root.sidebar_archiving_session("s", cx));
            assert!(root.sidebar_archiving_mission("remote-mission", cx));
            root.remote_client.clear_pending_archives_for("host", cx);
            assert!(!root.sidebar_archiving_session("s", cx));
            assert!(!root.sidebar_archiving_mission("remote-mission", cx));
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.request_count("archive"), 4);
}
