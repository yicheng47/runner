use super::*;
use crate::{companion::core::DaemonCore, repo, session::runtime::*, AppCore};
use serde_json::{json, Value};

#[derive(Default)]
struct Runtime {
    spawned: tokio::sync::Notify,
    spawns: StdMutex<Vec<(SpawnSpec, Option<std::sync::mpsc::Sender<RuntimeOutput>>)>>,
    inputs: StdMutex<Vec<(String, Vec<u8>)>>,
    input_ready: tokio::sync::Notify,
}

#[tokio::test]
async fn stalled_remote_socket_does_not_delay_local_input_on_the_same_ipc_connection() {
    let host = Host::new().await;
    let local = host.start().await;
    host.wait_live(&local).await;
    let root = super::super::super::tests::socket_root();
    let mut core = host.core.as_ref().clone();
    core.remote = super::super::super::RemoteHandle {
        client: Some(host.fixture.client.clone()),
        ..Default::default()
    };
    let server = super::super::super::tests::SocketHarness::new(root.path(), core).await;
    let ipc = server.client(Duration::from_secs(2)).await;
    let writer = host.fixture.relay.writer.clone();
    writer.blocked.store(true, Ordering::Release);
    let client = host.fixture.client.clone();
    let pending = tokio::spawn(async move { client.overview("2".into()).await });
    writer.entered.notified().await;
    let remote = runner_core::remote::terminal_address("2", &local);
    let caller = ipc.clone();
    let session = local.clone();
    tokio::task::spawn_blocking(move || {
        caller.input(&remote, b"remote").unwrap();
        caller.input(&session, b"local").unwrap();
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let ready = host.runtime.input_ready.notified();
            if host
                .runtime
                .inputs
                .lock()
                .unwrap()
                .iter()
                .any(|(id, bytes)| id == &local && bytes == b"local")
            {
                break;
            }
            ready.await;
        }
    })
    .await
    .expect("local input must not wait for the stalled remote writer");
    writer.blocked.store(false, Ordering::Release);
    writer.waker.wake();
    pending.await.unwrap().unwrap();
    crate::ops::session::session_stop(&host.core, &local).unwrap();
}
impl SessionRuntime for Runtime {
    fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = RuntimeSession {
            runtime: "fake".into(),
            session_id: spec.session_id.clone(),
        };
        self.spawns.lock().unwrap().push((spec, Some(tx)));
        self.spawned.notify_one();
        Ok((session, OutputStream::new(rx, Default::default())))
    }
    fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
        for (spec, tx) in self.spawns.lock().unwrap().iter_mut() {
            if spec.session_id == session.session_id {
                tx.take();
            }
        }
        Ok(())
    }
    fn send_bytes(&self, session: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()> {
        self.inputs
            .lock()
            .unwrap()
            .push((session.session_id.clone(), bytes.to_vec()));
        self.input_ready.notify_one();
        Ok(())
    }
    fn send_key(&self, _: &RuntimeSession, _: &str) -> RuntimeResult<()> {
        Ok(())
    }
    fn resize(&self, _: &RuntimeSession, _: u16, _: u16) -> RuntimeResult<()> {
        Ok(())
    }
    fn status(&self, _: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
        Ok(Some(SessionStatus {
            alive: false,
            exit_code: Some(0),
            ..Default::default()
        }))
    }
}
struct Host {
    fixture: Fixture,
    core: Arc<AppCore>,
    runtime: Arc<Runtime>,
    _root: tempfile::TempDir,
}
impl Host {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut core = crate::test_support::test_core_in(root.path().into());
        core.usage = Arc::new(crate::usage::UsageService::without_fetch());
        let runtime = Arc::new(Runtime::default());
        core.sessions = crate::session::SessionManager::new(
            core.runtime_shell_env.clone(),
            core.runtime_discovery.clone(),
            runtime.clone(),
        );
        let conn = core.db.get().unwrap();
        crate::test_support::insert_test_role(&conn, "role", "coder", "codex", "codex");
        let now = chrono::Utc::now();
        repo::crew::insert(
            &conn,
            &repo::crew::CrewRow {
                id: "crew".into(),
                name: "Test crew".into(),
                system_prompt_addendum: None,
                created_at: now,
                updated_at: now,
            },
        )
        .unwrap();
        crate::test_support::insert_test_slot(&conn, "slot", "crew", "role", "coder", 0, true);
        drop(conn);
        let core = Arc::new(core);
        let fixture = Fixture::new().await;
        assert!(fixture
            .core
            .backend
            .set(Arc::new(DaemonCore(Arc::downgrade(&core))))
            .is_ok());
        fixture.pair().await;
        Self {
            fixture,
            core,
            runtime,
            _root: root,
        }
    }
    async fn request(&self, mut value: Value) -> Result<Value> {
        value["id"] = json!(999);
        let message = c::ClientMessage::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
        self.fixture
            .client
            .request("2".into(), message)
            .await
            .map(|reply| reply.result)
            .map_err(|error| error.message)
    }
    async fn ok(&self, value: Value) -> Value {
        let kind = value["type"].as_str().unwrap_or("unknown").to_owned();
        self.request(value)
            .await
            .unwrap_or_else(|e| panic!("{kind}: {e}"))
    }
    fn path(&self) -> String {
        self._root.path().to_str().unwrap().into()
    }
    async fn start(&self) -> String {
        self.ok(
            json!({"type":"startChat","runtime":"codex","cwd":self.path(),"cols":123,"rows":45}),
        )
        .await["session"]
            .as_str()
            .unwrap()
            .into()
    }
    async fn layout(&self, tab: &str, session: Option<&str>, project: Option<&str>) {
        self.ok(json!({"type":"layout","target":{"kind":"tab","id":tab},"project":project,"name":"Chat","panes":{"id":"p1","session":session},"shells":[]})).await;
    }
    async fn wait_spawns(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let ready = self.runtime.spawned.notified();
                if self.runtime.spawns.lock().unwrap().len() >= count {
                    break;
                }
                ready.await;
            }
        })
        .await
        .unwrap();
    }
    async fn wait_live(&self, session: &str) {
        let mut events = self.core.events.subscribe();
        tokio::time::timeout(Duration::from_secs(10), async {
            while !self.core.sessions.session_live(session) {
                events.recv().await.unwrap();
            }
        })
        .await
        .unwrap();
    }
    fn latest_size(&self) -> (u16, u16) {
        let spawns = self.runtime.spawns.lock().unwrap();
        let size = spawns.last().and_then(|spawn| spawn.0.initial_size);
        drop(spawns);
        size.expect("runtime spawn has a size")
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        for (_, tx) in self.runtime.spawns.lock().unwrap().iter_mut() {
            tx.take();
        }
        let conn = self.core.db.get().unwrap();
        for mission in repo::mission::list(&conn, None).unwrap() {
            self.core.routers.unregister(&mission.id);
            self.core.buses.unmount(&mission.id);
        }
    }
}

