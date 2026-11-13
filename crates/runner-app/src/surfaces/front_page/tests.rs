use super::client::ChatStart;
use super::*;
use runner_app::pane_layout::{PaneLayout, SplitOrientation};
use runner_core::companion::{ActionKind, ActionTarget, ClientMessage, Pane, Tab};
use runner_core::protocol::{
    ClientError, DaemonClient, EventSubscription, Request, Response, Transport,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Fake {
    requests: Mutex<Vec<(String, Value)>>,
    local: Mutex<Vec<&'static str>>,
}

impl Transport for Fake {
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        Ok(match request {
            Request::remote_request { computer, request } => {
                let value = serde_json::to_value(&request).unwrap();
                self.requests
                    .lock()
                    .unwrap()
                    .push((computer, value.clone()));
                let result = match value["type"].as_str().unwrap() {
                    "overview" => json!({"deliberately": "not an overview"}),
                    "startChat" | "startShell" | "resume" | "restart" | "fork" => {
                        json!({"session": "same-id"})
                    }
                    "resumeMission" => {
                        json!({"mission":value["mission"],"sessions":["resumed-member"]})
                    }
                    "layout" => json!({"target": value["target"]}),
                    "createProject" => json!({"project":"host-project"}),
                    "catalog" => {
                        json!({"roles":[],"runtimes":[],"accounts":[],"activeAccount":"host-account","crews":[],"projects":[],"recentFolders":[]})
                    }
                    "folders" => {
                        json!({"path": value["path"].as_str().unwrap_or("C:\\Users\\Jason"),"parent":null,"folders":[]})
                    }
                    _ => Value::Null,
                };
                Response::remote_request(Ok(runner_core::remote::RemoteResult {
                    result,
                    features: [
                        "catalog",
                        "folders",
                        "startChat",
                        "startShell",
                        "stop",
                        "resume",
                        "restart",
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
                    .to_vec(),
                }))
            }
            Request::session_kill { .. } => {
                self.local.lock().unwrap().push("stop");
                Response::session_kill(Ok(()))
            }
            Request::session_close { .. } => {
                self.local.lock().unwrap().push("close");
                Response::session_close(Ok(()))
            }
            Request::session_rename { .. } => {
                self.local.lock().unwrap().push("rename");
                Response::session_rename(Ok(()))
            }
            Request::session_archive_direct { .. } => {
                self.local.lock().unwrap().push("archive");
                Response::session_archive_direct(Ok(()))
            }
            _ => panic!("Unexpected local request"),
        })
    }

    fn subscribe(&self) -> Box<dyn EventSubscription> {
        panic!("No subscriptions in dispatch tests")
    }
}

fn remote(fake: &Arc<Fake>, computer: &str) -> (FrontPageClient, Arc<RemoteAccess>) {
    let access = RemoteAccess::new(FrontPageTarget::Remote {
        computer: computer.into(),
        fingerprint: "identity".into(),
        generation: 1,
    });
    (
        FrontPageClient::remote(DaemonClient::new(fake.clone()), access.clone()),
        access,
    )
}

fn session() -> ActionTarget {
    ActionTarget {
        kind: ActionKind::Session,
        id: "same-id".into(),
    }
}

#[test]
fn capabilities_are_read_without_decoding_the_overview_and_invalidated_without_replay() {
    let fake = Arc::new(Fake::default());
    let (client, access) = remote(&fake, "a");
    assert!(!client.supports(&["stop"]));
    assert!(client.stop("same-id").is_err());
    assert!(fake.requests.lock().unwrap().is_empty());
    client.discover().unwrap();
    assert!(client.supports(&["startChat", "layout", "rename"]));
    assert!(!client.supports(&["startChat", "startMission"]));
    access.invalidate();
    access.set_features(vec!["stop".into()]);
    assert!(!client.supports(&["stop"]));
    assert!(client.stop("same-id").is_err());
    assert!(client.discover().is_err());
    assert_eq!(fake.requests.lock().unwrap().len(), 1);
    assert!(fake.local.lock().unwrap().is_empty());
}

#[test]
fn typed_remote_requests_keep_computer_scope_dimensions_defaults_and_host_paths() {
    let fake = Arc::new(Fake::default());
    let (a, _) = remote(&fake, "a");
    let (b, _) = remote(&fake, "b");
    a.discover().unwrap();
    b.discover().unwrap();
    assert_eq!(a.catalog().unwrap().active_account, "host-account");
    assert_eq!(a.folders(None).unwrap().path, "C:\\Users\\Jason");
    a.folders(Some("\\\\server\\share\\work".into())).unwrap();
    let input = ChatStart {
        role: Some("role".into()),
        runtime: None,
        model: None,
        effort: None,
        speed: None,
        project: None,
        cwd: None,
        size: (83, 29),
    };
    assert_eq!(a.start_chat(input).unwrap(), "same-id");
    b.start_shell(Some("project".into()), Some("C:\\work".into()), (91, 31))
        .unwrap();
    a.stop("same-id").unwrap();
    b.resume("same-id", (84, 30)).unwrap();
    a.restart("same-id", (85, 31)).unwrap();
    b.fork("same-id", Some("fork".into()), (86, 32)).unwrap();
    a.close("same-id").unwrap();
    b.rename(session(), "new title".into()).unwrap();
    a.archive(session()).unwrap();
    let mut layout = PaneLayout::single(Some("same-id"), &[]);
    layout.id = "tab".into();
    b.layout(&layout, None).unwrap();
    assert!(a
        .link_mission("mission-1".into(), "chat-1".into())
        .unwrap()
        .is_none());
    assert!(b.unlink_mission("mission-1".into()).unwrap().is_none());
    let requests = fake.requests.lock().unwrap();
    assert_eq!(
        requests[5],
        (
            "a".into(),
            json!({"type":"startChat","id":0,"role":"role","runtime":null,"model":null,"effort":null,"speed":null,"project":null,"cwd":null,"cols":83,"rows":29})
        )
    );
    assert_eq!(
        requests[6],
        (
            "b".into(),
            json!({"type":"startShell","id":0,"project":"project","cwd":"C:\\work","cols":91,"rows":31})
        )
    );
    assert_eq!(requests[8].0, "b");
    assert_eq!(requests[8].1["type"], "resume");
    assert_eq!(requests[8].1["cols"], 84);
    assert_eq!(requests[8].1["rows"], 30);
    assert_eq!(requests[9].1["type"], "restart");
    assert_eq!(requests[10].1["type"], "fork");
    assert_eq!(
        requests[14].1["panes"],
        json!({"id":"p1","session":"same-id"})
    );
    assert_eq!(
        requests[15],
        (
            "a".into(),
            json!({"type":"linkMission","id":0,"mission":"mission-1","session":"chat-1"})
        )
    );
    assert_eq!(
        requests[16],
        (
            "b".into(),
            json!({"type":"unlinkMission","id":0,"mission":"mission-1"})
        )
    );
    assert!(fake.local.lock().unwrap().is_empty());
}

#[test]
fn local_actions_keep_the_existing_daemon_calls_and_order() {
    let fake = Arc::new(Fake::default());
    let client = FrontPageClient::local(DaemonClient::new(fake.clone()));
    client.stop("same-id").unwrap();
    client.rename(session(), "name".into()).unwrap();
    client.archive(session()).unwrap();
    client.close("shell").unwrap();
    assert_eq!(
        *fake.local.lock().unwrap(),
        ["stop", "rename", "archive", "close"]
    );
    assert!(fake.requests.lock().unwrap().is_empty());
}

#[test]
fn reconciliation_preserves_surviving_ids_focus_and_drawer_and_uses_only_matching_hints() {
    let mut layout = PaneLayout::single(Some("a"), &[]);
    layout.id = "tab".into();
    let empty = layout.split("p1", SplitOrientation::Row).unwrap();
    layout.focus_pane(&empty);
    layout.add_drawer_shell("shell".into());
    layout.set_drawer_height(333.);
    let pane: Pane = serde_json::from_value(
        json!({"split":"row","sizes":[0.3,0.7],"a":{"session":"a"},"b":{"session":null}}),
    )
    .unwrap();
    let tab = Tab {
        id: "tab".into(),
        project: None,
        name: "Renamed".into(),
        pinned: false,
        panes: pane,
        shells: vec!["shell".into()],
    };
    let pushed = model::reconcile_tab(&tab, Some(&layout), None);
    assert_eq!(pushed.root.leaves()[0].id, "p1");
    assert_eq!(pushed.root.leaves()[1].id, empty);
    assert_eq!(pushed.focused_pane_id, empty);
    assert_eq!(pushed.active_drawer_shell(), Some("shell"));
    assert_eq!(pushed.drawer_height(), 333.);
    assert!(pushed.drawer_open());
    let mut hint = pushed.clone();
    hint.assign_session(&empty, "b").unwrap();
    let unchanged = model::reconcile_tab(&tab, Some(&pushed), Some(&hint));
    assert!(unchanged.root.leaves()[1].session_id.is_none());
    let mut confirmed = tab;
    confirmed.panes = serde_json::from_value(
        json!({"split":"row","sizes":[0.3,0.7],"a":{"session":"a"},"b":{"session":"b"}}),
    )
    .unwrap();
    let accepted = model::reconcile_tab(&confirmed, Some(&pushed), Some(&hint));
    assert_eq!(accepted.root.leaves()[1].id, empty);
    assert_eq!(accepted.root.leaves()[1].session_id.as_deref(), Some("b"));
    assert!(model::matches_layout(&accepted, &confirmed));
    assert!(matches!(
        serde_json::from_value::<ClientMessage>(json!({"type":"overview","id":0})).unwrap(),
        ClientMessage::Overview(_)
    ));
}

#[test]
fn sidebar_requests_use_public_targets_complete_orders_and_host_project_paths() {
    use runner_core::companion::{SidebarNode, SidebarNodeKind};
    let fake = Arc::new(Fake::default());
    let (client, _) = remote(&fake, "host");
    client.discover().unwrap();
    let node = |id: &str, kind| {
        model::SidebarNodeDisplay::from(SidebarNode {
            id: id.into(),
            kind,
            parent: None,
            position: 0,
            pinned_position: None,
        })
    };
    let project = node("host-project", SidebarNodeKind::Project);
    let mission = node("host-mission", SidebarNodeKind::Mission);
    let tab = node("host-tab", SidebarNodeKind::Tab);
    client.pin(&mission, true).unwrap();
    client
        .move_item(&tab, Some(&project), &[mission.clone(), tab.clone()])
        .unwrap();
    client.reorder_pinned(&[tab, mission]).unwrap();
    assert_eq!(
        client
            .create_project("Work".into(), "\\\\nas\\share".into())
            .unwrap(),
        "host-project"
    );
    client.delete_project("host-project".into(), false).unwrap();
    client.delete_project("host-project".into(), true).unwrap();
    client.stop_mission("host-mission".into()).unwrap();
    assert_eq!(
        client
            .resume_mission("host-mission".into(), (90, 32))
            .unwrap(),
        ["resumed-member"]
    );
    let requests = fake.requests.lock().unwrap();
    assert_eq!(
        requests[1].1,
        json!({"type":"pin","id":0,"target":{"kind":"mission","id":"host-mission"},"pinned":true})
    );
    assert_eq!(
        requests[2].1,
        json!({"type":"move","id":0,"target":{"kind":"tab","id":"host-tab"},"project":"host-project","order":[{"kind":"mission","id":"host-mission"},{"kind":"tab","id":"host-tab"}]})
    );
    assert_eq!(
        requests[3].1["order"],
        json!([{"kind":"tab","id":"host-tab"},{"kind":"mission","id":"host-mission"}])
    );
    assert_eq!(requests[4].1["path"], "\\\\nas\\share");
    assert_eq!(requests[5].1["force"], false);
    assert_eq!(requests[6].1["force"], true);
    assert_eq!(requests[8].1["cols"], 90);
    assert_eq!(requests[8].1["rows"], 32);
    assert!(fake.local.lock().unwrap().is_empty());
}

#[test]
fn host_paths_keep_platform_separators_and_project_choices_precede_deduplicated_recents() {
    use super::folder_picker::{child_path, choices};
    let catalog = serde_json::from_value(json!({"roles":[],"runtimes":[],"accounts":[],"activeAccount":"","crews":[],"projects":[{"id":"p","name":"Project","path":"C:\\work"}],"recentFolders":["C:\\work","\\\\nas\\share","/srv/work"]})).unwrap();
    let choices = choices(&catalog);
    assert_eq!(choices.len(), 3);
    assert!(choices[0].project);
    assert_eq!(choices[0].name, "Project");
    assert_eq!(choices[1].path, "\\\\nas\\share");
    assert_eq!(child_path("C:\\", "work", "windows"), "C:\\work");
    assert_eq!(
        child_path("\\\\nas\\share", "work", "WINDOWS"),
        "\\\\nas\\share\\work"
    );
    assert_eq!(child_path("/", "srv", "linux"), "/srv");
    assert_eq!(child_path("/srv", "work", "macos"), "/srv/work");
}

#[test]
fn late_bootstrap_cannot_enable_an_invalidated_connection_or_another_computer() {
    use std::sync::mpsc;
    struct Held {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    impl Transport for Held {
        fn call(&self, request: Request) -> Result<Response, ClientError> {
            assert!(matches!(
                request,
                Request::remote_request {
                    request: ClientMessage::Overview(_),
                    ..
                }
            ));
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Ok(Response::remote_request(Ok(
                runner_core::remote::RemoteResult {
                    result: json!({"tabs":[]}),
                    features: vec!["stop".into()],
                },
            )))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            panic!("No subscription")
        }
    }
    let (entered, started) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let transport = Arc::new(Held {
        entered,
        release: Mutex::new(resume),
    });
    let access = RemoteAccess::new(FrontPageTarget::Remote {
        computer: "first".into(),
        fingerprint: "old".into(),
        generation: 1,
    });
    let client = FrontPageClient::remote(DaemonClient::new(transport), access.clone());
    let held = client.clone();
    let read = std::thread::spawn(move || held.discover());
    started
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    access.invalidate();
    let other = RemoteAccess::new(FrontPageTarget::Remote {
        computer: "second".into(),
        fingerprint: "other".into(),
        generation: 2,
    });
    release.send(()).unwrap();
    read.join().unwrap().unwrap();
    assert!(!client.supports(&["stop"]));
    assert!(!other.supports(&["stop"]));
}

#[derive(Default)]
struct LocalDispatch {
    calls: Mutex<Vec<Request>>,
}

impl Transport for LocalDispatch {
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        self.calls.lock().unwrap().push(request);
        Err(ClientError::msg("unchanged daemon error"))
    }

    fn subscribe(&self) -> Box<dyn EventSubscription> {
        panic!("dispatch does not subscribe")
    }
}

#[test]
fn local_workspace_mutations_keep_daemon_requests_arguments_and_errors() {
    use runner_core::protocol::{mission::MissionStart, model::CodexSpeed, project::ProjectScope};
    let fake = Arc::new(LocalDispatch::default());
    let client = FrontPageClient::local(DaemonClient::new(fake.clone()));
    macro_rules! dispatch {
        ($operation:expr_2021, $request:expr_2021) => {{
            assert_eq!(
                $operation.err().unwrap().to_string(),
                "unchanged daemon error"
            );
            let calls = std::mem::take(&mut *fake.calls.lock().unwrap());
            assert_eq!(calls.len(), 1);
            assert_eq!(
                serde_json::to_value(&calls[0]).unwrap(),
                serde_json::to_value($request).unwrap()
            );
        }};
    }
    let scope = ProjectScope::or_root(Some("project".into()));
    let input = ChatStart {
        role: Some("role".into()),
        runtime: Some("codex".into()),
        model: Some("model".into()),
        effort: Some("low".into()),
        speed: Some("fast".into()),
        project: Some("project".into()),
        cwd: Some("/work".into()),
        size: (101, 37),
    };
    dispatch!(
        client.start_chat(input.clone()),
        Request::session_start_direct_with_speed {
            role_id: "role".into(),
            runtime: Some("codex".into()),
            model: Some("model".into()),
            effort: Some("low".into()),
            speed: Some(CodexSpeed::Fast),
            scope: scope.clone(),
            cwd: Some("/work".into()),
            cols: Some(101),
            rows: Some(37),
        }
    );
    dispatch!(
        client.start_chat(ChatStart {
            role: None,
            speed: None,
            project: None,
            ..input
        }),
        Request::session_start_runtime_with_speed {
            runtime: "codex".into(),
            model: Some("model".into()),
            effort: Some("low".into()),
            speed: None,
            scope: ProjectScope::Root,
            cwd: Some("/work".into()),
            cols: Some(101),
            rows: Some(37),
        }
    );
    dispatch!(
        client.start_shell(Some("project".into()), Some("/work".into()), (99, 31)),
        Request::session_start_shell_in {
            scope: scope.clone(),
            cwd: Some("/work".into()),
            cols: Some(99),
            rows: Some(31),
        }
    );
    dispatch!(
        client.resume("chat", (83, 29)),
        Request::session_resume {
            session_id: "chat".into(),
            cols: Some(83),
            rows: Some(29)
        }
    );
    dispatch!(
        client.restart("chat", (84, 30)),
        Request::session_restart {
            session_id: "chat".into(),
            cols: Some(84),
            rows: Some(30)
        }
    );
    dispatch!(
        client.resume_session("shell", (85, 31), true),
        Request::session_resume_on_launch {
            session_id: "shell".into(),
            cols: Some(85),
            rows: Some(31)
        }
    );
    dispatch!(
        client.fork("chat", Some("Fork title".into()), (86, 32)),
        Request::session_fork {
            source_session_id: "chat".into(),
            title: Some("Fork title".into()),
            cols: Some(86),
            rows: Some(32)
        }
    );
    dispatch!(
        client.rename_title(
            ActionTarget {
                kind: ActionKind::Session,
                id: "chat".into()
            },
            None
        ),
        Request::session_rename {
            session_id: "chat".into(),
            title: None
        }
    );
    dispatch!(
        client.pin_chat("chat", true, None),
        Request::session_pin {
            session_id: "chat".into(),
            pinned: true
        }
    );
    dispatch!(
        client.archive_chat("chat", false),
        Request::session_archive {
            session_id: "chat".into()
        }
    );
    dispatch!(
        client.start_mission(
            "".into(),
            "Mission".into(),
            Some("Goal".into()),
            Some("project".into()),
            Some("/work".into()),
            (91, 33)
        ),
        Request::mission_start_impl_with_size {
            input: MissionStart {
                crew_id: None,
                scope,
                title: "Mission".into(),
                goal_override: Some("Goal".into()),
                cwd: Some("/work".into()),
                started_by_session_id: None,
                no_report: false,
            },
            initial_size: Some((91, 33)),
        }
    );
    dispatch!(
        client.stop_mission("mission".into()),
        Request::mission_stop_impl {
            id: "mission".into()
        }
    );
    dispatch!(
        client.pin_mission_state("mission".into(), true),
        Request::mission_pin_impl {
            id: "mission".into(),
            pinned: true
        }
    );
    dispatch!(
        client.rename_mission_state("mission".into(), "Title".into()),
        Request::mission_rename_impl {
            id: "mission".into(),
            title: "Title".into()
        }
    );
    dispatch!(
        client.archive_mission_state("mission".into()),
        Request::mission_archive_impl {
            id: "mission".into()
        }
    );
    dispatch!(
        client.mark_viewed("chat".into()),
        Request::mark_direct_sessions_viewed {
            session_ids: vec!["chat".into()]
        }
    );
    dispatch!(
        client.post(
            "mission".into(),
            vec!["coder".into(), "reviewer".into()],
            "Text".into()
        ),
        Request::mission_post_impl {
            input: runner_core::protocol::mission::PostMessageInput {
                mission_id: "mission".into(),
                from: None,
                text: "Text".into(),
                to: runner_core::model::Recipients::new(vec!["coder".into(), "reviewer".into()])
            },
        }
    );
    dispatch!(
        client.answer("mission".into(), "question".into(), "yes".into()),
        Request::mission_signal_impl {
            input: runner_core::protocol::mission::PostSignalInput {
                mission_id: "mission".into(),
                from: None,
                signal_type: "human_response".into(),
                payload: json!({"question_id":"question","choice":"yes"})
            },
        }
    );
    let layout = runner_app::pane_layout::MissionLayout::default();
    dispatch!(
        client.save_mission_presentation(Some("node"), &layout),
        Request::node_mission_layout_set {
            node_id: "node".into(),
            layout: layout.serialize().unwrap()
        }
    );
    dispatch!(
        client.start_role_chat("role".into(), Some("/role-work".into()), (80, 24)),
        Request::session_start_direct {
            role_id: "role".into(),
            runtime: None,
            model: None,
            effort: None,
            scope: ProjectScope::Root,
            cwd: Some("/role-work".into()),
            cols: Some(80),
            rows: Some(24)
        }
    );
    let node = |id: &str, kind, reference: Option<&str>| model::SidebarNodeDisplay {
        id: id.into(),
        parent_id: None,
        node_type: kind,
        ref_id: reference.map(str::to_owned),
        position: 0,
        pinned_position: None,
        created_at: None,
    };
    let tab = node("tab-node", runner_core::protocol::node::NodeType::Tab, None);
    let project = node(
        "project-node",
        runner_core::protocol::node::NodeType::Project,
        Some("project"),
    );
    dispatch!(
        client.pin(&tab, true),
        Request::node_set_pinned {
            id: "tab-node".into(),
            pinned: true
        }
    );
    dispatch!(
        client.move_item(&tab, Some(&project), std::slice::from_ref(&tab)),
        Request::node_move {
            id: "tab-node".into(),
            parent_id: Some("project-node".into()),
            ordered_ids: vec!["tab-node".into()]
        }
    );
    dispatch!(
        client.reorder_pinned(&[tab]),
        Request::node_reorder_pinned {
            ordered_ids: vec!["tab-node".into()]
        }
    );
    let layout = PaneLayout::single(Some("chat"), &[]);
    dispatch!(
        client.layout(&layout, Some("project".into())),
        Request::node_tab_upsert {
            input: layout.upsert_input().unwrap()
        }
    );
    dispatch!(
        client.rename(
            ActionTarget {
                kind: ActionKind::Tab,
                id: "tab-node".into()
            },
            "Tab".into()
        ),
        Request::node_rename {
            id: "tab-node".into(),
            name: "Tab".into()
        }
    );
    dispatch!(
        client.rename(
            ActionTarget {
                kind: ActionKind::Project,
                id: "project".into()
            },
            "Project".into()
        ),
        Request::project_rename {
            id: "project".into(),
            name: "Project".into()
        }
    );
    dispatch!(
        client.archive(ActionTarget {
            kind: ActionKind::Tab,
            id: "tab-node".into()
        }),
        Request::node_tab_delete {
            node_id: "tab-node".into()
        }
    );
    dispatch!(
        client.create_project("Project".into(), "/work".into()),
        Request::project_create {
            name: "Project".into(),
            cwd: "/work".into()
        }
    );
    dispatch!(
        client.delete_project("project".into(), true),
        Request::project_delete {
            id: "project".into()
        }
    );
    dispatch!(
        client.delete_project("project".into(), false),
        Request::project_delete_checked {
            id: "project".into(),
            force: false
        }
    );
    client.follow_feed("mission".into()).unwrap();
    client.unfollow_feed("mission".into()).unwrap();
    assert!(fake.calls.lock().unwrap().is_empty());
}

struct ScriptedLocal {
    steps: Mutex<std::collections::VecDeque<(Request, Response)>>,
}

impl Transport for ScriptedLocal {
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        let (expected, response) = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected daemon request");
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        Ok(response)
    }
    fn subscribe(&self) -> Box<dyn EventSubscription> {
        panic!("no dispatch subscription")
    }
}

