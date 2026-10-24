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
        requests.last().unwrap().1["panes"],
        json!({"id":"p1","session":"same-id"})
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