#[tokio::test]
async fn actions_use_real_operations_over_the_relay_and_keep_host_state_authoritative() {
    let host = Host::new().await;
    let mut events = host.core.events.subscribe();
    let project = host
        .ok(json!({"type":"createProject","name":"Project","path":host.path()}))
        .await["project"]
        .as_str()
        .unwrap()
        .to_owned();
    let session = host.start().await;
    assert_eq!(host.latest_size(), (123, 45));
    let tab = ulid::Ulid::new().to_string();
    host.layout(&tab, Some(&session), None).await;
    let overview = host.fixture.client.overview("2".into()).await.unwrap();
    assert_eq!(
        overview.tabs[0].panes,
        c::Pane::Leaf {
            session: Some(session.clone())
        }
    );
    assert!(overview.sessions.contains_key(&session));
    host.ok(json!({"type":"rename","target":{"kind":"session","id":session},"name":"Named chat"}))
        .await;
    assert_eq!(
        repo::session::get_row(&host.core.db.get().unwrap(), &session)
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("Named chat")
    );
    for target in [
        json!({"kind":"tab","id":tab}),
        json!({"kind":"project","id":project}),
    ] {
        host.ok(json!({"type":"rename","target":target,"name":"Renamed"}))
            .await;
    }
    host.ok(json!({"type":"pin","target":{"kind":"tab","id":tab},"pinned":true}))
        .await;
    host.ok(json!({"type":"reorderPinned","order":[{"kind":"tab","id":tab}]}))
        .await;
    host.ok(json!({"type":"move","target":{"kind":"tab","id":tab},"project":project,"order":[]}))
        .await;
    let overview = host.fixture.client.overview("2".into()).await.unwrap();
    assert_eq!(overview.tabs[0].project.as_deref(), Some(project.as_str()));
    assert!(overview.tabs[0].pinned);
    assert_eq!(overview.tabs[0].name, "Renamed");
    let blocked = host
        .request(json!({"type":"deleteProject","project":project,"force":false}))
        .await
        .unwrap_err();
    assert!(blocked.contains("running"));
    let before = serde_json::to_value(overview).unwrap();
    assert!(host
        .request(
            json!({"type":"move","target":{"kind":"tab","id":tab},"project":"missing","order":[]})
        )
        .await
        .is_err());
    assert_eq!(
        serde_json::to_value(host.fixture.client.overview("2".into()).await.unwrap()).unwrap(),
        before
    );
    assert!(host
        .request(json!({"type":"settings","value":"private-body-canary"}))
        .await
        .is_err());
    let names: Vec<_> = std::iter::from_fn(|| events.try_recv().ok())
        .map(|event| event.name)
        .collect();
    assert!(names.contains(&"chat/layout-changed"));
    assert!(names.contains(&"project/changed"));
    host.ok(json!({"type":"stop","session":session})).await;
    host.ok(json!({"type":"resume","session":session,"cols":101,"rows":31}))
        .await;
    assert_eq!(host.latest_size(), (101, 31));
    assert!(host
        .request(json!({"type":"restart","session":session,"cols":102,"rows":32}))
        .await
        .is_err());
    let source = ulid::Ulid::new().to_string();
    let mut row = repo::session::SessionRowDb::new_running(source.clone());
    row.status = crate::model::SessionStatus::Stopped;
    row.agent_session_key = Some(uuid::Uuid::new_v4().to_string());
    row.agent_runtime = Some("claude-code".into());
    row.launch_definition = Some(crate::test_support::session_definition(
        "claude-code",
        "m8-fake-claude",
    ));
    row.cwd = Some(host.path());
    repo::session::insert(&host.core.db.get().unwrap(), &row).unwrap();
    let fork = host
        .ok(json!({"type":"fork","session":source,"title":"Fork","cols":103,"rows":33}))
        .await["session"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(fork, session);
    assert_eq!(host.latest_size(), (103, 33));
    host.ok(json!({"type":"archive","target":{"kind":"session","id":fork}}))
        .await;
    host.ok(json!({"type":"archive","target":{"kind":"session","id":session}}))
        .await;
    let empty = ulid::Ulid::new().to_string();
    host.layout(&empty, None, None).await;
    host.ok(json!({"type":"archive","target":{"kind":"tab","id":empty}}))
        .await;
    assert!(
        repo::session::get_row(&host.core.db.get().unwrap(), &session)
            .unwrap()
            .unwrap()
            .archived_at
            .is_some()
    );
    let shell = host
        .ok(json!({"type":"startShell","cwd":host.path(),"cols":90,"rows":25}))
        .await["session"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(host.latest_size(), (90, 25));
    host.ok(json!({"type":"close","session":shell})).await;
    host.ok(json!({"type":"deleteProject","project":project,"force":false}))
        .await;
    assert!(host
        .fixture
        .client
        .overview("2".into())
        .await
        .unwrap()
        .projects
        .is_empty());
}

#[tokio::test]
async fn catalog_and_folders_are_projections_without_configuration_or_file_names() {
    let host = Host::new().await;
    std::fs::create_dir(host._root.path().join("subfolder")).unwrap();
    std::fs::write(
        host._root.path().join("private-file-canary"),
        "private-body-canary",
    )
    .unwrap();
    host.core.db.get().unwrap().execute("UPDATE roles SET command='private-command-canary', env_json='{}', system_prompt='private-body-canary' WHERE id='role'",[]).unwrap();
    crate::db::app_state_set(&host.core.db.get().unwrap(),"claude_accounts",&json!({"accounts":[{"id":"named","name":"Work","folder":"private-config-path-canary","native":false}],"main_id":"named","active_id":"named"}).to_string()).unwrap();
    let catalog = host.ok(json!({"type":"catalog"})).await;
    let catalog: c::Catalog = serde_json::from_value(catalog).unwrap();
    assert_eq!(catalog.accounts[0].name, "Work");
    assert_eq!(catalog.active_account, "named");
    assert_eq!(catalog.crews[0].slots[0].handle, "coder");
    assert_eq!(catalog.roles[0].name, "coder");
    let encoded = serde_json::to_string(&catalog).unwrap();
    for canary in [
        "private-config-path-canary",
        "private-command-canary",
        "private-body-canary",
        "folder\"",
        "environment",
        "configDir",
    ] {
        assert!(!encoded.contains(canary), "{canary}");
    }
    let folders = host.ok(json!({"type":"folders","path":host.path()})).await;
    assert!(folders["folders"]
        .as_array()
        .unwrap()
        .contains(&json!("subfolder")));
    assert!(!folders.to_string().contains("private-file-canary"));
    for path in [
        host._root.path().join("private-file-canary"),
        host._root.path().join("missing"),
    ] {
        let error = host
            .request(json!({"type":"folders","path":path}))
            .await
            .unwrap_err();
        assert!(!error.contains("private-file-canary"));
    }
    let folders = host.ok(json!({"type":"folders","path":null})).await;
    assert!(folders["path"].is_string());
    assert!(host.request(json!({"type":"startChat","runtime":"codex","cwd":host._root.path().join("missing"),"cols":80,"rows":24})).await.is_err());
    assert!(host.runtime.spawns.lock().unwrap().is_empty());
}

async fn remote_event(
    events: &mut tokio::sync::broadcast::Receiver<crate::events::AppEvent>,
    name: &str,
) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.name == name {
                return event.payload;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn missions_feed_pages_follows_answers_and_posts_cross_the_real_host_adapter() {
    let host = Host::new().await;
    let mut events = host.fixture.client.0.events.subscribe();
    host.fixture
        .client
        .follow("2".into(), 7, true)
        .await
        .unwrap();
    let mission=host.ok(json!({"type":"startMission","crew":"crew","title":"Mission","goal":"Do the task","cwd":host.path(),"cols":111,"rows":41})).await["mission"].as_str().unwrap().to_owned();
    host.wait_spawns(1).await;
    assert_eq!(host.latest_size(), (111, 41));
    let member = repo::session::list_for_mission(&host.core.db.get().unwrap(), &mission).unwrap()
        [0]
    .session
    .id
    .clone();
    host.wait_live(&member).await;
    host.ok(json!({"type":"restart","session":member,"cols":113,"rows":43}))
        .await;
    assert_eq!(host.latest_size(), (113, 43));
    let push = remote_event(&mut events, "remote/overview").await;
    if push["overview"]["missions"].as_array().unwrap().is_empty() {
        let push = remote_event(&mut events, "remote/overview").await;
        assert_eq!(push["overview"]["missions"][0]["id"], mission);
    }
    let mut host_events = host.core.events.subscribe();
    for (kind, id) in [("mission", mission.as_str())] {
        host.ok(json!({"type":"rename","target":{"kind":kind,"id":id},"name":"Renamed mission"}))
            .await;
        host.ok(json!({"type":"pin","target":{"kind":kind,"id":id},"pinned":true}))
            .await;
    }
    let changed: Vec<_> = std::iter::from_fn(|| host_events.try_recv().ok())
        .filter(|event| event.name == "mission/changed")
        .collect();
    assert_eq!(changed.len(), 2);
    assert!(changed
        .iter()
        .all(|event| event.payload == json!({"mission_id":mission})));
    let project = host
        .ok(json!({"type":"createProject","name":"Missions","path":host.path()}))
        .await["project"]
        .as_str()
        .unwrap()
        .to_owned();
    host.ok(json!({"type":"move","target":{"kind":"mission","id":mission},"project":project,"order":[]})).await;
    host.ok(json!({"type":"layout","target":{"kind":"mission","id":mission},"name":"unused","panes":{"id":"m1","session":member},"shells":[]})).await;
    let node = repo::node::find_by_ref(
        &host.core.db.get().unwrap(),
        repo::node::NodeType::Mission,
        &mission,
    )
    .unwrap()
    .unwrap();
    let layout: Value = serde_json::from_str(node.layout.as_deref().unwrap()).unwrap();
    assert!(layout.get("slots").is_none());
    assert!(layout.get("tree").is_none());
    assert_eq!(layout["drawer"]["shells"], json!([]));
    let page = host
        .ok(json!({"type":"feed","mission":mission,"limit":1}))
        .await;
    assert_eq!(page["entries"].as_array().unwrap().len(), 1);
    let before = page["before"].as_str().unwrap();
    let previous = host
        .ok(json!({"type":"feed","mission":mission,"before":before,"limit":500}))
        .await;
    assert!(!previous["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["id"] == page["entries"][0]["id"]));
    assert_eq!(previous["before"], Value::Null);
    host.ok(json!({"type":"follow","mission":mission})).await;
    let dir = runner_core::event_log::mission_dir(host._root.path(), Some("crew"), &mission);
    let log = runner_core::event_log::EventLog::open(&dir).unwrap();
    let question = log
        .append(runner_core::model::EventDraft::signal(
            Some("crew"),
            &mission,
            "human",
            "human_question",
            json!({"prompt":"Proceed?","choices":["yes","no"],"on_behalf_of":"coder"}),
        ))
        .unwrap();
    host.core
        .events
        .emit("event/appended", &json!({"mission_id":mission}));
    let append = remote_event(&mut events, "remote/feed").await;
    assert_eq!(append["computer"], "2");
    assert_eq!(append["feed"]["entries"][0]["id"], question.id);
    host.ok(json!({"type":"post","mission":mission,"to":["coder"],"text":"private-body-canary"}))
        .await;
    host.ok(json!({"type":"answer","mission":mission,"question":question.id,"text":"yes"}))
        .await;
    host.core
        .events
        .emit("event/appended", &json!({"mission_id":mission}));
    let mut delivered = vec![];
    while delivered.len() < 2 {
        let append = remote_event(&mut events, "remote/feed").await;
        // The router appends its mission_warning signals on its own
        // thread, so one can land between the post and the answer.
        delivered.extend(
            append["feed"]["entries"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|entry| entry["kind"] != "signal")
                .cloned(),
        );
    }
    assert_eq!(delivered[0]["kind"], "message");
    assert_eq!(delivered[0]["to"], json!(["coder"]));
    assert_eq!(delivered[1]["kind"], "answer");
    assert!(host
        .request(json!({"type":"answer","mission":mission,"question":question.id,"text":"again"}))
        .await
        .is_err());
    let page = host
        .ok(json!({"type":"feed","mission":mission,"limit":500}))
        .await;
    let entries = page["entries"].as_array().unwrap();
    assert!(entries
        .iter()
        .any(|entry| entry["kind"] == "answer" && entry["question"] == question.id));
    assert!(
        crate::ops::mission::feed::mission_status(&host.core, &mission)
            .unwrap()
            .pending_asks
            .is_empty()
    );
    host.ok(json!({"type":"unfollow","mission":mission})).await;
    while events.try_recv().is_ok() {}
    host.ok(json!({"type":"post","mission":mission,"to":[],"text":"After unfollow"}))
        .await;
    host.core
        .events
        .emit("event/appended", &json!({"mission_id":mission}));
    host.fixture.client.overview("2".into()).await.unwrap();
    while let Ok(event) = events.try_recv() {
        assert_ne!(event.name, "remote/feed");
    }
    host.ok(json!({"type":"stopMission","mission":mission}))
        .await;
    host.ok(json!({"type":"resumeMission","mission":mission,"cols":112,"rows":42}))
        .await;
    assert_eq!(host.latest_size(), (112, 42));
    host.ok(json!({"type":"archive","target":{"kind":"mission","id":mission}}))
        .await;
    assert!(repo::mission::get(&host.core.db.get().unwrap(), &mission)
        .unwrap()
        .unwrap()
        .archived_at
        .is_some());
}

#[tokio::test]
async fn a_started_terminal_attaches_with_take_at_the_client_size_and_revocation_refuses_actions() {
    let host = Host::new().await;
    let session = host.start().await;
    let terminal = host
        .fixture
        .client
        .attach(RemoteTerminal {
            computer: "2".into(),
            session: session.clone(),
            cols: 123,
            rows: 45,
            take_control: true,
        })
        .await
        .unwrap();
    assert_eq!((terminal.snapshot.cols, terminal.snapshot.rows), (123, 45));
    assert!(matches!(
        terminal.snapshot.controller,
        TerminalController::Desktop
    ));
    let keys = host.fixture.client.keys().await.unwrap();
    let paired = keys.0.values().next().unwrap();
    let public = remote::HostKey::from_private(&paired.private.0)
        .unwrap()
        .public;
    host.fixture
        .host(HostCommand::RevokeDevice(remote::fingerprint(&public)))
        .await;
    let error = host
        .request(json!({"type":"stop","session":session}))
        .await
        .unwrap_err();
    assert!(!error.contains("private-body-canary"));
    assert!(host.core.sessions.session_live(&session));
}

#[tokio::test]
async fn generic_requests_cannot_bypass_terminal_input_route_guards() {
    let fixture = Fixture::new().await;
    let request = c::ClientMessage::Input(c::InputRequest {
        id: 1,
        session: "session".into(),
        data: c::TerminalBytes(b"private-terminal-canary".to_vec()),
    });
    assert!(fixture.client.request("2".into(), request).await.is_err());
    assert_eq!(fixture.relay.opens.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn a_followed_feed_keeps_the_shared_route_alive_without_an_overview_follower() {
    let host = Host::new().await;
    let out = crate::ops::mission::start(
        &mut host.core.db.get().unwrap(),
        host._root.path(),
        runner_core::protocol::MissionStart {
            crew_id: Some("crew".into()),
            scope: runner_core::protocol::ProjectScope::Root,
            title: "Feed".into(),
            goal_override: None,
            cwd: Some(host.path()),
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();
    host.ok(json!({"type":"follow","mission":out.mission.id}))
        .await;
    tokio::time::pause();
    super::keep_alive(&host.fixture, 65).await;
    tokio::time::resume();
    host.fixture.client.overview("2".into()).await.unwrap();
    assert_eq!(host.fixture.relay.opens.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn role_starts_preserve_the_local_scope_and_working_folder_fallback() {
    let host = Host::new().await;
    let project = host
        .ok(json!({"type":"createProject","name":"Project","path":host.path()}))
        .await["project"]
        .as_str()
        .unwrap()
        .to_owned();
    host.core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE roles SET working_dir=?1 WHERE id='role'",
            [host.path()],
        )
        .unwrap();
    let session = host
        .ok(json!({"type":"startChat","role":"role","cols":121,"rows":35}))
        .await["session"]
        .as_str()
        .unwrap()
        .to_owned();
    let row = repo::session::get_row(&host.core.db.get().unwrap(), &session)
        .unwrap()
        .unwrap();
    assert_eq!(row.cwd.as_deref(), Some(host.path().as_str()));
    assert!(row.project_id.is_none());
    assert_eq!(host.latest_size(), (121, 35));
    host.layout(
        &ulid::Ulid::new().to_string(),
        Some(&session),
        Some(&project),
    )
    .await;
    let result = host
        .ok(json!({"type":"deleteProject","project":project,"force":true}))
        .await;
    assert_eq!(result["archivedSessions"], json!([session]));
    assert!(
        repo::session::get_row(&host.core.db.get().unwrap(), &session)
            .unwrap()
            .unwrap()
            .archived_at
            .is_some()
    );
}

#[tokio::test]
async fn action_results_carry_the_negotiated_features_for_frontend_gating() {
    let host = Host::new().await;
    let reply = host
        .fixture
        .client
        .request(
            "2".into(),
            c::ClientMessage::Overview(c::OverviewRequest { id: 999 }),
        )
        .await
        .unwrap();
    assert!(reply.result["tabs"].is_array());
    for name in c::ActionRequest::NAMES {
        assert!(reply.features.iter().any(|feature| feature == name));
    }
}

#[tokio::test]
async fn viewing_a_crashed_remote_chat_clears_attention_without_starting_it() {
    let host = Host::new().await;
    let mut row = repo::session::SessionRowDb::new_running("crashed".into());
    row.status = crate::model::SessionStatus::Crashed;
    row.agent_runtime = Some("codex".into());
    row.agent_command = Some("codex".into());
    row.agent_session_key = Some("conversation".into());
    row.stopped_at = Some(chrono::Utc::now() - chrono::Duration::seconds(1));
    {
        let conn = host.core.db.get().unwrap();
        repo::session::insert(&conn, &row).unwrap();
        repo::session_attention::record_completion(&conn, &row.id, false, 100).unwrap();
    }
    host.layout(&ulid::Ulid::new().to_string(), Some(&row.id), None)
        .await;
    let mut events = host.fixture.client.0.events.subscribe();
    host.fixture
        .client
        .follow("2".into(), 7, true)
        .await
        .unwrap();
    let before = remote_event(&mut events, "remote/overview").await;
    assert_eq!(before["overview"]["sessions"][&row.id]["status"], "error");
    assert_eq!(before["overview"]["sessions"][&row.id]["unread"], true);

    host.ok(json!({"type":"markViewed","session":row.id})).await;
    let push = remote_event(&mut events, "remote/session-status").await;
    assert_eq!(push["status"]["session"], row.id);
    assert_eq!(push["status"]["status"], "error");
    assert_eq!(push["status"]["errorAcknowledged"], true);
    assert_eq!(push["status"]["unread"], false);
    let after = host.ok(json!({"type":"overview"})).await;
    assert_eq!(after["sessions"][&row.id]["errorAcknowledged"], true);
    let saved = repo::session::get_row(&host.core.db.get().unwrap(), &row.id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.status, row.status);
    assert_eq!(saved.agent_session_key, row.agent_session_key);
    assert_eq!(saved.stopped_at, row.stopped_at);
    assert!(!host.core.sessions.session_live(&row.id));
    assert!(host.runtime.spawns.lock().unwrap().is_empty());
}

#[tokio::test]
async fn remote_layouts_reject_chat_drawers_and_preserve_mission_shell_ownership() {
    let host = Host::new().await;
    let session = host.start().await;
    let mut shells = vec![];
    for _ in 0..2 {
        shells.push(
            host.ok(json!({"type":"startShell","cwd":host.path(),"cols":80,"rows":24}))
                .await["session"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    let tab = ulid::Ulid::new().to_string();
    host.layout(&tab, Some(&session), None).await;
    assert!(host.request(json!({"type":"layout","target":{"kind":"tab","id":tab},"name":"Split","panes":{"id":"p1","session":session},"shells":shells})).await.is_err());
    host.ok(json!({"type":"layout","target":{"kind":"tab","id":tab},"name":"Split","panes":{"id":"p1","session":session},"shells":[]})).await;
    let layout: Value = serde_json::from_str(
        repo::node::get(&host.core.db.get().unwrap(), &tab)
            .unwrap()
            .unwrap()
            .layout
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    assert!(layout.get("drawer").is_none());
    shells.reverse();
    let drawer =
        json!({"shells":shells,"open":true,"height":456.0,"active":1,"futurePresentation":true});
    let mission=host.ok(json!({"type":"startMission","crew":"crew","title":"Drawer","cwd":host.path(),"cols":80,"rows":24})).await["mission"].as_str().unwrap().to_owned();
    let member = repo::session::list_for_mission(&host.core.db.get().unwrap(), &mission).unwrap()
        [0]
    .session
    .id
    .clone();
    host.wait_live(&member).await;
    let node = repo::node::find_by_ref(
        &host.core.db.get().unwrap(),
        repo::node::NodeType::Mission,
        &mission,
    )
    .unwrap()
    .unwrap();
    crate::ops::node::node_mission_layout_set(
        &host.core,
        &node.id,
        json!({"drawer":drawer}).to_string(),
    )
    .unwrap();
    host.ok(json!({"type":"layout","target":{"kind":"mission","id":mission},"name":"unused","panes":{"id":"member","session":member},"shells":[shells[1],shells[0]]})).await;
    let saved: Value = serde_json::from_str(
        repo::node::get(&host.core.db.get().unwrap(), &node.id)
            .unwrap()
            .unwrap()
            .layout
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    let mut expected = drawer;
    expected["shells"] = json!([shells[1], shells[0]]);
    expected["active"] = json!(0);
    assert_eq!(saved, json!({"drawer":expected}));
    assert!(host.request(json!({"type":"layout","target":{"kind":"mission","id":mission},"name":"unused","panes":{"id":"member","session":member},"shells":[member]})).await.is_err());
    host.ok(json!({"type":"archive","target":{"kind":"mission","id":mission}}))
        .await;
    for shell in shells {
        assert!(repo::session::get_row(&host.core.db.get().unwrap(), &shell)
            .unwrap()
            .is_none());
    }
}

fn feed_mission(host: &Host) -> (String, runner_core::event_log::EventLog) {
    let out = crate::ops::mission::start(
        &mut host.core.db.get().unwrap(),
        host._root.path(),
        runner_core::protocol::MissionStart {
            crew_id: Some("crew".into()),
            scope: runner_core::protocol::ProjectScope::Root,
            title: "Feed fixture".into(),
            goal_override: None,
            cwd: Some(host.path()),
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();
    let mission = out.mission.id;
    let path = runner_core::event_log::mission_dir(host._root.path(), Some("crew"), &mission);
    (
        mission,
        runner_core::event_log::EventLog::open(&path).unwrap(),
    )
}

#[tokio::test]
async fn feed_filters_noise_before_paging_and_advances_past_noise_only_tails() {
    let host = Host::new().await;
    let (mission, log) = feed_mission(&host);
    let mut ids = vec![];
    for i in 0..2 {
        ids.push(
            log.append(runner_core::model::EventDraft::message(
                Some("crew"),
                &mission,
                "human",
                None,
                format!("Visible {i}"),
            ))
            .unwrap()
            .id,
        );
        for signal in ["session_status", "runner_status", "inbox_read", "ask_human"] {
            log.append(runner_core::model::EventDraft::signal(
                Some("crew"),
                &mission,
                "coder",
                signal,
                json!({"prompt":"hidden-canary","state":"busy"}),
            ))
            .unwrap();
        }
    }
    let newest = host
        .ok(json!({"type":"feed","mission":mission,"limit":1}))
        .await;
    assert_eq!(newest["entries"][0]["id"], ids[1]);
    let older = host
        .ok(json!({"type":"feed","mission":mission,"limit":1,"before":newest["before"]}))
        .await;
    assert_eq!(older["entries"][0]["id"], ids[0]);
    let page = host
        .ok(json!({"type":"feed","mission":mission,"limit":2}))
        .await;
    assert_eq!(page["entries"].as_array().unwrap().len(), 2);
    assert!(!page.to_string().contains("hidden-canary"));
    let backend = DaemonCore(Arc::downgrade(&host.core));
    let cursor = backend.feed_cursor(&mission).unwrap();
    let mut events = host.fixture.client.0.events.subscribe();
    host.ok(json!({"type":"follow","mission":mission})).await;
    for signal in ["session_status", "runner_status", "inbox_read", "ask_human"] {
        log.append(runner_core::model::EventDraft::signal(
            Some("crew"),
            &mission,
            "coder",
            signal,
            json!({"prompt":"hidden-canary"}),
        ))
        .unwrap();
    }
    let updates = backend.feed_since(&mission, cursor).unwrap();
    assert!(updates.entries.is_empty());
    assert!(updates.cursor > cursor);
    assert!(!updates.more);
    host.core
        .events
        .emit("event/appended", &json!({"mission_id":mission}));
    host.fixture.client.overview("2".into()).await.unwrap();
    let visible = log
        .append(runner_core::model::EventDraft::message(
            Some("crew"),
            &mission,
            "human",
            None,
            "After noise",
        ))
        .unwrap();
    host.core
        .events
        .emit("event/appended", &json!({"mission_id":mission}));
    let append = remote_event(&mut events, "remote/feed").await;
    assert_eq!(append["feed"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(append["feed"]["entries"][0]["id"], visible.id);
}

#[tokio::test]
async fn a_single_append_drains_later_feeds_after_idle_noise_and_deleted_subscriptions() {
    let host = Host::new().await;
    let mut missions: Vec<_> = (0..16).map(|_| feed_mission(&host)).collect();
    missions.sort_by(|a, b| a.0.cmp(&b.0));
    for (mission, _) in &missions {
        host.ok(json!({"type":"follow","mission":mission})).await;
    }
    repo::mission::delete_many(
        &host.core.db.get().unwrap(),
        std::slice::from_ref(&missions[0].0),
    )
    .unwrap();
    missions[1]
        .1
        .append(runner_core::model::EventDraft::signal(
            Some("crew"),
            &missions[1].0,
            "coder",
            "session_status",
            json!({"state":"busy"}),
        ))
        .unwrap();
    let (mission, log) = missions.last().unwrap();
    let visible = log
        .append(runner_core::model::EventDraft::message(
            Some("crew"),
            mission,
            "human",
            None,
            "Only the final feed has visible work",
        ))
        .unwrap();
    let mut events = host.fixture.client.0.events.subscribe();
    host.core
        .events
        .emit("event/appended", &json!({"mission_id":mission}));
    let append = remote_event(&mut events, "remote/feed").await;
    assert_eq!(append["feed"]["mission"], *mission);
    assert_eq!(append["feed"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(append["feed"]["entries"][0]["id"], visible.id);
    assert_eq!(host.fixture.relay.opens.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn large_feed_pages_and_incremental_pushes_keep_the_shared_terminal_route_usable() {
    let host = Host::new().await;
    let (mission, log) = feed_mission(&host);
    let shell = host
        .ok(json!({"type":"startShell","cwd":host.path(),"cols":80,"rows":24}))
        .await["session"]
        .as_str()
        .unwrap()
        .to_owned();
    host.ok(json!({"type":"layout","target":{"kind":"tab","id":ulid::Ulid::new().to_string()},"name":"Terminal","panes":{"id":"p1","session":shell},"shells":[]})).await;
    let mut terminal = host
        .fixture
        .client
        .attach(RemoteTerminal {
            computer: "2".into(),
            session: shell.clone(),
            cols: 80,
            rows: 24,
            take_control: true,
        })
        .await
        .unwrap();
    let mut events = host.fixture.client.0.events.subscribe();
    host.ok(json!({"type":"follow","mission":mission})).await;
    let mut adapter = crate::companion::Adapter::new(
        Arc::new(DaemonCore(Arc::downgrade(&host.core))),
        42,
        "budget probe".into(),
    )
    .unwrap();
    adapter
        .request(c::ClientMessage::Follow(c::MissionRequest {
            id: 1,
            mission: mission.clone(),
        }))
        .await
        .unwrap();
    loop {
        if let Some(bytes) = adapter.next(true).await.unwrap() {
            let c::HostMessage::Result(reply) = serde_json::from_slice(&bytes).unwrap() else {
                panic!("follow result expected")
            };
            assert_eq!(reply.id, 1);
            break;
        }
        adapter.changed().await.unwrap();
    }
    let mut ids = vec![];
    for i in 0..512 {
        let text = format!("packet-{i}: {}", "x".repeat(3072));
        ids.push(
            log.append(runner_core::model::EventDraft::message(
                Some("crew"),
                &mission,
                "human",
                None,
                text,
            ))
            .unwrap()
            .id,
        );
    }
    let huge = log
        .append(runner_core::model::EventDraft::message(
            Some("crew"),
            &mission,
            "human",
            None,
            "private-oversize-canary".repeat(60_000),
        ))
        .unwrap();
    ids.push(huge.id.clone());
    ids.push(
        log.append(runner_core::model::EventDraft::message(
            Some("crew"),
            &mission,
            "human",
            None,
            "After oversized entry",
        ))
        .unwrap()
        .id,
    );
    host.core
        .events
        .emit("event/appended", &json!({"mission_id":mission}));
    adapter.changed().await.unwrap();
    adapter.in_flight(c::MAX_MESSAGE_SIZE - 10);
    assert!(adapter.next(true).await.unwrap().is_none());
    adapter.in_flight(0);
    let mut probed = vec![];
    while probed.len() < ids.len() {
        let Some(bytes) = adapter.next(true).await.unwrap() else {
            adapter.changed().await.unwrap();
            continue;
        };
        let c::HostMessage::FeedAppended(feed) = serde_json::from_slice(&bytes).unwrap() else {
            continue;
        };
        assert!(bytes.len() <= crate::companion::feed::MAX_BATCH_BYTES);
        probed.extend(feed.entries.into_iter().map(|entry| entry.id));
    }
    assert_eq!(probed, ids);
    let mut delivered = vec![];
    let mut batches = 0;
    while delivered.len() < ids.len() {
        let append = remote_event(&mut events, "remote/feed").await;
        assert!(
            serde_json::to_vec(&append).unwrap().len() <= crate::companion::feed::MAX_BATCH_BYTES
        );
        assert!(!append.to_string().contains("private-oversize-canary"));
        for entry in append["feed"]["entries"].as_array().unwrap() {
            if entry["id"] == huge.id {
                assert_eq!(entry["signal"], "feedEntryTooLarge");
            }
            delivered.push(entry["id"].as_str().unwrap().to_owned());
        }
        batches += 1;
    }
    assert!(batches > 1);
    assert_eq!(delivered, ids);
    let mut before = Value::Null;
    let mut paged = vec![];
    loop {
        let page = host
            .ok(json!({"type":"feed","mission":mission,"before":before,"limit":500}))
            .await;
        assert!(
            serde_json::to_vec(&page).unwrap().len() <= crate::companion::feed::MAX_BATCH_BYTES
        );
        assert!(!page.to_string().contains("private-oversize-canary"));
        paged.extend(
            page["entries"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|entry| {
                    let id = entry["id"].as_str().unwrap();
                    ids.iter()
                        .any(|expected| expected == id)
                        .then(|| id.to_owned())
                }),
        );
        before = page["before"].clone();
        if before.is_null() {
            break;
        }
    }
    let mut newest_first = ids;
    newest_first.reverse();
    assert_eq!(paged, newest_first);
    host.fixture
        .client
        .viewport("2".into(), shell.clone(), 91, 31, true)
        .await
        .unwrap();
    host.fixture
        .client
        .input("2".into(), shell.clone(), b"still usable".to_vec())
        .await
        .unwrap();
    let tx = host
        .runtime
        .spawns
        .lock()
        .unwrap()
        .iter()
        .find(|(spec, _)| spec.session_id == shell)
        .unwrap()
        .1
        .clone()
        .unwrap();
    tx.send(RuntimeOutput::Stream(b"terminal after backlog".to_vec()))
        .unwrap();
    let output = tokio::task::spawn_blocking(move || loop {
        if let TerminalFrame::Output { bytes, .. } = terminal.frames.recv().unwrap() {
            if bytes == b"terminal after backlog" {
                break bytes;
            }
        }
    });
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(10), output)
            .await
            .unwrap()
            .unwrap(),
        b"terminal after backlog"
    );
    host.fixture.client.overview("2".into()).await.unwrap();
    assert_eq!(host.fixture.relay.opens.load(Ordering::Relaxed), 1);
}

fn role_request(kind: &str) -> Value {
    json!({"type":kind,"role":"role","handle":"builder","name":"Builder","runtime":"codex","command":null,"args":["--profile","work"],"workingDir":"/tmp","systemPrompt":"Build carefully.","model":"model","effort":"high","speed":"fast","permissionMode":"auto"})
}

fn assert_change_events(
    events: &mut tokio::sync::broadcast::Receiver<crate::events::AppEvent>,
    expected: &[&str],
) {
    let mut actual = Vec::new();
    while let Ok(event) = events.try_recv() {
        if matches!(event.name, "role/changed" | "crew/changed" | "slot/changed") {
            assert_eq!(event.payload, Value::Null);
            actual.push(event.name);
        }
    }
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn role_and_crew_actions_round_trip_through_the_host_operations_and_emit_events() {
    let host = Host::new().await;
    let mut events = host.core.events.subscribe();
    let created = host.ok(role_request("createRole")).await;
    let role = created["role"].as_str().unwrap();
    assert_change_events(&mut events, &["role/changed"]);
    host.core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE roles SET env_json = ?1 WHERE id = ?2",
            rusqlite::params![r#"{"API_KEY":"private-env-canary"}"#, role],
        )
        .unwrap();
    let read = host.ok(json!({"type":"role","role":role})).await;
    assert_eq!(
        read,
        json!({"id":role,"handle":"builder","name":"Builder","runtime":"codex","command":"codex","args":["--profile","work","--ask-for-approval","on-request","--sandbox","workspace-write"],"workingDir":"/tmp","systemPrompt":"Build carefully.","model":"model","effort":"high","speed":"fast","permissionMode":"auto"})
    );
    assert!(!read.to_string().contains("env"));
    let mut update = role_request("updateRole");
    update["role"] = json!(role);
    update["command"] = json!("custom-codex");
    update["handle"] = json!("cannot-rename-role");
    update["env"] = json!({"API_KEY":"must-not-store"});
    update["permissionMode"] = Value::Null;
    for field in ["workingDir", "systemPrompt", "model", "effort", "speed"] {
        update[field] = Value::Null;
    }
    assert_eq!(host.ok(update).await, Value::Null);
    assert_change_events(&mut events, &["role/changed"]);
    let read = host.ok(json!({"type":"role","role":role})).await;
    assert_eq!(read["args"], json!(["--profile", "work"]));
    assert_eq!(read["command"], "custom-codex");
    assert_eq!(read["handle"], "builder");
    assert_eq!(read["permissionMode"], "default");
    for field in ["workingDir", "systemPrompt", "model", "effort", "speed"] {
        assert!(read[field].is_null());
    }
    let stored = repo::role::get(&host.core.db.get().unwrap(), role)
        .unwrap()
        .unwrap();
    assert_eq!(stored.env["API_KEY"], "private-env-canary");
    let crew = host
        .ok(json!({"type":"createCrew","name":"Pair","conventions":"Review everything."}))
        .await["crew"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_change_events(&mut events, &["crew/changed"]);
    let first = host.ok(json!({"type":"addSlot","crew":crew,"role":role,"handle":"coder","runtime":null,"model":null})).await["slot"].as_str().unwrap().to_owned();
    assert_change_events(&mut events, &["slot/changed"]);
    let second = host.ok(json!({"type":"addSlot","crew":crew,"role":role,"handle":"reviewer","runtime":"codex","model":"slot-model"})).await["slot"].as_str().unwrap().to_owned();
    assert_change_events(&mut events, &["slot/changed"]);
    assert_eq!(host.ok(json!({"type":"updateSlot","slot":second,"handle":"critic","runtime":"codex","model":"slot-model","effort":"low","speed":"standard"})).await, Value::Null);
    assert_change_events(&mut events, &["slot/changed"]);
    host.core
        .db
        .get()
        .unwrap()
        .execute("UPDATE slots SET lead = 1 WHERE crew_id = ?1", [&crew])
        .unwrap();
    let read = host.ok(json!({"type":"crew","crew":crew})).await;
    assert_eq!(
        read,
        json!({"id":crew,"name":"Pair","conventions":"Review everything.","slots":[{"id":first,"handle":"coder","role":role,"lead":false,"runtime":null,"model":null,"effort":null,"speed":null},{"id":second,"handle":"critic","role":role,"lead":false,"runtime":"codex","model":"slot-model","effort":"low","speed":"standard"}]})
    );
    assert!(!read.to_string().contains("env"));
    assert_eq!(
        host.ok(json!({"type":"reorderSlots","crew":crew,"order":[second,first]}))
            .await,
        Value::Null
    );
    assert_change_events(&mut events, &["slot/changed"]);
    let read = host.ok(json!({"type":"crew","crew":crew})).await;
    assert_eq!(read["slots"][0]["id"], second);
    assert_eq!(read["slots"][0]["lead"], false);
    assert_eq!(read["slots"][1]["lead"], false);
    assert_eq!(host.ok(json!({"type":"updateSlot","slot":second,"handle":"critic","runtime":null,"model":null,"effort":null,"speed":null})).await, Value::Null);
    assert_change_events(&mut events, &["slot/changed"]);
    let read = host.ok(json!({"type":"crew","crew":crew})).await;
    for field in ["runtime", "model", "effort", "speed"] {
        assert!(read["slots"][0][field].is_null());
    }
    assert_eq!(
        host.ok(json!({"type":"removeSlot","slot":second})).await,
        Value::Null
    );
    assert_change_events(&mut events, &["slot/changed"]);
    let read = host.ok(json!({"type":"crew","crew":crew})).await;
    assert_eq!(read["slots"].as_array().unwrap().len(), 1);
    assert_eq!(read["slots"][0]["lead"], false);
    assert_eq!(
        host.ok(json!({"type":"updateCrew","crew":crew,"name":"Updated","conventions":null}))
            .await,
        Value::Null
    );
    assert_change_events(&mut events, &["crew/changed"]);
    let read = host.ok(json!({"type":"crew","crew":crew})).await;
    assert_eq!(read["name"], "Updated");
    assert!(read["conventions"].is_null());
    assert_eq!(
        host.ok(json!({"type":"deleteCrew","crew":crew})).await,
        Value::Null
    );
    assert_change_events(&mut events, &["crew/changed", "slot/changed"]);
    assert!(repo::slot::get(&host.core.db.get().unwrap(), &first)
        .unwrap()
        .is_none());
    assert_eq!(
        host.ok(json!({"type":"deleteRole","role":role})).await,
        Value::Null
    );
    assert_change_events(&mut events, &["role/changed"]);
    assert!(repo::role::get(&host.core.db.get().unwrap(), role)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn role_and_crew_actions_reject_invalid_fields_and_unknown_ids_without_leaking_diagnostics() {
    use crate::companion::core::Core;
    let host = Host::new().await;
    let backend = DaemonCore(Arc::downgrade(&host.core));
    let mut events = host.core.events.subscribe();
    let reject = |mut value: Value, code: c::ErrorCode, message: &str| {
        value["id"] = json!(7);
        let request = serde_json::from_value(value).unwrap();
        let error = Core::action(&backend, request).unwrap_err();
        assert_eq!(error.0, code);
        assert_eq!(error.1, message);
        assert!(!error.1.contains("private-canary"));
    };
    for (kind, field) in [
        ("role", "role"),
        ("crew", "crew"),
        ("updateRole", "role"),
        ("deleteRole", "role"),
        ("updateCrew", "crew"),
        ("deleteCrew", "crew"),
        ("addSlot", "crew"),
        ("updateSlot", "slot"),
        ("removeSlot", "slot"),
        ("reorderSlots", "crew"),
    ] {
        let mut value = match kind {
            "updateRole" => role_request(kind),
            "updateCrew" => json!({"type":kind,"name":"Crew"}),
            "addSlot" => json!({"type":kind,"role":"role","handle":"new"}),
            "updateSlot" => json!({"type":kind,"handle":"new"}),
            "reorderSlots" => json!({"type":kind,"order":[]}),
            _ => json!({"type":kind}),
        };
        value[field] = json!("missing-private-canary");
        let message = match field {
            "role" => "No role has that id.",
            "crew" => "No crew has that id.",
            _ => "No slot has that id.",
        };
        reject(value, c::ErrorCode::NotFound, message);
    }
    reject(
        json!({"type":"addSlot","crew":"crew","role":"missing-private-canary","handle":"new"}),
        c::ErrorCode::NotFound,
        "No role has that id.",
    );
    for kind in ["createRole", "updateRole"] {
        for (field, invalid) in [
            ("name", json!("  ")),
            ("runtime", json!("unknown-private-canary")),
            ("runtime", json!("shell")),
            ("speed", json!("unknown-private-canary")),
            ("permissionMode", json!("unknown-private-canary")),
        ] {
            let mut value = role_request(kind);
            value[field] = invalid;
            reject(
                value,
                c::ErrorCode::BadRequest,
                "The action has invalid fields.",
            );
        }
    }
    for handle in [
        "",
        "UPPER",
        "has space",
        "-prefix",
        "abcdefghijklmnopqrstuvwxyz1234567",
    ] {
        let mut value = role_request("createRole");
        value["handle"] = json!(handle);
        reject(
            value,
            c::ErrorCode::BadRequest,
            "The action has invalid fields.",
        );
        for kind in ["addSlot", "updateSlot"] {
            reject(
                json!({"type":kind,"crew":"crew","role":"role","slot":"slot","handle":handle}),
                c::ErrorCode::BadRequest,
                "The action has invalid fields.",
            );
        }
    }
    for kind in ["createCrew", "updateCrew"] {
        reject(
            json!({"type":kind,"crew":"crew","name":"  "}),
            c::ErrorCode::BadRequest,
            "The action has invalid fields.",
        );
    }
    for kind in ["addSlot", "updateSlot"] {
        for runtime in ["shell", "unknown-private-canary"] {
            reject(
                json!({"type":kind,"crew":"crew","role":"role","slot":"slot","handle":"valid","runtime":runtime}),
                c::ErrorCode::BadRequest,
                "The action has invalid fields.",
            );
        }
    }
    reject(
        json!({"type":"updateSlot","slot":"slot","handle":"valid","speed":"unknown-private-canary"}),
        c::ErrorCode::BadRequest,
        "The action has invalid fields.",
    );
    for order in [
        json!([]),
        json!(["slot", "slot"]),
        json!(["missing-private-canary"]),
    ] {
        reject(
            json!({"type":"reorderSlots","crew":"crew","order":order}),
            c::ErrorCode::BadRequest,
            "The action has invalid fields.",
        );
    }
    let mut value = role_request("createRole");
    value["handle"] = json!("coder");
    reject(
        value,
        c::ErrorCode::Failed,
        "That role handle is already in use. Choose another handle in the catalog.",
    );
    reject(
        json!({"type":"addSlot","crew":"crew","role":"role","handle":"coder"}),
        c::ErrorCode::Failed,
        "That slot handle is already in use. Choose another handle in the crew.",
    );
    let other = host
        .ok(json!({"type":"addSlot","crew":"crew","role":"role","handle":"other"}))
        .await["slot"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_change_events(&mut events, &["slot/changed"]);
    reject(
        json!({"type":"updateSlot","slot":other,"handle":"coder","runtime":"claude-code"}),
        c::ErrorCode::Failed,
        "That slot handle is already in use. Choose another handle in the crew.",
    );
    let stored = repo::slot::get(&host.core.db.get().unwrap(), &other)
        .unwrap()
        .unwrap();
    assert_eq!(stored.slot_handle, "other");
    assert_eq!(stored.runtime_override, None);
    reject(json!({"type":"deleteRole","role":"role"}), c::ErrorCode::Failed, "This role is used by crews. Remove its slots from the crews in the catalog before deleting it.");
    // An operation's other diagnostics, including supplied prompts, stay generic.
    let mut value = role_request("createRole");
    value["systemPrompt"] = json!("private-canary".repeat(2000));
    reject(
        value,
        c::ErrorCode::Failed,
        "The host could not complete the request.",
    );
    assert_change_events(&mut events, &[]);
    host.ok(json!({"type":"deleteCrew","crew":"crew"})).await;
    assert_change_events(&mut events, &["crew/changed", "slot/changed"]);
    {
        let conn = host.core.db.get().unwrap();
        let mut chat = repo::session::SessionRowDb::new_running("chat-private-canary".into());
        chat.role_id = Some("role".into());
        repo::session::insert(&conn, &chat).unwrap();
    }
    reject(
        json!({"type":"deleteRole","role":"role"}),
        c::ErrorCode::Failed,
        "This role has unarchived chats. Archive them on the host before deleting it.",
    );
    {
        let conn = host.core.db.get().unwrap();
        conn.execute(
            "UPDATE sessions SET archived_at = ?1 WHERE id = 'chat-private-canary'",
            [chrono::Utc::now().to_rfc3339()],
        )
        .unwrap();
        let now = chrono::Utc::now();
        repo::mission::insert(
            &conn,
            &repo::mission::MissionRow {
                id: "mission-private-canary".into(),
                crew_id: None,
                project_id: None,
                title: "private-canary".into(),
                status: crate::model::MissionStatus::Running,
                goal_override: None,
                cwd: None,
                started_at: now,
                stopped_at: None,
                pinned_at: None,
                archived_at: None,
                linked_session_id: None,
                report_to_chat: true,
            },
        )
        .unwrap();
        let mut member = repo::session::SessionRowDb::new_running("member-private-canary".into());
        member.role_id = Some("role".into());
        member.mission_id = Some("mission-private-canary".into());
        repo::session::insert(&conn, &member).unwrap();
    }
    reject(
        json!({"type":"deleteRole","role":"role"}),
        c::ErrorCode::Failed,
        "This role is used by unarchived missions. Archive them on the host before deleting it.",
    );
    assert_change_events(&mut events, &[]);
}

#[tokio::test]
async fn role_permission_modes_match_the_desktop_inference_and_preserve_sent_args_on_null_update() {
    let host = Host::new().await;
    for (runtime, args, mode) in [
        (
            "claude-code",
            json!(["--permission-mode", "acceptEdits"]),
            json!("acceptEdits"),
        ),
        (
            "codex",
            json!([
                "--ask-for-approval",
                "never",
                "--sandbox",
                "workspace-write"
            ]),
            json!("bypass"),
        ),
        (
            "trae",
            json!(["--permission-mode", "auto"]),
            json!("default"),
        ),
        ("pi", json!([]), Value::Null),
    ] {
        let mut update = role_request("updateRole");
        update["runtime"] = json!(runtime);
        update["args"] = args.clone();
        update["permissionMode"] = Value::Null;
        host.ok(update).await;
        let read = host.ok(json!({"type":"role","role":"role"})).await;
        assert_eq!(read["permissionMode"], mode, "{runtime}");
        assert_eq!(read["args"], args);
        assert_eq!(
            read["command"],
            crate::model::Runtime::parse(runtime)
                .unwrap()
                .command()
                .unwrap()
        );
    }
    let mut update = role_request("updateRole");
    update["permissionMode"] = json!("bypass");
    host.ok(update).await;
    assert_eq!(
        host.ok(json!({"type":"role","role":"role"})).await["permissionMode"],
        "bypass"
    );
    let mut create = role_request("createRole");
    create["permissionMode"] = Value::Null;
    create["args"] = json!([
        "--ask-for-approval",
        "never",
        "--sandbox",
        "workspace-write"
    ]);
    let id = host.ok(create).await["role"].as_str().unwrap().to_owned();
    let read = host.ok(json!({"type":"role","role":id})).await;
    assert_eq!(read["permissionMode"], "default");
    assert_eq!(read["args"], json!([]));
}

#[tokio::test]
async fn usage_is_forwarded_with_the_negotiated_features() {
    let host = Host::new().await;
    let result = host
        .fixture
        .client
        .request(
            "2".into(),
            serde_json::from_value(json!({"type":"usage","id":1,"popover":true})).unwrap(),
        )
        .await
        .unwrap();
    assert!(result.features.iter().any(|feature| feature == "usage"));
    let usage: c::Usage = serde_json::from_value(result.result).unwrap();
    assert!(usage.agents.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn remote_launch_refusal_keeps_the_typed_runtime_version_gate() {
    use std::os::unix::fs::PermissionsExt;
    let host = Host::new().await;
    let executable = host._root.path().join("codex-fixture");
    std::fs::write(&executable, "#!/bin/sh\necho 'codex-cli 0.147.0'\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    crate::db::set_runtime_override(&host.core.db, "codex", executable.to_str()).unwrap();
    host.core
        .runtime_discovery
        .write()
        .unwrap()
        .versions
        .set_installed(
            runner_core::protocol::Runtime::Codex,
            executable.to_str().unwrap(),
            Some("0.147.0"),
        );
    let request = c::ClientMessage::decode(&serde_json::to_vec(&json!({"type":"startChat","id":1,"runtime":"codex","cwd":host.path(),"cols":80,"rows":24})).unwrap()).unwrap();
    let error = host
        .fixture
        .client
        .request("2".into(), request)
        .await
        .unwrap_err();
    let gate = error
        .version_gate
        .expect("remote refusal lost its typed recovery details");
    assert_eq!(gate.runtime, runner_core::protocol::Runtime::Codex);
    assert_eq!(gate.installed_version, "0.147.0");
    assert_eq!(gate.min_version, "0.149.0");
    assert!(error.message.contains("Settings → Agents"));

    let mission = {
        let mut conn = host.core.db.get().unwrap();
        let mission = crate::ops::mission::start(
            &mut conn,
            &host.core.app_data_dir,
            runner_core::protocol::StartMissionInput {
                crew_id: Some("crew".into()),
                project_id: None,
                title: "Old Codex mission".into(),
                goal_override: None,
                cwd: Some(host.path()),
                started_by_session_id: None,
                no_report: false,
            },
        )
        .unwrap()
        .mission;
        let role = crate::ops::role::get(&conn, "role").unwrap();
        let mut row = crate::test_support::test_session_row(
            "old-codex",
            crate::model::SessionStatus::Stopped,
        );
        row.mission_id = Some(mission.id.clone());
        row.role_id = Some("role".into());
        row.slot_id = Some("slot".into());
        row.agent_runtime = Some("codex".into());
        row.agent_command = Some("codex".into());
        row.launch_definition = Some(crate::session::launch_definition::from_effective(&role));
        crate::test_support::insert_session_row(&conn, &row).unwrap();
        mission
    };
    let local = crate::daemon::InProcessTransport::client(host.core.as_ref().clone());
    assert_eq!(
        local
            .mission_resume(&mission.id)
            .unwrap_err()
            .version_gate
            .unwrap()
            .installed_version,
        "0.147.0"
    );
    let request = c::ClientMessage::decode(
        &serde_json::to_vec(
            &json!({"type":"resumeMission","id":2,"mission":mission.id,"cols":80,"rows":24}),
        )
        .unwrap(),
    )
    .unwrap();
    let error = host
        .fixture
        .client
        .request("2".into(), request)
        .await
        .unwrap_err();
    assert_eq!(error.version_gate.unwrap().min_version, "0.149.0");
}

#[tokio::test]
async fn remote_mission_link_and_unlink_apply_host_rules_and_update_the_overview() {
    let host = Host::new().await;
    let mut events = host.core.events.subscribe();
    let project = host
        .ok(json!({"type":"createProject","name":"Other","path":host.path()}))
        .await["project"]
        .as_str()
        .unwrap()
        .to_owned();
    let chat = host.start().await;
    host.wait_live(&chat).await;
    let second_chat = host.start().await;
    host.wait_live(&second_chat).await;
    let archived_chat = host.start().await;
    host.ok(json!({"type":"stop","session":archived_chat}))
        .await;
    host.ok(json!({"type":"archive","target":{"kind":"session","id":archived_chat}}))
        .await;
    let shell = host
        .ok(json!({"type":"startShell","cwd":host.path(),"cols":80,"rows":24}))
        .await["session"]
        .as_str()
        .unwrap()
        .to_owned();
    let other_project_chat = host
        .ok(json!({
            "type":"startChat",
            "runtime":"codex",
            "project":project,
            "cwd":host.path(),
            "cols":80,
            "rows":24
        }))
        .await["session"]
        .as_str()
        .unwrap()
        .to_owned();
    let tab = ulid::Ulid::new().to_string();
    host.layout(&tab, Some(&chat), None).await;
    let second_tab = ulid::Ulid::new().to_string();
    host.layout(&second_tab, Some(&second_chat), None).await;

    let mission = host
        .ok(json!({
            "type":"startMission",
            "crew":"crew",
            "title":"Linkable mission",
            "cwd":host.path(),
            "cols":80,
            "rows":24
        }))
        .await["mission"]
        .as_str()
        .unwrap()
        .to_owned();
    let overview = host.fixture.client.overview("2".into()).await.unwrap();
    let member_session = overview.missions[0].members[0].session.clone();
    assert_eq!(overview.missions[0].linked_chat, None);

    // Drain prior events before checking link/unlink emissions.
    while events.try_recv().is_ok() {}

    let link_reply = tokio::time::timeout(
        Duration::from_secs(5),
        host.fixture.client.request(
            "2".into(),
            c::ClientMessage::decode(
                &serde_json::to_vec(&json!({
                    "type":"linkMission",
                    "id":1,
                    "mission":mission,
                    "session":chat
                }))
                .unwrap(),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(link_reply.features.iter().any(|f| f == "linkMission"));
    assert!(link_reply.features.iter().any(|f| f == "unlinkMission"));
    assert_eq!(link_reply.result, Value::Null);

    let mut saw_mission_changed = false;
    while let Ok(event) = events.try_recv() {
        if event.name == "mission/changed" {
            saw_mission_changed = true;
        }
    }
    assert!(saw_mission_changed, "linkMission must emit mission/changed");

    let overview = host.fixture.client.overview("2".into()).await.unwrap();
    assert_eq!(
        overview.missions[0].linked_chat,
        Some(c::LinkedChat {
            session_id: chat.clone(),
            title: overview.sessions[&chat].title.clone(),
            archived: false,
            reports: true,
        })
    );

    // Move link to a second direct agent chat in the same scope.
    assert_eq!(
        host.ok(json!({"type":"linkMission","mission":mission,"session":second_chat}))
            .await,
        Value::Null
    );
    let overview = host.fixture.client.overview("2".into()).await.unwrap();
    assert_eq!(
        overview.missions[0]
            .linked_chat
            .as_ref()
            .map(|c| c.session_id.as_str()),
        Some(second_chat.as_str())
    );

    // Reject non-linkable targets without changing the existing link.
    for target in [
        archived_chat.as_str(),
        shell.as_str(),
        member_session.as_str(),
        other_project_chat.as_str(),
        "missing-chat-canary",
    ] {
        let error = host
            .request(json!({"type":"linkMission","mission":mission,"session":target}))
            .await
            .unwrap_err();
        assert_eq!(
            error, "The remote computer refused the request.",
            "{target}"
        );
        assert!(!error.contains("missing-chat-canary"));
        let stored = crate::ops::mission::get(&host.core.db.get().unwrap(), &mission).unwrap();
        assert_eq!(
            stored.linked_chat.map(|c| c.session_id),
            Some(second_chat.clone()),
            "{target}"
        );
    }

    // Unlink clears the link and keeps the mission unarchived.
    while events.try_recv().is_ok() {}
    assert_eq!(
        host.ok(json!({"type":"unlinkMission","mission":mission}))
            .await,
        Value::Null
    );
    let mut saw_unlink_changed = false;
    while let Ok(event) = events.try_recv() {
        if event.name == "mission/changed" {
            saw_unlink_changed = true;
        }
    }
    assert!(
        saw_unlink_changed,
        "unlinkMission must emit mission/changed"
    );
    let overview = host.fixture.client.overview("2".into()).await.unwrap();
    assert_eq!(overview.missions.len(), 1);
    assert_eq!(overview.missions[0].linked_chat, None);

    let stored = crate::ops::mission::get(&host.core.db.get().unwrap(), &mission).unwrap();
    assert!(stored.archived_at.is_none());
    assert_eq!(stored.linked_chat, None);
}