#[test]
fn local_mission_resume_preserves_concurrent_refresh_partial_failure_and_dimensions() {
    use runner_core::protocol::{
        model::{Session, SessionStatus},
        session::{SessionRow, SpawnedSession},
    };
    let member = |id: &str, status| SessionRow {
        session: Session {
            id: id.into(),
            mission_id: Some("mission".into()),
            role_id: "role".into(),
            slot_id: None,
            cwd: None,
            status,
            pid: None,
            started_at: None,
            stopped_at: None,
        },
        live_title: None,
        handle: id.into(),
        role_handle: "role".into(),
        runtime: "codex".into(),
        lead: false,
        agent_session_key: None,
    };
    let list = || Request::session_list {
        mission_id: "mission".into(),
    };
    let resume = |id: &str| Request::session_resume {
        session_id: id.into(),
        cols: Some(91),
        rows: Some(33),
    };
    let initial = vec![
        member("running", SessionStatus::Running),
        member("concurrent", SessionStatus::Stopped),
        member("failed", SessionStatus::Stopped),
        member("last", SessionStatus::Stopped),
    ];
    let refreshed = vec![
        member("running", SessionStatus::Running),
        member("concurrent", SessionStatus::Running),
        member("failed", SessionStatus::Stopped),
        member("last", SessionStatus::Stopped),
    ];
    let final_rows = vec![
        member("running", SessionStatus::Running),
        member("concurrent", SessionStatus::Running),
        member("failed", SessionStatus::Stopped),
        member("last", SessionStatus::Running),
    ];
    let fake = Arc::new(ScriptedLocal {
        steps: Mutex::new(
            [
                (list(), Response::session_list(Ok(initial))),
                (
                    resume("concurrent"),
                    Response::session_resume(Err(ClientError::msg(
                        "session is already being resumed",
                    ))),
                ),
                (list(), Response::session_list(Ok(refreshed))),
                (
                    resume("failed"),
                    Response::session_resume(Err(ClientError::msg("original resume failure"))),
                ),
                (
                    resume("last"),
                    Response::session_resume(Ok(SpawnedSession {
                        id: "last".into(),
                        mission_id: Some("mission".into()),
                        role_id: Some("role".into()),
                        handle: "last".into(),
                        pid: Some(12),
                    })),
                ),
                (list(), Response::session_list(Ok(final_rows.clone()))),
            ]
            .into(),
        ),
    });
    let client = FrontPageClient::local(DaemonClient::new(fake.clone()));
    let result = client
        .resume_mission_state("mission".into(), (91, 33))
        .unwrap();
    assert_eq!(result.sessions, ["last"]);
    assert_eq!(result.error.as_deref(), Some("original resume failure"));
    assert_eq!(
        serde_json::to_value(result.local_sessions).unwrap(),
        serde_json::to_value(final_rows).unwrap()
    );
    assert!(fake.steps.lock().unwrap().is_empty());
}

struct EntryTrace {
    base: Arc<crate::surfaces::remote_client::tests::Fake>,
    calls: Mutex<Vec<Value>>,
}

impl Transport for EntryTrace {
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        let value = serde_json::to_value(&request).unwrap();
        self.calls.lock().unwrap().push(value.clone());
        let local = [
            "session_kill",
            "session_resume",
            "session_fork",
            "session_archive",
        ]
        .iter()
        .any(|method| value.get(method).is_some());
        let remote = value["remote_request"]["request"]["type"]
            .as_str()
            .is_some_and(|method| matches!(method, "stop" | "resume" | "fork" | "archive"));
        if local || remote {
            return Err(ClientError::msg("entry dispatch failure"));
        }
        self.base.call(request)
    }
    fn subscribe(&self) -> Box<dyn EventSubscription> {
        self.base.subscribe()
    }
    fn attach(
        &self,
        session: &str,
    ) -> Result<runner_core::protocol::terminal::TerminalAttachment, ClientError> {
        self.base.attach(session)
    }
    fn attach_remote(
        &self,
        config: runner_core::remote::RemoteTerminal,
    ) -> Result<runner_core::protocol::terminal::TerminalAttachment, ClientError> {
        self.base.attach_remote(config)
    }
}

#[test]
fn chat_lifecycle_entry_points_dispatch_once_for_local_and_remote_targets() {
    use crate::surfaces::remote_client::tests as remote_test;
    use crate::{AppRoute, ForkConfirm};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for remote in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
        remote_test::enable_front_page(&fake);
        let trace = Arc::new(EntryTrace {
            base: fake,
            calls: Mutex::new(Vec::new()),
        });
        store.update(&mut cx, |store, _| {
            store.client = DaemonClient::new(trace.clone())
        });
        let window =
            remote_test::add_window(&mut cx, &store, temp.path(), "workspace-entry-points");
        window.update(&mut cx, |root, window, cx| {
            if remote { remote_test::choose_host(root, window, cx); }
            else {
                let conn = root.app_store.read(cx).test_core.db.get().unwrap();
                conn.execute("INSERT INTO sessions(id, status, agent_runtime, agent_command, agent_session_key) VALUES ('local-chat', 'stopped', 'codex', 'codex', 'conversation')", []).unwrap();
                drop(conn);
                let mut layout = PaneLayout::single(Some("local-chat"), &[]);
                layout.id = "01ARZ3NDEKTSV4RRFFQ69G5FAV".into();
                root.core(cx).node_tab_upsert(layout.upsert_input().unwrap()).unwrap();
                root.refresh_sessions(cx);
                root.reload_tabs(cx).unwrap();
                root.tabs.activate(&layout.id);
                root.route = AppRoute::Chat;
            }
        }).unwrap();
        cx.run_until_parked();
        let session = if remote { "s" } else { "local-chat" };
        for (command, local_method) in [
            ("stop", "session_kill"),
            ("resume", "session_resume"),
            ("fork", "session_fork"),
            ("archive", "session_archive"),
        ] {
            trace.calls.lock().unwrap().clear();
            window
                .update(&mut cx, |root, window, cx| match command {
                    "stop" => root.stop_chat(session, window, cx),
                    "resume" => root.resume_chat("p1", session, window, cx),
                    "archive" => {
                        let close = crate::PendingPaneClose {
                            tab_id: root.front_page_layout().unwrap().id.clone(),
                            pane_id: "p1".into(),
                        };
                        crate::TerminalCloseConfirm::open(
                            &mut root.terminal_close_confirm,
                            crate::TerminalCloseTarget::ArchiveChatPane {
                                session_id: session.into(),
                                close,
                            },
                            window,
                            cx,
                        );
                        root.confirm_terminal_close(window, cx);
                    }
                    _ => {
                        root.fork_confirm = Some(ForkConfirm {
                            session_id: session.into(),
                            pending: false,
                        });
                        root.confirm_fork_chat(window, cx);
                    }
                })
                .unwrap();
            cx.run_until_parked();
            let calls = trace.calls.lock().unwrap();
            let requests = calls
                .iter()
                .filter(|value| {
                    if remote {
                        value["remote_request"]["request"]["type"] == command
                    } else {
                        value.get(local_method).is_some()
                    }
                })
                .collect::<Vec<_>>();
            assert_eq!(requests.len(), 1, "{remote}/{command}: {calls:?}");
            let request = if remote {
                &requests[0]["remote_request"]["request"]
            } else {
                &requests[0][local_method]
            };
            assert_eq!(
                if remote && command == "archive" {
                    request["target"]["id"].clone()
                } else {
                    request[if remote {
                        "session"
                    } else if command == "fork" {
                        "source_session_id"
                    } else {
                        "session_id"
                    }]
                    .clone()
                },
                session
            );
            window
                .read_with(&cx, |root, _| {
                    let error = if remote {
                        root.remote_client
                            .workspace()
                            .unwrap()
                            .action_error
                            .as_deref()
                            .map(str::to_owned)
                    } else {
                        root.chat_error
                            .as_ref()
                            .map(|error| error.resolve().to_string())
                    };
                    assert_eq!(
                        error.as_deref(),
                        Some("entry dispatch failure"),
                        "{remote}/{command}"
                    );
                })
                .unwrap();
        }
    }
}

#[test]
fn new_terminal_entry_points_inherit_remote_sibling_cwd_when_focused_pane_is_empty() {
    use crate::surfaces::remote_client::tests as remote_test;
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for shortcut in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
        remote_test::enable_front_page(&fake);
        fake.set_overview(
            serde_json::from_value(json!({
                "projects":[{"id":"p", "name":"Project", "path":"/project/default"}],
                "tabs":[{"id":"remote-tab", "project":"p", "name":"Shell tab", "pinned":false,
                    "panes":{"split":"column", "sizes":[0.5,0.5],
                        "a":{"session":"s"}, "b":{"session":null}}, "shells":[]}],
                "missions":[],
                "sessions":{"s":{"kind":"shell", "title":"Sibling", "runtime":"shell",
                    "status":"stopped", "unread":false, "cwd":"/custom/sibling-folder"}}
            }))
            .unwrap(),
        );
        let window = remote_test::add_window(&mut cx, &store, temp.path(), "empty-shell-cwd");
        window.update(&mut cx, remote_test::choose_host).unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, window, cx| {
                let layout = root.front_page_layout_mut().unwrap();
                let empty = layout.root.leaves()[1].id.clone();
                layout.focus_pane(&empty);
                assert!(layout.focused_session_id().is_none());
                if shortcut {
                    root.new_terminal_action(&crate::NewTerminal, window, cx);
                } else {
                    root.new_terminal(window, cx);
                }
            })
            .unwrap();
        cx.run_until_parked();
        let calls = remote_test::action_trace(&fake, "startShell");
        assert_eq!(calls.len(), 1, "shortcut={shortcut}");
        assert_eq!(calls[0]["cwd"], "/custom/sibling-folder");
        assert_eq!(calls[0]["project"], "p");
    }
}

struct PendingActionTrace {
    base: Arc<crate::surfaces::remote_client::tests::Fake>,
    operation: &'static str,
    entered: std::sync::mpsc::Sender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    completed: std::sync::atomic::AtomicBool,
}

impl Transport for PendingActionTrace {
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        let value = serde_json::to_value(&request).unwrap();
        if value["remote_request"]["request"]["type"] == self.operation {
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            self.completed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            return Err(ClientError::msg("held action failure"));
        }
        self.base.call(request)
    }

    fn subscribe(&self) -> Box<dyn EventSubscription> {
        self.base.subscribe()
    }

    fn attach_remote(
        &self,
        config: runner_core::remote::RemoteTerminal,
    ) -> Result<runner_core::protocol::terminal::TerminalAttachment, ClientError> {
        self.base.attach_remote(config)
    }
}

#[test]
fn remote_actions_publish_pending_and_clear_errors_before_held_requests_complete() {
    use crate::surfaces::remote_client::tests as remote_test;
    use std::sync::{atomic::Ordering, mpsc};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for operation in ["stop", "archive"] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
        remote_test::enable_front_page(&fake);
        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let trace = Arc::new(PendingActionTrace {
            base: fake,
            operation,
            entered,
            release: Mutex::new(blocked),
            completed: Default::default(),
        });
        store.update(&mut cx, |store, _| {
            store.client = DaemonClient::new(trace.clone());
        });
        let window = remote_test::add_window(&mut cx, &store, temp.path(), "pending-action");
        window.update(&mut cx, remote_test::choose_host).unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, _, cx| {
                root.remote_client.workspace_mut().unwrap().action_error = Some("old error".into());
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let entity = window.root(&mut cx).unwrap();
        let (observed, notifications) = mpsc::channel();
        let pending_trace = trace.clone();
        let key = if operation == "stop" {
            "session:s"
        } else {
            "archive-sessions"
        };
        let _subscription = cx.update(|cx| {
            cx.observe(&entity, move |root, cx| {
                let root = root.read(cx);
                let remote = root.remote_client.workspace().unwrap();
                let _ = observed.send((
                    remote.actions_pending.contains(key),
                    remote.action_error.clone(),
                    root.session_lifecycle_disabled("s", cx),
                    pending_trace.completed.load(Ordering::SeqCst),
                ));
            })
        });
        let observer = std::thread::spawn(move || {
            let timeout = std::time::Duration::from_secs(5);
            let entered = started.recv_timeout(timeout);
            let state = notifications.recv_timeout(timeout);
            let _ = release.send(());
            (entered, state)
        });
        window
            .update(&mut cx, |root, window, cx| {
                if operation == "stop" {
                    root.stop_chat("s", window, cx);
                } else {
                    root.archive_workspace_sessions(
                        vec!["s".into()],
                        crate::ArchiveAllSource::Chat,
                        None,
                        window,
                        cx,
                    );
                }
            })
            .unwrap();
        cx.run_until_parked();
        let (entered, state) = observer.join().unwrap();
        entered.unwrap();
        let (pending, error, lifecycle_disabled, completed) = state.unwrap();
        assert!(pending, "{operation}");
        assert_eq!(error, None, "{operation}");
        if operation == "stop" {
            assert!(lifecycle_disabled);
        }
        assert!(
            !completed,
            "{operation} was only published after completion"
        );
        assert!(trace.completed.load(Ordering::SeqCst));
    }
}

struct HeldLauncherStart {
    base: Arc<crate::surfaces::remote_client::tests::Fake>,
    calls: std::sync::atomic::AtomicUsize,
    entered: std::sync::mpsc::Sender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}
impl Transport for HeldLauncherStart {
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        let value = serde_json::to_value(&request).unwrap();
        if value["remote_request"]["request"]["type"] == "startChat"
            && self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0
        {
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        self.base.call(request)
    }
    fn subscribe(&self) -> Box<dyn EventSubscription> {
        self.base.subscribe()
    }
    fn attach_remote(
        &self,
        config: runner_core::remote::RemoteTerminal,
    ) -> Result<runner_core::protocol::terminal::TerminalAttachment, ClientError> {
        self.base.attach_remote(config)
    }
}

#[test]
fn delayed_launcher_starts_serialize_two_empty_panes_and_preserve_the_first_assignment() {
    use crate::surfaces::remote_client::tests as remote_test;
    use std::sync::{atomic::Ordering, mpsc};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
    remote_test::enable_front_page(&fake);
    let mut data = remote_test::overview();
    data.tabs[0].panes = serde_json::from_value(
        json!({"split":"column","sizes":[0.5,0.5],"a":{"session":null},"b":{"session":null}}),
    )
    .unwrap();
    fake.set_overview(data.clone());
    let (entered, started) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let (attempted, attempts) = mpsc::channel();
    let trace = Arc::new(HeldLauncherStart {
        base: fake.clone(),
        calls: Default::default(),
        entered,
        release: Mutex::new(blocked),
    });
    store.update(&mut cx, |store, _| {
        store.client = DaemonClient::new(trace.clone())
    });
    let window = remote_test::add_window(&mut cx, &store, temp.path(), "serialized-launchers");
    window.update(&mut cx, remote_test::choose_host).unwrap();
    let other = remote_test::add_window(&mut cx, &store, temp.path(), "other-launcher-window");
    other.update(&mut cx, remote_test::choose_host).unwrap();
    cx.run_until_parked();
    let observer = std::thread::spawn(move || {
        let entered = started.recv_timeout(std::time::Duration::from_secs(5));
        let attempted = attempts.recv_timeout(std::time::Duration::from_secs(5));
        let _ = release.send(());
        (entered, attempted)
    });
    window
        .update(&mut cx, |root, window, cx| {
            let original = root.front_page_layout().unwrap().clone();
            let first = original.root.leaves()[0].id.clone();
            let second = original.root.leaves()[1].id.clone();
            root.focus_pane(&first, cx);
            root.test_start_launcher(window, cx);
            assert!(root.launcher_tab_pending(&original.id, cx));
            root.focus_pane(&second, cx);
            root.test_start_launcher(window, cx);
            root.split_pane(&second, SplitOrientation::Row, window, cx);
            root.close_pane(&second, window, cx);
            assert_eq!(root.front_page_layout().unwrap().root, original.root);
            assert!(root.remote_client.layout_pending(&original.id));
        })
        .unwrap();
    attempted.send(()).unwrap();
    cx.run_until_parked();
    let (entered, attempted) = observer.join().unwrap();
    entered.unwrap();
    attempted.unwrap();
    assert_eq!(trace.calls.load(Ordering::SeqCst), 1);
    let layouts = remote_test::action_trace(&fake, "layout");
    assert_eq!(layouts.len(), 1);
    assert_eq!(layouts[0]["panes"]["a"]["session"], "started-session");
    assert!(layouts[0]["panes"]["b"]["session"].is_null());
    other
        .update(&mut cx, |root, window, cx| {
            let layout = root.front_page_layout().unwrap();
            assert!(root.launcher_tab_pending(&layout.id, cx));
            let second = layout.root.leaves()[1].id.clone();
            root.focus_pane(&second, cx);
            root.test_start_launcher(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(trace.calls.load(Ordering::SeqCst), 1);
    data.tabs[0].panes = serde_json::from_value(json!({"split":"column","sizes":[0.5,0.5],"a":{"session":"started-session"},"b":{"session":null}})).unwrap();
    data.sessions
        .insert("started-session".into(), data.sessions["s"].clone());
    fake.set_overview(data);
    cx.run_until_parked();
    fake.queue_result("startChat", Ok(json!({"session":"second-session"})));
    other
        .update(&mut cx, |root, window, cx| {
            let layout = root.front_page_layout().unwrap();
            assert!(!root.launcher_tab_pending(&layout.id, cx));
            let second = layout.root.leaves()[1].id.clone();
            root.focus_pane(&second, cx);
            root.test_start_launcher(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(trace.calls.load(Ordering::SeqCst), 2);
    let layouts = remote_test::action_trace(&fake, "layout");
    assert_eq!(layouts.len(), 2);
    assert_eq!(layouts[1]["panes"]["a"]["session"], "started-session");
    assert_eq!(layouts[1]["panes"]["b"]["session"], "second-session");
}

#[test]
fn closing_a_single_empty_remote_tab_is_blocked_during_a_held_launcher_start() {
    use crate::surfaces::remote_client::tests as remote_test;
    use std::sync::mpsc;
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
    remote_test::enable_front_page(&fake);
    let mut data = remote_test::overview();
    data.tabs[0].panes = runner_core::companion::Pane::Leaf { session: None };
    fake.set_overview(data);
    let (entered, started) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let trace = Arc::new(HeldLauncherStart {
        base: fake.clone(),
        calls: Default::default(),
        entered,
        release: Mutex::new(blocked),
    });
    store.update(&mut cx, |store, _| store.client = DaemonClient::new(trace));
    let window = remote_test::add_window(&mut cx, &store, temp.path(), "held-empty-close");
    window.update(&mut cx, remote_test::choose_host).unwrap();
    cx.run_until_parked();
    let observer = std::thread::spawn(move || {
        started
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        release.send(()).unwrap();
    });
    window
        .update(&mut cx, |root, window, cx| {
            let original = root.front_page_layout().unwrap().clone();
            root.test_start_launcher(window, cx);
            crate::surfaces::windowing::close_tab(root, window, cx);
            assert!(root.launcher_tab_pending(&original.id, cx));
            assert_eq!(root.front_page_layout().unwrap().id, original.id);
        })
        .unwrap();
    cx.run_until_parked();
    observer.join().unwrap();
    assert!(remote_test::action_trace(&fake, "archive").is_empty());
    assert_eq!(remote_test::action_trace(&fake, "layout").len(), 1);
}
