use std::sync::Arc;

use chrono::Utc;
use gpui::prelude::*;
use gpui::px;
use runner_app::ui::SessionControlKind;
use runner_daemon::model::{Event, EventKind, MissionStatus, SessionStatus};

use super::*;
use crate::surfaces::*;
use crate::*;

struct SuspendedMissionClient {
    base: runner_daemon::daemon::InProcessTransport,
    replay: std::sync::Mutex<Vec<Event>>,
    viewed: std::sync::Mutex<Vec<String>>,
}

impl runner_core::protocol::Transport for SuspendedMissionClient {
    fn call(
        &self,
        request: runner_core::protocol::Request,
    ) -> Result<runner_core::protocol::Response, runner_core::protocol::ClientError> {
        use runner_core::protocol::{Request, Response};
        match request {
            Request::mission_attach { mission_id } => Ok(Response::mission_attach(
                runner_daemon::ops::mission::mission_get(&self.base.0, &mission_id)
                    .map_err(|error| runner_core::protocol::ClientError::msg(error.to_string())),
            )),
            Request::mission_events_replay { .. } => Ok(Response::mission_events_replay(Ok(self
                .replay
                .lock()
                .unwrap()
                .clone()))),
            Request::remote_follow { .. } => Ok(Response::remote_follow(Ok(()))),
            Request::mark_direct_sessions_viewed { session_ids } => {
                self.viewed.lock().unwrap().extend(session_ids.clone());
                self.base
                    .call(Request::mark_direct_sessions_viewed { session_ids })
            }
            Request::remote_overview { .. } => Ok(Response::remote_overview(Ok(
                runner_core::remote::RemoteOverview(
                    serde_json::from_value(
                        serde_json::json!({"projects":[],"tabs":[],"missions":[],"sessions":{}}),
                    )
                    .unwrap(),
                ),
            ))),
            request => self.base.call(request),
        }
    }

    fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
        self.base.subscribe()
    }
}

fn suspended_mission_test() -> (
    tempfile::TempDir,
    gpui::TestAppContext,
    gpui::WindowHandle<NativeRoot>,
    Arc<SuspendedMissionClient>,
) {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store)
    });
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    let fake = Arc::new(SuspendedMissionClient {
        base: runner_daemon::daemon::InProcessTransport(core.clone()),
        replay: Default::default(),
        viewed: Default::default(),
    });
    let client = runner_core::protocol::DaemonClient::new(fake.clone());
    let clients = cx.new(|cx| remote_client::model::ClientState::new(client.clone(), cx));
    store.update(&mut cx, |store, _| {
        store.client = client;
        store.remote_clients = clients;
        let events: Arc<dyn runner_daemon::session::manager::SessionEvents> =
            Arc::new(core.session_events());
        for id in ["mission-agent", "mission-shell"] {
            core.sessions
                .prepare_unlisted_terminal(id, (80, 24), &core.db, &events)
                .unwrap();
            store.bridge.attach(id).unwrap();
        }
    });
    cx.update(|cx| {
        cx.set_global(GlobalNativePaths(NativePaths::new(
            temp.path().into(),
            temp.path().join("logs"),
        )));
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        crate::notifications::install(cx);
    });
    let host = cx.add_window(|window, cx| {
        NativeRoot::new(
            "main".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    cx.simulate_window_resize(host.into(), size(px(1440.), px(900.)));
    cx.run_until_parked();
    store.update(&mut cx, |store, _| {
        store.remote_status = Some(runner_core::remote::RemoteStatus {
            state: runner_core::remote::RemoteState::Paused,
            ..Default::default()
        });
    });
    (temp, cx, host, fake)
}

fn other_computer() -> runner_core::remote::Computer {
    runner_core::remote::Computer {
        id: "host".into(),
        name: "Other computer".into(),
        paired: true,
        online: true,
        ..Default::default()
    }
}

fn local_mission_event(id: &str) -> Event {
    Event {
        id: id.into(),
        ts: Utc::now(),
        crew_id: Some("test-crew".into()),
        mission_id: "test-mission".into(),
        kind: EventKind::Message,
        from: "human".into(),
        to: Default::default(),
        signal_type: None,
        payload: serde_json::json!({"text": id}),
    }
}

#[test]
fn local_mission_resumes_terminals_selection_and_missed_state_after_a_remote_visit() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, host, fake) = suspended_mission_test();
    host.update(&mut cx, |root, window, cx| {
        root.open_mission("test-mission".into(), window, cx)
    })
    .unwrap();
    cx.run_until_parked();
    let layout = host
        .update(&mut cx, |root, window, cx| {
            let layout = root.mission_workspace.update(cx, |workspace, cx| {
                assert!(!workspace.loading);
                assert!(workspace.attached.contains_key("mission-agent"));
                workspace.active_tab = MissionTab::Session("mission-agent".into());
                workspace.open_tabs.retain(|id| id != "mission-shell");
                workspace.layout.drawer.set_height(333.);
                workspace.composer.draft = "Keep this draft".into();
                assert!(workspace.mission_terminal_interactive("mission-agent", cx));
                workspace.layout.serialize().unwrap()
            });
            root.visit_computer(other_computer(), window, cx);
            assert!(!root.mission_workspace.read(cx).active);
            assert!(root.mission_workspace.read(cx).attached.is_empty());
            layout
        })
        .unwrap();
    cx.run_until_parked();
    let missed = local_mission_event("missed-while-remote");
    fake.replay.lock().unwrap().push(missed.clone());
    fake.base.0.events.emit(
        "event/appended",
        &serde_json::json!({"mission_id":"test-mission", "event": missed}),
    );
    fake.base
        .0
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE missions SET title = 'Changed while remote' WHERE id = 'test-mission'",
            [],
        )
        .unwrap();
    cx.run_until_parked();
    host.read_with(&cx, |root, cx| {
        assert!(root.mission_workspace.read(cx).events.is_empty())
    })
    .unwrap();
    host.update(&mut cx, |root, window, cx| {
        root.return_to_this_computer(window, cx)
    })
    .unwrap();
    cx.run_until_parked();
    host.read_with(&cx, |root, cx| {
        let workspace = root.mission_workspace.read(cx);
        assert!(workspace.active);
        assert!(workspace.attached.contains_key("mission-agent"));
        assert!(!workspace.attached.contains_key("mission-shell"));
        assert!(workspace.mission_terminal_interactive("mission-agent", cx));
        assert_eq!(
            workspace.active_tab,
            MissionTab::Session("mission-agent".into())
        );
        assert_eq!(workspace.layout.serialize().unwrap(), layout);
        assert_eq!(workspace.composer.draft, "Keep this draft");
        assert_eq!(
            workspace.mission.as_ref().unwrap().title,
            "Changed while remote"
        );
        assert!(workspace
            .events
            .iter()
            .any(|event| event.id == "missed-while-remote"));
    })
    .unwrap();
    fake.base.0.events.emit(
        "event/appended",
        &serde_json::json!({"mission_id":"test-mission", "event": local_mission_event("after-return")}),
    );
    cx.run_until_parked();
    host.read_with(&cx, |root, cx| {
        assert!(root
            .mission_workspace
            .read(cx)
            .events
            .iter()
            .any(|event| event.id == "after-return"))
    })
    .unwrap();
}

#[cfg(unix)]
#[test]
fn same_mission_notification_opens_local_window_and_leaves_remote_host_connected() {
    fn wait_app(
        cx: &mut gpui::TestAppContext,
        label: &str,
        mut pred: impl FnMut(&mut gpui::TestAppContext) -> bool,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            if pred(cx) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {label}"
            );
            std::thread::yield_now();
        }
    }

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, host, fake) = suspended_mission_test();
    host.update(&mut cx, |root, window, cx| {
        root.open_mission("test-mission".into(), window, cx)
    })
    .unwrap();
    wait_app(&mut cx, "initial local mission load", |cx| {
        host.read_with(cx, |root, cx| {
            let workspace = root.mission_workspace.read(cx);
            !workspace.loading
                && workspace.mission.is_some()
                && workspace.attached.contains_key("mission-agent")
        })
        .unwrap()
    });
    host.update(&mut cx, |root, window, cx| {
        root.mission_workspace.update(cx, |workspace, _| {
            workspace.active_tab = MissionTab::Session("mission-shell".into());
        });
        root.visit_computer(other_computer(), window, cx)
    })
    .unwrap();
    wait_app(&mut cx, "local mission suspended while remote", |cx| {
        host.read_with(cx, |root, cx| {
            let workspace = root.mission_workspace.read(cx);
            root.remote_client.is_active() && !workspace.active && workspace.attached.is_empty()
        })
        .unwrap()
    });
    fake.viewed.lock().unwrap().clear();
    cx.simulate_system_notification_response(gpui::SystemNotificationResponse {
        tag: "mission-agent".into(),
        action_id: None,
    });
    wait_app(
        &mut cx,
        "new local window attached mission-agent and marked it viewed",
        |cx| {
            let local = cx.update(|cx| {
                cx.windows()
                    .into_iter()
                    .filter_map(|handle| handle.downcast::<NativeRoot>())
                    .find(|handle| handle.window_id() != host.window_id())
            });
            local.is_some_and(|local| {
                local
                    .read_with(cx, |root, cx| {
                        let workspace = root.mission_workspace.read(cx);
                        root.remote_client.is_local()
                            && root.route == AppRoute::Mission("test-mission".into())
                            && workspace.active
                            && !workspace.loading
                            && workspace.active_tab == MissionTab::Session("mission-agent".into())
                            && workspace.attached.contains_key("mission-agent")
                            && workspace.mission_terminal_interactive("mission-agent", cx)
                    })
                    .unwrap()
            }) && fake
                .viewed
                .lock()
                .unwrap()
                .iter()
                .any(|id| id == "mission-agent")
        },
    );
    host.read_with(&cx, |root, cx| {
        assert!(root.remote_client.is_active());
        assert!(!root.mission_workspace.read(cx).active);
    })
    .unwrap();
    let local_window = cx.update(|cx| {
        let windows = cx
            .windows()
            .into_iter()
            .filter_map(|handle| handle.downcast::<NativeRoot>())
            .collect::<Vec<_>>();
        assert_eq!(windows.len(), 2);
        windows
            .into_iter()
            .find(|handle| handle.window_id() != host.window_id())
            .unwrap()
    });
    assert_ne!(
        local_window
            .read_with(&cx, |root, _| root.window_label.clone())
            .unwrap(),
        host.read_with(&cx, |root, _| root.window_label.clone())
            .unwrap()
    );
    assert!(!fake
        .viewed
        .lock()
        .unwrap()
        .iter()
        .any(|id| id == "mission-shell"));
}

#[test]
fn local_mission_load_finishes_while_remote_and_attaches_only_on_return() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, host, fake) = suspended_mission_test();
    host.update(&mut cx, |root, window, cx| {
        root.open_mission("test-mission".into(), window, cx);
        assert!(root.mission_workspace.read(cx).loading);
        root.visit_computer(other_computer(), window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    host.read_with(&cx, |root, cx| {
        assert!(root.remote_client.is_active());
        let workspace = root.mission_workspace.read(cx);
        assert!(!workspace.active);
        assert!(!workspace.loading);
        assert!(workspace.mission.is_some());
        assert!(workspace.attached.is_empty());
    })
    .unwrap();
    fake.replay
        .lock()
        .unwrap()
        .push(local_mission_event("after-load"));
    host.update(&mut cx, |root, window, cx| {
        root.return_to_this_computer(window, cx)
    })
    .unwrap();
    cx.run_until_parked();
    host.read_with(&cx, |root, cx| {
        let workspace = root.mission_workspace.read(cx);
        assert!(workspace.active);
        assert!(!workspace.loading);
        assert!(workspace.attached.contains_key("mission-agent"));
        assert!(workspace.mission_terminal_interactive("mission-agent", cx));
        assert!(workspace
            .events
            .iter()
            .any(|event| event.id == "after-load"));
    })
    .unwrap();
}

struct MissionComposerInputTest {
    workspace: Entity<MissionWorkspace>,
    show_feed: bool,
    width: f32,
}

impl gpui::Render for MissionComposerInputTest {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(px(16. * self.workspace.read(cx).settings(cx).app_zoom));
        let feed = self.show_feed.then(|| {
            self.workspace
                .update(cx, |workspace, cx| workspace.render_mission_feed(cx))
        });
        div()
            .w(px(self.width))
            .h(px(700.))
            .flex()
            .flex_col()
            .children(feed)
            .child(self.workspace.update(cx, |workspace, cx| {
                workspace.render_mission_composer(window, cx)
            }))
    }
}

fn mission_workspace_test() -> (
    tempfile::TempDir,
    gpui::TestAppContext,
    gpui::WindowHandle<MissionComposerInputTest>,
    Entity<MissionWorkspace>,
) {
    use runner_daemon::{db, session, shell_path};
    use std::sync::RwLock;
    let temp = tempfile::tempdir().unwrap();
    let env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
    let discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
    let core = crate::test_support::core(
        Arc::new(db::open_fixture_pool(&temp.path().join("runner.db")).unwrap()),
        temp.path().to_owned(),
        session::SessionManager::new(
            env.clone(),
            discovery.clone(),
            Arc::new(session::pty_runtime::PtyRuntime::new()),
        ),
        env,
        discovery,
    );
    let mut cx = gpui::TestAppContext::single();
    let store = cx.new(|cx| {
        AppStore::new(
            core,
            None,
            None,
            temp.path().join("settings.json"),
            AppSettings::default(),
            None,
            cx,
        )
    });
    let host = cx.add_window(|window, cx| {
        window.set_rem_size(px(16.));
        let workspace = cx.new(|cx| {
            let mut workspace = MissionWorkspace::new(
                "composer-test".into(),
                gpui::WeakEntity::new_invalid(),
                store,
                window,
                cx,
            );
            workspace.roster = vec![runner_core::protocol::model::SlotWithRole {
                slot: runner_core::protocol::model::Slot {
                    id: "slot".into(),
                    crew_id: "crew".into(),
                    role_id: "role".into(),
                    slot_handle: "reviewer".into(),
                    position: 0,
                    lead: false,
                    runtime_override: None,
                    model_override: None,
                    effort_override: None,
                    codex_speed_override: None,
                    added_at: Utc::now(),
                },
                role: runner_core::protocol::model::Role {
                    id: "role".into(),
                    handle: "reviewer".into(),
                    display_name: "Reviewer".into(),
                    runtime: "codex".into(),
                    command: "codex".into(),
                    args: Vec::new(),
                    working_dir: None,
                    system_prompt: None,
                    env: Default::default(),
                    model: None,
                    effort: None,
                    codex_speed: None,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
            }];
            workspace
                .composer_input
                .read(cx)
                .focus_handle()
                .focus(window, cx);
            workspace
        });
        MissionComposerInputTest {
            workspace,
            show_feed: false,
            width: 600.,
        }
    });
    let workspace = host
        .read_with(&cx, |host, _| host.workspace.clone())
        .unwrap();
    (temp, cx, host, workspace)
}

#[test]
fn composer_tracks_keyboard_and_mouse_caret_moves_and_completes_without_resetting_input() {
    use gpui::{EntityInputHandler, MouseButton, VisualTestContext};
    let (_temp, cx, host, workspace) = mission_workspace_test();
    let input = workspace.read_with(&cx, |workspace, _| workspace.composer_input.clone());
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_input("please check @rev");
    let query = |visual: &VisualTestContext| {
        workspace.read_with(visual, |workspace, _| {
            crate::surfaces::mission_workspace::composer_model::mention_query(&workspace.composer)
                .map(str::to_owned)
        })
    };
    assert_eq!(query(&visual).as_deref(), Some("rev"));
    visual.simulate_keystrokes("left");
    assert_eq!(query(&visual), None);
    visual.simulate_keystrokes("right");
    assert_eq!(query(&visual).as_deref(), Some("rev"));
    visual.simulate_keystrokes("tab");
    input.read_with(&visual, |input, _| {
        assert_eq!(input.text(), "please check @reviewer ");
        assert_eq!(input.caret_offset(), input.text().len());
    });
    assert!(workspace.read_with(&visual, |workspace, _| workspace
        .composer
        .targets
        .is_empty()));
    visual.simulate_keystrokes(if cfg!(any(windows, target_os = "linux")) {
        "ctrl-z"
    } else {
        "cmd-z"
    });
    assert_eq!(
        input.read_with(&visual, |input, _| input.text().to_owned()),
        "please check @rev"
    );
    assert_eq!(query(&visual).as_deref(), Some("rev"));

    let start = visual.update(|window, cx| {
        input.update(cx, |input, cx| {
            input
                .bounds_for_range(0..0, gpui::Bounds::default(), window, cx)
                .unwrap()
                .origin
        })
    });
    visual.simulate_mouse_down(
        start + gpui::point(px(0.), px(4.)),
        MouseButton::Left,
        gpui::Modifiers::none(),
    );
    visual.simulate_mouse_up(
        start + gpui::point(px(0.), px(4.)),
        MouseButton::Left,
        gpui::Modifiers::none(),
    );
    assert_eq!(
        workspace.read_with(&visual, |workspace, _| workspace.composer.caret),
        0
    );
    assert_eq!(query(&visual), None);
    let roster = workspace.update(&mut visual, |workspace, cx| {
        let roster = std::mem::take(&mut workspace.roster);
        workspace.composer = Default::default();
        workspace
            .composer_input
            .update(cx, |input, cx| input.reset("", cx));
        cx.notify();
        roster
    });
    cx.run_until_parked();
    visual.simulate_input("cannot send without a role");
    assert_eq!(
        input.read_with(&visual, |input, _| input.text().to_owned()),
        ""
    );
    workspace.update(&mut visual, |workspace, cx| {
        workspace.roster = roster;
        cx.notify();
    });
    cx.run_until_parked();
    visual.simulate_input("worker joined");
    assert_eq!(
        input.read_with(&visual, |input, _| input.text().to_owned()),
        "worker joined"
    );
}

fn signal(signal_type: &str, payload: serde_json::Value) -> Event {
    Event {
        id: signal_type.into(),
        ts: Utc::now(),
        crew_id: Some("crew".into()),
        mission_id: "mission".into(),
        kind: EventKind::Signal,
        from: "system".into(),
        to: Default::default(),
        signal_type: Some(runner_daemon::model::SignalType::new(signal_type)),
        payload,
    }
}

#[test]
fn session_status_projection_reads_legacy_runner_status_rows() {
    let from = |handle: &str, event: Event| Event {
        from: handle.into(),
        ..event
    };
    let events = vec![
        from(
            "coder",
            signal("runner_status", serde_json::json!({ "state": "busy" })),
        ),
        from(
            "reviewer",
            signal("runner_status", serde_json::json!({ "state": "idle" })),
        ),
        from(
            "coder",
            signal("session_status", serde_json::json!({ "state": "idle" })),
        ),
        from(
            "reviewer",
            signal("ask_lead", serde_json::json!({ "state": "busy" })),
        ),
    ];

    let (statuses, observations) = state::project_session_statuses(&events);

    assert_eq!(
        statuses.get("coder"),
        Some(&runner_daemon::session::manager::SessionActivityState::Idle)
    );
    assert_eq!(
        statuses.get("reviewer"),
        Some(&runner_daemon::session::manager::SessionActivityState::Idle)
    );
    assert!(observations.is_empty());
}

#[test]
fn sidebar_and_mission_fills_follow_carbon_and_runner_light() {
    use crate::theme_snapshot::{assert_fill, ThemeGuard};
    use gpui::{TestAppContext, VisualTestContext};

    use runner_daemon::{db, session, shell_path};
    use std::sync::RwLock;

    let _theme = ThemeGuard::new();
    theme::set_active_variant(theme::ThemeVariant::Carbon);
    let temp = tempfile::tempdir().unwrap();
    let pool = Arc::new(db::open_fixture_pool(&temp.path().join("runner.db")).unwrap());
    pool.get()
            .unwrap()
            .execute_batch(
                "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('crew', 'Crew', '2026-09-06T00:00:00Z', '2026-09-06T00:00:00Z');
                 INSERT INTO missions (id, crew_id, title, status, started_at)
                 VALUES ('mission', 'crew', 'Original mission', 'running', '2026-09-06T00:00:00Z');",
            )
            .unwrap();
    let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
    let runtime_discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
    let core = crate::test_support::core(
        pool.clone(),
        temp.path().to_owned(),
        session::SessionManager::new(
            runtime_shell_env.clone(),
            runtime_discovery.clone(),
            Arc::new(session::pty_runtime::PtyRuntime::new()),
        ),
        runtime_shell_env,
        runtime_discovery,
    );

    let mut cx = TestAppContext::single();
    let store = cx.new(|cx| {
        AppStore::new(
            core.clone(),
            None,
            None,
            temp.path().join("settings.json"),
            AppSettings {
                app_theme: theme::ThemeIntent::Dark,
                dark_app_theme: theme::DarkTheme::CatppuccinMocha,
                ..AppSettings::default()
            },
            None,
            cx,
        )
    });
    cx.update(|cx| {
        cx.set_global(crate::GlobalAppStore(store.clone()));
        cx.set_global(crate::WindowLayoutCheckpoint::default());
        #[cfg(target_os = "macos")]
        let updater = cx.new(|cx| crate::Updater::new(false, cx));
        #[cfg(any(windows, target_os = "linux"))]
        let updater = cx.new(|cx| crate::Updater::new(false, temp.path().join("updates"), cx));
        cx.set_global(crate::GlobalUpdater(updater));
    });
    let bridge = cx.update(|cx| store.read(cx).bridge.clone());
    for id in ["direct", "slot"] {
        let events: Arc<dyn runner_daemon::session::manager::SessionEvents> =
            Arc::new(core.session_events());
        core.sessions
            .prepare_unlisted_terminal(id, (80, 24), &core.db, &events)
            .unwrap();
        let terminal = bridge.attach(id).unwrap();
        terminal.test_feed(1, b"ready");
    }
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "theme-snapshot".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = AppRoute::Mission("mission".into());
        root.mission_workspace.update(cx, |workspace, cx| {
            workspace.active = true;
            workspace.mission_id = Some("mission".into());
            workspace.mission =
                Some(runner_daemon::ops::mission::mission_get(&core, "mission").unwrap());
            workspace.composer.draft = "Ready to review".into();
            workspace
                .composer_input
                .update(cx, |input, cx| input.reset("Ready to review", cx));
        });
        root
    });
    assert_eq!(
        theme::active_variant(),
        theme::ThemeVariant::CatppuccinMocha
    );
    assert_eq!(
        bridge.session("direct").unwrap().palette(),
        runner_terminal::palette::CATPPUCCIN_MOCHA
    );
    assert_eq!(
        bridge.session("slot").unwrap().palette(),
        runner_terminal::palette::CATPPUCCIN_MOCHA
    );
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    for (intent, variant) in [
        (theme::ThemeIntent::Dark, theme::ThemeVariant::Carbon),
        (theme::ThemeIntent::Light, theme::ThemeVariant::RunnerLight),
    ] {
        store.update(&mut cx, |store, cx| {
            store.update_settings(
                |settings| {
                    settings.app_theme = intent;
                    settings.dark_app_theme = theme::DarkTheme::Runner;
                    true
                },
                false,
                cx,
            );
        });
        cx.run_until_parked();
        visual.refresh().unwrap();
        cx.run_until_parked();
        let colors = theme::colors_for(variant);
        assert_eq!(theme::active_variant(), variant);
        assert_fill(&mut visual, "APP_SIDEBAR", colors.sidebar);
        assert_fill(&mut visual, "MISSION_PANEL", None);
        assert_fill(&mut visual, "MISSION_HEADER_ROW", None);
        #[cfg(target_os = "macos")]
        {
            let header = visual.debug_bounds("MISSION_HEADER_ROW").unwrap();
            crate::platform_ui::take_titlebar_actions();
            visual.simulate_mouse_down(
                header.center(),
                gpui::MouseButton::Left,
                gpui::Modifiers::default(),
            );
            assert_eq!(
                crate::platform_ui::take_titlebar_actions(),
                [crate::platform_ui::TitlebarAction::Move]
            );
            visual.simulate_mouse_move(
                header.center() + gpui::point(px(4.), px(0.)),
                Some(gpui::MouseButton::Left),
                gpui::Modifiers::default(),
            );
            assert!(crate::platform_ui::take_titlebar_actions().is_empty());
            visual.simulate_mouse_up(
                header.center(),
                gpui::MouseButton::Left,
                gpui::Modifiers::default(),
            );
            visual.run_until_parked();
        }

        assert_fill(&mut visual, "MISSION_RAIL_HEADER", None);
        assert_fill(&mut visual, "MISSION_TABS", None);
        assert_fill(&mut visual, "MISSION_ACCENT", colors.accent);
        let palette = app_settings::terminal_palette(&AppSettings::default(), variant);
        assert_eq!(bridge.session("direct").unwrap().palette(), palette);
        assert_eq!(bridge.session("slot").unwrap().palette(), palette);
    }

    let archived_slot = SessionRow {
        live_title: None,
        session: runner_daemon::model::Session {
            id: "archived-slot".into(),
            mission_id: Some("mission".into()),
            role_id: "role".into(),
            slot_id: None,
            cwd: None,
            status: SessionStatus::Stopped,
            pid: None,
            started_at: None,
            stopped_at: None,
        },
        handle: "archived".into(),
        runtime: "codex".into(),
        agent_model: None,
        agent_effort: None,
        role_handle: String::new(),
        lead: false,
        agent_session_key: None,
    };
    {
        let conn = core.db.get().unwrap();
        conn.execute("INSERT INTO sessions(id, status, agent_runtime, archived_at) VALUES ('archived-slot', 'stopped', 'codex', '2026-09-14T00:00:00Z')", []).unwrap();
        runner_daemon::repo::session_attention::record_completion(&conn, "archived-slot", false, 1)
            .unwrap();
    }
    host.update(&mut cx, |root, window, cx| {
        root.mission_workspace.update(cx, |workspace, _| {
            workspace.sessions = vec![archived_slot.clone()];
            workspace.active_tab = MissionTab::Session("archived-slot".into());
            workspace.mission.as_mut().unwrap().archived_at = Some(Utc::now());
            workspace.session_observations.insert(
                "archived".into(),
                runner_daemon::session::status::AgentStatus {
                    lifecycle: runner_daemon::session::status::Lifecycle::Stopped,
                    unread_since: Some(1),
                    ..Default::default()
                },
            );
        });
        window.activate_window();
    })
    .unwrap();
    cx.run_until_parked();
    let mut attention_events = core.events.subscribe();
    for _ in 0..3 {
        host.update(&mut cx, |root, window, cx| {
            assert!(window.is_window_active());
            root.mission_workspace.update(cx, |workspace, cx| {
                drop(workspace.render_mission_terminal_pane(
                    crate::surfaces::front_page::model::MissionMemberDisplay::from(&archived_slot),
                    window,
                    cx,
                ));
                workspace.mark_active_session_viewed(window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
    }
    assert!(runner_daemon::repo::session_attention::any_unread(
        &core.db.get().unwrap(),
        &["archived-slot".into()]
    )
    .unwrap());
    while let Ok(event) = attention_events.try_recv() {
        assert_ne!(event.name, "chat/tab-attention-changed");
    }
    core.db
        .get()
        .unwrap()
        .execute(
            "UPDATE sessions SET archived_at = NULL WHERE id = 'archived-slot'",
            [],
        )
        .unwrap();
    host.update(&mut cx, |root, window, cx| {
        root.mission_workspace.update(cx, |workspace, _| {
            workspace.mission.as_mut().unwrap().archived_at = None;
        });
        root.sync_window_activation(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!runner_daemon::repo::session_attention::any_unread(
        &core.db.get().unwrap(),
        &["archived-slot".into()]
    )
    .unwrap());
}

#[test]
fn permissions_section_reads_the_recorded_mission_start_mode() {
    let goal = signal("mission_goal", serde_json::json!({ "text": "Ship it" }));
    for (recorded, shown) in [
        ("bypass", "bypass"),
        ("auto", "auto"),
        ("role-default", "role default"),
        ("runner-default", "role default"),
    ] {
        let start = signal(
            "mission_start",
            serde_json::json!({ "title": "t", "cwd": null, "permission_mode": recorded }),
        );
        assert_eq!(
            mission_permission_mode_label(&[start, goal.clone()]),
            Some(shown.to_owned()),
            "{recorded}"
        );
    }

    // Pre-feature missions recorded no key: no section.
    let legacy = signal(
        "mission_start",
        serde_json::json!({ "title": "t", "cwd": null }),
    );
    assert_eq!(mission_permission_mode_label(&[legacy, goal.clone()]), None);
    assert_eq!(mission_permission_mode_label(&[goal]), None);
    assert_eq!(mission_permission_mode_label(&[]), None);
}

#[test]
fn slot_overlay_precedence_matches_the_shipped_workspace() {
    assert_eq!(
        resolve_slot_overlay(
            true,
            Some(MissionTransitionKind::Resuming),
            SessionStatus::Stopped,
        ),
        SlotOverlayState::Archiving
    );
    assert_eq!(
        resolve_slot_overlay(
            false,
            Some(MissionTransitionKind::Resuming),
            SessionStatus::Stopped,
        ),
        SlotOverlayState::Resuming
    );
    assert_eq!(
        resolve_slot_overlay(
            false,
            Some(MissionTransitionKind::Starting),
            SessionStatus::Stopped,
        ),
        SlotOverlayState::Starting
    );
    assert_eq!(
        resolve_slot_overlay(false, None, SessionStatus::Crashed),
        SlotOverlayState::Stopped
    );
    assert_eq!(
        resolve_slot_overlay(false, None, SessionStatus::Running),
        SlotOverlayState::None
    );
}

#[test]
fn draft_recovery_follows_hook_status_when_a_return_leaves_the_session_busy() {
    use runner_core::protocol::status::{
        Activity, AgentObservation, AgentStatus, HumanInteraction, ObservationSource, TurnOutcome,
        WaitReason,
    };
    let status = |activity, source| AgentStatus {
        observation: AgentObservation {
            activity,
            source,
            outcome: Some(TurnOutcome::Interrupted),
            ..Default::default()
        },
        ..Default::default()
    };
    let busy = Some(&SessionActivityState::Busy);
    let idle = Some(&SessionActivityState::Idle);

    assert!(draft_recovery_available(
        busy,
        &status(Activity::Ready, ObservationSource::Hook)
    ));
    assert!(draft_recovery_available(
        busy,
        &status(Activity::Unavailable, ObservationSource::Hook)
    ));
    assert!(!draft_recovery_available(
        busy,
        &status(Activity::Working, ObservationSource::Hook)
    ));
    assert!(!draft_recovery_available(
        busy,
        &status(Activity::Idle, ObservationSource::Baseline)
    ));
    assert!(draft_recovery_available(
        idle,
        &status(Activity::Idle, ObservationSource::Baseline)
    ));

    let mut waiting = status(Activity::Ready, ObservationSource::Hook);
    waiting.observation.interactions.push(HumanInteraction {
        id: "ask".into(),
        reason: WaitReason::Answer,
        owners: Vec::new(),
        since: 0,
    });
    assert!(!draft_recovery_available(idle, &waiting));
}

#[test]
fn session_spawn_does_not_replace_an_existing_resume_transition() {
    assert_eq!(
        transition_to_begin_on_spawn(None),
        Some(MissionTransitionKind::Starting)
    );
    assert_eq!(
        transition_to_begin_on_spawn(Some(MissionTransitionKind::Resuming)),
        None
    );
    assert_eq!(
        transition_to_begin_on_spawn(Some(MissionTransitionKind::Starting)),
        None
    );
}

#[test]
fn concurrent_resume_errors_match_the_backend_contract() {
    assert!(is_concurrent_resume_error(
        "session abc is already being resumed"
    ));
    assert!(is_concurrent_resume_error(
        "session abc is already running — attach instead"
    ));
    assert!(!is_concurrent_resume_error("session abc failed to spawn"));
}

#[test]
fn measured_mission_slot_size_wins_over_cache_and_layout_estimate() {
    let cached = CachedTerminalSize {
        measured: (100, 30),
        layout_estimate: (80, 24),
    };
    assert_eq!(
        preferred_terminal_size(Some((120, 40)), Some(cached), (80, 24)),
        (120, 40)
    );
    assert_eq!(
        preferred_terminal_size(None, Some(cached), (80, 24)),
        (100, 30)
    );
    assert_eq!(
        preferred_terminal_size(None, Some(cached), (90, 28)),
        (90, 28)
    );
    assert_eq!(preferred_terminal_size(None, None, (80, 24)), (80, 24));
}

#[test]
fn mission_drawer_is_only_available_on_primary_active_rows() {
    assert!(mission_drawer_available(false, false));
    assert!(!mission_drawer_available(true, false));
    assert!(!mission_drawer_available(false, true));
    assert!(!mission_drawer_available(true, true));
}

#[test]
fn only_the_visible_active_drawer_shell_takes_focus_after_lifecycle_changes() {
    let mut layout = MissionLayout::default();
    layout.drawer.add("one".into());
    layout.drawer.add("two".into());

    assert!(!drawer_session_should_take_focus(&layout, "one"));
    assert!(drawer_session_should_take_focus(&layout, "two"));

    layout.drawer.set_open(false);
    assert!(!drawer_session_should_take_focus(&layout, "two"));
}

#[test]
fn mission_tab_shortcuts_cycle_feed_and_open_slots() {
    let tabs = vec![
        MissionTab::Feed,
        MissionTab::Session("coder".into()),
        MissionTab::Session("reviewer".into()),
    ];
    assert_eq!(
        mission_tab_in_direction(&tabs, &MissionTab::Feed, 1),
        Some(MissionTab::Session("coder".into()))
    );
    assert_eq!(
        mission_tab_in_direction(&tabs, &MissionTab::Feed, -1),
        Some(MissionTab::Session("reviewer".into()))
    );
    assert_eq!(
        mission_tab_in_direction(&tabs, &MissionTab::Session("reviewer".into()), 1),
        Some(MissionTab::Feed)
    );
    assert_eq!(
        mission_tab_in_direction(&tabs, &MissionTab::Session("closed".into()), -1),
        Some(MissionTab::Feed)
    );
}

#[test]
fn selecting_feed_forgets_the_remembered_session_so_reopening_restores_feed() {
    use gpui::{TestAppContext, WeakEntity};
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, cx| {
        store.update_settings(
            |settings| {
                settings
                    .last_mission_terminal_ids
                    .insert("mission".into(), "coder".into());
                settings
                    .last_mission_terminal_ids
                    .insert("other".into(), "reviewer".into());
                true
            },
            false,
            cx,
        );
    });
    let host = cx.add_window(|_, _| gpui::Empty);
    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    let visual = &mut visual;
    let workspace = visual.update(|window, cx| {
        cx.new(|cx| {
            let mut workspace = MissionWorkspace::new(
                "feed-memory".into(),
                WeakEntity::new_invalid(),
                store.clone(),
                window,
                cx,
            );
            workspace.mission_id = Some("mission".into());
            workspace.secondary = true;
            workspace
        })
    });
    let remembered = |visual: &mut gpui::VisualTestContext| {
        workspace.read_with(visual, |workspace, cx| {
            workspace.settings(cx).last_mission_terminal_ids.clone()
        })
    };

    workspace.update_in(visual, |workspace, window, cx| {
        workspace.select_mission_feed(window, cx)
    });
    assert_eq!(
        remembered(visual).get("mission").map(String::as_str),
        Some("coder"),
        "a secondary window's feed does not change the primary's memory"
    );

    workspace.update_in(visual, |workspace, window, cx| {
        workspace.secondary = false;
        workspace.select_mission_feed(window, cx)
    });
    assert_eq!(
        remembered(visual),
        [("other".to_owned(), "reviewer".to_owned())].into()
    );
}
#[test]
fn slot_rail_actions_match_status() {
    assert_eq!(
        slot_controls(SessionStatus::Running),
        [SessionControlKind::Stop, SessionControlKind::Restart]
    );
    for status in [SessionStatus::Stopped, SessionStatus::Crashed] {
        assert_eq!(
            slot_controls(status),
            [SessionControlKind::Resume, SessionControlKind::Restart]
        );
    }
}

#[test]
fn sidebar_archiving_shows_the_pill_instead_of_paused_actions() {
    use gpui::{TestAppContext, VisualTestContext};

    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store);
    });
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "archive-progress".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = AppRoute::Mission("test-mission".into());
        root.mission_workspace.update(cx, |workspace, _| {
            workspace.active = true;
            workspace.mission_id = Some("test-mission".into());
            workspace.mission =
                Some(runner_daemon::ops::mission::mission_get(&core, "test-mission").unwrap());
            workspace.roster =
                runner_daemon::ops::mission::mission_roster(&core, "test-mission").unwrap();
            workspace.sessions =
                runner_daemon::ops::session::session_list(&core, "test-mission").unwrap();
            for session in &mut workspace.sessions {
                session.session.status = SessionStatus::Stopped;
            }
        });
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1200.), px(900.)));

    for tab in [
        MissionTab::Feed,
        MissionTab::Session("mission-agent".into()),
    ] {
        host.update(&mut visual, |root, _, cx| {
            root.mission_workspace.update(cx, |workspace, cx| {
                workspace.active_tab = tab.clone();
                cx.notify();
            });
        })
        .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("SESSION_ENDED_CARD").is_some());
        assert!(visual.debug_bounds("SESSION_ARCHIVING").is_none());

        host.update(&mut visual, |root, _, cx| {
            root.set_sidebar_mission_archiving("other-mission".into(), true, cx);
        })
        .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("SESSION_ENDED_CARD").is_some());

        host.update(&mut visual, |root, _, cx| {
            root.set_sidebar_mission_archiving("test-mission".into(), true, cx);
        })
        .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("SESSION_ENDED_CARD").is_none());
        assert!(visual.debug_bounds("SESSION_ARCHIVING").is_some());

        host.update(&mut visual, |root, _, cx| {
            root.set_sidebar_mission_archiving("test-mission".into(), false, cx);
        })
        .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("SESSION_ENDED_CARD").is_some());
        assert!(visual.debug_bounds("SESSION_ARCHIVING").is_none());
    }
    host.update(&mut visual, |root, window, cx| {
        root.set_sidebar_mission_archiving("test-mission".into(), true, cx);
        root.set_route(AppRoute::Roles, cx);
        root.open_mission("test-mission".into(), window, cx);
        assert!(root.mission_workspace.read(cx).archiving);
    })
    .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SESSION_ENDED_CARD").is_none());
    assert!(visual.debug_bounds("SESSION_ARCHIVING").is_some());
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn cached_mission_header_tabs_and_feed_span_the_content_column() {
    use gpui::{TestAppContext, VisualTestContext};
    use runner_daemon::{db, session, shell_path};
    use std::sync::RwLock;

    let temp = tempfile::tempdir().unwrap();
    let pool = Arc::new(db::open_fixture_pool(&temp.path().join("runner.db")).unwrap());
    pool.get()
        .unwrap()
        .execute_batch(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('crew', 'Crew', '2026-10-09T00:00:00Z', '2026-10-09T00:00:00Z');
             INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('mission', 'crew', '#839 sidebar footer', 'running', '2026-10-09T00:00:00Z');",
        )
        .unwrap();
    let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
    let runtime_discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
    let core = crate::test_support::core(
        pool,
        temp.path().to_owned(),
        session::SessionManager::new(
            runtime_shell_env.clone(),
            runtime_discovery.clone(),
            Arc::new(session::pty_runtime::PtyRuntime::new()),
        ),
        runtime_shell_env,
        runtime_discovery,
    );
    let mut cx = TestAppContext::single();
    let store = cx.new(|cx| {
        AppStore::new(
            core.clone(),
            None,
            None,
            temp.path().join("settings.json"),
            AppSettings::default(),
            None,
            cx,
        )
    });
    cx.update(|cx| {
        cx.set_global(crate::GlobalAppStore(store.clone()));
        cx.set_global(crate::WindowLayoutCheckpoint::default());
        #[cfg(target_os = "macos")]
        let updater = cx.new(|cx| crate::Updater::new(false, cx));
        #[cfg(any(windows, target_os = "linux"))]
        let updater = cx.new(|cx| crate::Updater::new(false, temp.path().join("updates"), cx));
        cx.set_global(crate::GlobalUpdater(updater));
    });
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "mission-header-layout".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = AppRoute::Mission("mission".into());
        root.mission_workspace.update(cx, |workspace, _| {
            workspace.active = true;
            workspace.mission_id = Some("mission".into());
            workspace.mission =
                Some(runner_daemon::ops::mission::mission_get(&core, "mission").unwrap());
        });
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    for width in [1200., 1600.] {
        visual.simulate_resize(size(px(width), px(900.)));
        cx.run_until_parked();
        let column = visual.debug_bounds("MISSION_CONTENT_COLUMN").unwrap();
        for selector in ["MISSION_HEADER_ROW", "MISSION_TABS", "MISSION_FEED"] {
            let bounds = visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing {selector}"));
            assert_eq!(
                (bounds.left(), bounds.size.width),
                (column.left(), column.size.width),
                "{selector} at a {width}px window"
            );
        }
    }

    assert!(visual.debug_bounds("MISSION_NO_EVENTS").is_some());
    assert!(visual.debug_bounds("MISSION_NO_ROLES").is_some());
    assert!(visual.debug_bounds("MISSION_RAIL_NO_ROLES").is_some());
    assert!(visual.debug_bounds("MISSION_RAIL_ADD_ROLE").is_some());
    assert!(visual.debug_bounds("SESSION_ENDED_CARD").is_none());

    let handles = [
        "coder",
        "reviewer-with-a-long-handle",
        "qa-with-a-long-handle",
    ];
    host.update(&mut visual, |root, _, cx| {
        root.mission_workspace.update(cx, |workspace, cx| {
            workspace.sessions = handles
                .iter()
                .map(|handle| SessionRow {
                    session: runner_core::protocol::model::Session {
                        id: format!("slot-{handle}"),
                        mission_id: Some("mission".into()),
                        role_id: "role".into(),
                        slot_id: None,
                        cwd: None,
                        status: SessionStatus::Running,
                        pid: None,
                        started_at: None,
                        stopped_at: None,
                    },
                    handle: (*handle).into(),
                    role_handle: String::new(),
                    lead: false,
                    runtime: "codex".into(),
                    agent_model: None,
                    agent_effort: None,
                    live_title: None,
                    agent_session_key: None,
                })
                .collect();
            workspace.roster = vec![runner_core::protocol::model::SlotWithRole {
                slot: runner_core::protocol::model::Slot {
                    id: "slot-coder".into(),
                    crew_id: "crew".into(),
                    role_id: "role".into(),
                    slot_handle: "coder".into(),
                    position: 0,
                    lead: false,
                    runtime_override: None,
                    model_override: None,
                    effort_override: None,
                    codex_speed_override: None,
                    added_at: Utc::now(),
                },
                role: runner_core::protocol::model::Role {
                    id: "role".into(),
                    handle: "coder".into(),
                    display_name: "Coder".into(),
                    runtime: "codex".into(),
                    command: "codex".into(),
                    args: Vec::new(),
                    working_dir: None,
                    system_prompt: None,
                    env: Default::default(),
                    model: None,
                    effort: None,
                    codex_speed: None,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
            }];
            workspace.events = vec![signal(
                "mission_goal",
                serde_json::json!({"text": "Ship it"}),
            )];
            workspace.rebuild_event_projection();
            workspace.open_tabs = handles
                .iter()
                .map(|handle| format!("slot-{handle}"))
                .collect();
            cx.notify();
        });
    })
    .unwrap();
    visual.simulate_resize(size(px(1100.), px(900.)));
    cx.run_until_parked();
    let strip = visual.debug_bounds("MISSION_TABS").unwrap();
    for selector in [
        "MISSION_TAB slot-coder",
        "MISSION_TAB slot-reviewer-with-a-long-handle",
        "MISSION_TAB slot-qa-with-a-long-handle",
    ] {
        let tab = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        assert!(
            tab.left() >= strip.left() && tab.right() <= strip.right(),
            "{selector} {tab:?} escapes the tab strip {strip:?}"
        );
    }
    assert!(visual
        .debug_bounds("MISSION_CARD_LEAD slot-coder")
        .is_none());
    assert!(visual
        .debug_bounds("MISSION_MESSAGE_TARGET coder")
        .is_none());
    host.update(&mut visual, |root, window, cx| {
        root.mission_workspace.update(cx, |workspace, cx| {
            workspace.sessions[0].lead = true;
            workspace.roster[0].slot.lead = true;
            workspace.request_remove_role("slot-coder", window, cx);
            assert_eq!(workspace.remove_confirm.as_deref(), Some("slot-coder"));
            workspace.remove_confirm = None;
            cx.notify();
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert!(visual
        .debug_bounds("MISSION_CARD_LEAD slot-coder")
        .is_none());
    assert!(visual
        .debug_bounds("MISSION_MESSAGE_TARGET coder")
        .is_none());
    host.read_with(&cx, |root, cx| {
        let workspace = root.mission_workspace.read(cx);
        assert_eq!(
            workspace.composer_input.read(cx).placeholder(),
            "Message everyone — @handle to address one"
        );
    })
    .unwrap();
}

struct MissionRailLayoutTest {
    workspace: Entity<MissionWorkspace>,
    width: Option<f32>,
}

#[cfg(unix)]
#[test]
fn mission_rail_scrolls_long_roster_and_goal_above_corner_padding() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext};
    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store)
    });
    let mission = store.read_with(&cx, |store, _| {
        runner_daemon::ops::mission::mission_get(&store.test_core, "test-mission").unwrap()
    });
    let host = cx.add_window(|window, cx| {
        let workspace = cx.new(|cx| {
            let mut workspace = MissionWorkspace::new(
                "rail-scroll".into(),
                WeakEntity::new_invalid(),
                store,
                window,
                cx,
            );
            workspace.mission = Some(mission);
            workspace.goal =
                Some("A long goal that must remain reachable in a short window.\n".repeat(40));
            workspace.sessions = (0..20)
                .map(|index| SessionRow {
                    session: runner_core::protocol::model::Session {
                        id: format!("slot-{index}"),
                        mission_id: Some("test-mission".into()),
                        role_id: "test-shell-role".into(),
                        slot_id: None,
                        cwd: None,
                        status: SessionStatus::Stopped,
                        pid: None,
                        started_at: None,
                        stopped_at: None,
                    },
                    handle: format!("worker-{index}"),
                    role_handle: String::new(),
                    lead: index == 0,
                    runtime: "shell".into(),
                    agent_model: None,
                    agent_effort: None,
                    live_title: None,
                    agent_session_key: None,
                })
                .collect();
            workspace
        });
        MissionRailLayoutTest {
            workspace,
            width: None,
        }
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(960.), px(300.)));
    visual.run_until_parked();
    let (view, viewport_selector, final_selector) = (
        PanelViewId::Sessions,
        "MISSION_ROLES_SCROLL",
        "MISSION_CARD slot-19",
    );
    host.update(&mut visual, |host, _, cx| {
        host.workspace
            .update(cx, |workspace, _| workspace.rail_surfaces.select(view));
        cx.notify();
    })
    .unwrap();
    visual.run_until_parked();
    let rail = visual.debug_bounds("MISSION_RAIL").unwrap();
    let viewport = visual.debug_bounds(viewport_selector).unwrap();
    assert_eq!(viewport.bottom(), rail.bottom() - px(9.));
    let last = visual.debug_bounds(final_selector).unwrap();
    assert!(
        last.bottom() > viewport.bottom(),
        "{view:?}: {last:?} {viewport:?}"
    );
    visual.simulate_event(ScrollEvent {
        position: viewport.center(),
        delta: ScrollDelta::Lines(point(0., -1000.)),
        ..Default::default()
    });
    visual.run_until_parked();
    let last = visual.debug_bounds(final_selector).unwrap();
    assert!(
        last.top() >= viewport.top() && last.bottom() <= viewport.bottom(),
        "{view:?}: {last:?} {viewport:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_rail_widened_by_an_open_file_lays_the_roster_two_per_row() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{TestAppContext, VisualTestContext};
    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store)
    });
    let mission = store.read_with(&cx, |store, _| {
        runner_daemon::ops::mission::mission_get(&store.test_core, "test-mission").unwrap()
    });
    let host = cx.add_window(|window, cx| {
        let workspace = cx.new(|cx| {
            let mut workspace = MissionWorkspace::new(
                "rail-two-up".into(),
                WeakEntity::new_invalid(),
                store,
                window,
                cx,
            );
            workspace.mission = Some(mission);
            workspace.sessions = (0..3)
                .map(|index| SessionRow {
                    session: runner_core::protocol::model::Session {
                        id: format!("slot-{index}"),
                        mission_id: Some("test-mission".into()),
                        role_id: "test-shell-role".into(),
                        slot_id: None,
                        cwd: None,
                        status: SessionStatus::Stopped,
                        pid: None,
                        started_at: None,
                        stopped_at: None,
                    },
                    handle: format!("worker-{index}"),
                    role_handle: String::new(),
                    lead: index == 0,
                    runtime: "shell".into(),
                    agent_model: None,
                    agent_effort: None,
                    live_title: None,
                    agent_session_key: None,
                })
                .collect();
            workspace
        });
        MissionRailLayoutTest {
            workspace,
            width: None,
        }
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(2400.), px(900.)));
    for zoom in [1., 1.1, 1.2, 1.3] {
        for width in [560., 607.3, 640., 680., 733.7] {
            host.update(&mut visual, |host, window, cx| {
                window.set_rem_size(px(16. * zoom));
                host.width = Some(width);
                cx.notify();
            })
            .unwrap();
            visual.run_until_parked();
            let [first, second, third] = [0, 1, 2].map(|index| {
                visual
                    .debug_bounds(format!("MISSION_CARD slot-{index}").leak())
                    .unwrap()
            });
            let rail = visual.debug_bounds("MISSION_RAIL").unwrap();
            let at = format!("zoom {zoom}, width {width}: {first:?} {second:?} {third:?}");
            assert_eq!(first.top(), second.top(), "{at}");
            assert!(second.left() > first.right(), "{at}");
            assert!(second.right() <= rail.right(), "{at}");
            assert!(third.top() > first.bottom(), "{at}");
            assert_eq!(third.left(), first.left(), "{at}");
            assert_eq!(third.size.width, first.size.width, "{at}");
        }
    }
}

fn rail_header_window(
    cx: &mut gpui::TestAppContext,
    temp: &tempfile::TempDir,
) -> (
    Entity<crate::app_store::AppStore>,
    gpui::WindowHandle<MissionRailLayoutTest>,
) {
    let store = crate::app_store::test_lifecycle_store(cx, temp.path());
    store.update(cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store);
        store
            .test_core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE missions SET cwd = '/tmp/panel-mission' WHERE id = 'test-mission'",
                [],
            )
            .unwrap();
    });
    let mission = store.read_with(cx, |store, _| {
        runner_daemon::ops::mission::mission_get(&store.test_core, "test-mission").unwrap()
    });
    let host = cx.add_window(|window, cx| {
        let workspace = cx.new(|cx| {
            let mut workspace = MissionWorkspace::new(
                "rail-header".into(),
                WeakEntity::new_invalid(),
                store.clone(),
                window,
                cx,
            );
            workspace.mission = Some(mission);
            workspace
        });
        MissionRailLayoutTest {
            workspace,
            width: None,
        }
    });
    (store, host)
}

#[test]
fn local_missions_keep_separate_rail_surfaces_and_collapse_choices() {
    use crate::surfaces::side_panel::{self, PanelSelection, PanelSurface};

    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let (store, host) = rail_header_window(&mut cx, &temp);
    let workspace = host
        .read_with(&cx, |host, _| host.workspace.clone())
        .unwrap();
    workspace.update(&mut cx, |workspace, cx| {
        workspace.prepare_mission("first".into());
        workspace.rail_surfaces.select(PanelViewId::Files);
        workspace.set_rail_open(true, cx);

        workspace.prepare_mission("second".into());
        assert_eq!(workspace.rail_surfaces.selection, PanelSelection::default());
        assert!(
            workspace.rail_is_open(cx),
            "a new wide mission defaults open independently"
        );
        workspace.rail_surfaces.select(PanelViewId::Changes);
        workspace.set_rail_open(false, cx);

        workspace.prepare_mission("first".into());
        assert!(workspace.rail_is_open(cx));
        assert_eq!(
            side_panel::shown(
                PanelSurface::Mission,
                workspace.settings(cx),
                &workspace.rail_surfaces.selection
            ),
            Some(PanelViewId::Files)
        );

        workspace.prepare_mission("second".into());
        assert!(!workspace.rail_is_open(cx));
        assert_eq!(
            side_panel::shown(
                PanelSurface::Mission,
                workspace.settings(cx),
                &workspace.rail_surfaces.selection
            ),
            Some(PanelViewId::Changes)
        );

        // Collapsing "second" never mutates future missions (#1403 per-tab isolation).
        workspace.prepare_mission("new".into());
        assert!(workspace.rail_is_open(cx));
        assert_eq!(workspace.rail_surfaces.selection, PanelSelection::default());
        workspace.prepare_mission("second".into());
        workspace.rail_surfaces.select(PanelViewId::Files);
    });
    store.update(&mut cx, |store, cx| {
        store.update_settings(
            |settings| {
                settings.files_view = false;
                true
            },
            false,
            cx,
        );
    });
    cx.run_until_parked();
    workspace.update(&mut cx, |workspace, cx| {
        assert_eq!(
            side_panel::shown(
                PanelSurface::Mission,
                workspace.settings(cx),
                &workspace.rail_surfaces.selection
            ),
            Some(PanelViewId::Sessions)
        );
        workspace.prepare_mission("first".into());
        assert_eq!(
            side_panel::shown(
                PanelSurface::Mission,
                workspace.settings(cx),
                &workspace.rail_surfaces.selection
            ),
            Some(PanelViewId::Sessions)
        );
        assert!(
            workspace.rail_is_open(cx),
            "Settings preserves each mission's collapse choice"
        );
    });
}

#[test]
fn mission_rail_toggles_remain_per_mission_without_mutating_settings() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let (store, host) = rail_header_window(&mut cx, &temp);
    let workspace = host
        .read_with(&cx, |host, _| host.workspace.clone())
        .unwrap();
    workspace.update(&mut cx, |workspace, cx| {
        let initial_rev = store.read(cx).revisions.settings;
        workspace.prepare_mission("open".into());
        workspace.set_rail_open(true, cx);
        workspace.prepare_mission("collapsed".into());
        workspace.set_rail_open(false, cx);
        assert_eq!(
            store.read(cx).revisions.settings,
            initial_rev,
            "toggling one mission's rail never mutates global settings"
        );

        workspace.prepare_mission("open".into());
        assert!(workspace.rail_is_open(cx));
        workspace.set_rail_open(false, cx);
        assert!(!workspace.rail_is_open(cx));

        workspace.prepare_mission("fresh".into());
        assert!(
            workspace.rail_is_open(cx),
            "collapsing earlier missions leaves a new mission open"
        );
        workspace.prepare_mission("collapsed".into());
        assert!(!workspace.rail_is_open(cx));
        workspace.set_rail_open(true, cx);
        assert!(workspace.rail_is_open(cx));
    });
}

#[test]
fn mission_terminal_estimates_follow_the_shown_missions_rail() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let (_store, host) = rail_header_window(&mut cx, &temp);
    host.update(&mut cx, |host, window, cx| {
        host.workspace.update(cx, |workspace, cx| {
            workspace.prepare_mission("collapsed".into());
            workspace.set_rail_open(false, cx);
            let collapsed = workspace.estimated_mission_terminal_size(window, cx);
            let collapsed_drawer = workspace.estimated_mission_drawer_terminal_size(window, cx);
            workspace.prepare_mission("open".into());
            workspace.set_rail_open(true, cx);
            let open = workspace.estimated_mission_terminal_size(window, cx);
            let open_drawer = workspace.estimated_mission_drawer_terminal_size(window, cx);
            assert!(open.0 < collapsed.0);
            assert!(open_drawer.0 < collapsed_drawer.0);

            workspace.prepare_mission("collapsed".into());
            assert_eq!(
                workspace.estimated_mission_terminal_size(window, cx),
                collapsed
            );
            assert_eq!(
                workspace.estimated_mission_drawer_terminal_size(window, cx),
                collapsed_drawer
            );
            workspace.set_rail_open(false, cx);
            workspace.prepare_mission("open".into());
            assert_eq!(workspace.estimated_mission_terminal_size(window, cx), open);
            assert_eq!(
                workspace.estimated_mission_drawer_terminal_size(window, cx),
                open_drawer
            );
        });
    })
    .unwrap();
}

#[test]
fn mission_panel_activity_bar_keeps_sessions_first_and_switches_files_and_changes() {
    use crate::surfaces::side_panel::{PanelContext, PanelSubject};
    use crate::theme_snapshot::{assert_fill_color, ThemeGuard};
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, host) = rail_header_window(&mut cx, &temp);
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(960.), px(600.)));
    visual.run_until_parked();
    let click = |visual: &mut VisualTestContext, selector: &'static str| {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        visual.simulate_click(bounds.center(), Modifiers::default());
        visual.run_until_parked();
    };
    // Sessions first and active by default; Files and Changes icons beside it; no `+` or Info.
    let rail = visual.debug_bounds("MISSION_RAIL").unwrap();
    assert!(visual
        .debug_bounds("PANEL_ACTIVITY Sessions active")
        .is_some());
    let sessions = visual.debug_bounds("PANEL_ACTIVITY Sessions").unwrap();
    let files = visual.debug_bounds("PANEL_ACTIVITY Files").unwrap();
    let changes = visual.debug_bounds("PANEL_ACTIVITY Changes").unwrap();
    assert!(
        sessions.left() - rail.left() <= px(16.),
        "{sessions:?} {rail:?}"
    );
    assert_eq!(sessions.size, gpui::size(px(28.), px(28.)));
    assert!(sessions.right() < files.left());
    assert!(files.right() < changes.left());
    assert!(visual.debug_bounds("PANEL_ACTIVITY_BAR Sessions").is_some());
    assert!(visual.debug_bounds("PANEL_ADD_SURFACE").is_none());
    assert!(visual.debug_bounds("PANEL_TAB Info").is_none());
    assert!(visual.debug_bounds("MISSION_ROLES_SCROLL").is_some());
    for variant in [
        theme::ThemeVariant::Carbon,
        theme::ThemeVariant::RunnerLight,
    ] {
        theme::set_active_variant(variant);
        visual.refresh().unwrap();
        visual.run_until_parked();
        assert_fill_color(
            &mut visual,
            "PANEL_ACTIVITY Sessions",
            Some(theme::raised()),
        );
        assert_fill_color(&mut visual, "PANEL_ACTIVITY Files", None);
    }

    // One click switches to Files; one click switches to Changes; one click returns to Sessions.
    click(&mut visual, "PANEL_ACTIVITY Files");
    let context = PanelContext {
        subject: PanelSubject::Mission("test-mission".into()),
        surface: crate::surfaces::side_panel::PanelSurface::Mission,
        cwd: Some("/tmp/panel-mission".into()),
        project: None,
        remote: false,
        following: None,
    };
    assert!(visual
        .debug_bounds(format!("FILES_VIEW {context:?}").leak())
        .is_some());
    assert!(visual.debug_bounds("MISSION_ROLES_SCROLL").is_none());
    assert!(visual.debug_bounds("PANEL_ACTIVITY_BAR Files").is_some());
    for variant in [
        theme::ThemeVariant::Carbon,
        theme::ThemeVariant::RunnerLight,
    ] {
        theme::set_active_variant(variant);
        visual.refresh().unwrap();
        visual.run_until_parked();
        assert_fill_color(&mut visual, "PANEL_ACTIVITY Sessions", None);
        assert_fill_color(&mut visual, "PANEL_ACTIVITY Files", Some(theme::raised()));
    }

    click(&mut visual, "PANEL_ACTIVITY Changes");
    assert!(visual
        .debug_bounds(format!("CHANGES_VIEW {context:?}").leak())
        .is_some());
    assert!(visual.debug_bounds("PANEL_ACTIVITY_BAR Changes").is_some());

    click(&mut visual, "PANEL_ACTIVITY Sessions");
    assert!(visual.debug_bounds("MISSION_ROLES_SCROLL").is_some());

    // Settings → Side panel turning Files and Changes off leaves Sessions alone.
    click(&mut visual, "PANEL_ACTIVITY Files");
    store.update(&mut visual, |store, cx| {
        store.update_settings(
            |settings| {
                settings.files_view = false;
                settings.changes_view = false;
                true
            },
            false,
            cx,
        );
    });
    visual.refresh().unwrap();
    visual.run_until_parked();
    assert!(visual
        .debug_bounds("PANEL_ACTIVITY Sessions active")
        .is_some());
    assert!(visual.debug_bounds("PANEL_ACTIVITY Files").is_none());
    assert!(visual.debug_bounds("PANEL_ACTIVITY Changes").is_none());
    assert!(visual.debug_bounds("MISSION_ROLES_SCROLL").is_some());

    store.update(&mut visual, |store, cx| {
        store.update_settings(
            |settings| {
                settings.files_view = true;
                settings.changes_view = true;
                true
            },
            false,
            cx,
        );
    });
    visual.refresh().unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("PANEL_ACTIVITY Files").is_some());
    assert!(visual.debug_bounds("PANEL_ACTIVITY Changes").is_some());
}

#[test]
fn mission_panel_activity_bar_fits_in_a_narrow_rail_in_both_locales() {
    use crate::i18n::{testing::with_locale, Locale};
    use crate::surfaces::side_panel::PanelViewId;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (_store, host) = rail_header_window(&mut cx, &temp);
            let mut visual = VisualTestContext::from_window(host.into(), &cx);
            visual.simulate_resize(size(px(960.), px(600.)));
            host.update(&mut visual, |host, _, cx| {
                host.workspace.update(cx, |workspace, _| {
                    workspace.rail_surfaces.select(PanelViewId::Files);
                });
                cx.notify();
            })
            .unwrap();
            for width in [crate::app_settings::MISSION_RAIL_DEFAULT, 200.] {
                host.update(&mut visual, |host, _, cx| {
                    host.width = Some(width);
                    cx.notify();
                })
                .unwrap();
                visual.run_until_parked();
                let rail = visual.debug_bounds("MISSION_RAIL").unwrap();
                let sessions = visual.debug_bounds("PANEL_ACTIVITY Sessions").unwrap();
                let files = visual.debug_bounds("PANEL_ACTIVITY Files active").unwrap();
                let changes = visual.debug_bounds("PANEL_ACTIVITY Changes").unwrap();
                let collapse = visual.debug_bounds("PANEL_COLLAPSE").unwrap();
                assert!(sessions.left() >= rail.left(), "{locale:?} {width}");
                assert!(sessions.right() < files.left(), "{locale:?} {width}");
                assert!(files.right() < changes.left(), "{locale:?} {width}");
                assert!(changes.right() <= collapse.left(), "{locale:?} {width}");
                assert!(
                    collapse.right() <= rail.right(),
                    "{locale:?} {width}: {collapse:?} {rail:?}"
                );
                visual.simulate_click(sessions.center(), Modifiers::default());
                visual.run_until_parked();
                assert!(visual.debug_bounds("MISSION_ROLES_SCROLL").is_some());
                let files = visual.debug_bounds("PANEL_ACTIVITY Files").unwrap();
                visual.simulate_click(files.center(), Modifiers::default());
                visual.run_until_parked();
                assert!(visual.debug_bounds("PANEL_ACTIVITY Files active").is_some());
            }
        });
    }
}

impl gpui::Render for MissionRailLayoutTest {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rail = self.workspace.update(cx, |workspace, cx| {
            let layout = match self.width {
                Some(width) => crate::surfaces::side_panel::PanelLayout {
                    width,
                    widened: None,
                    presentation: crate::surfaces::side_panel::PanelPresentation::Docked,
                    reserved_docked_width: 0.,
                },
                None => workspace.rail_layout(cx),
            };
            workspace.render_mission_rail(layout, layout.width, 1., true, window, cx)
        });
        let handle_text = |selector: &'static str, text: &'static str| {
            div()
                .debug_selector(|| selector.into())
                .flex_none()
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_size(theme::text_body())
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(text)
        };
        div().size_full().flex().child(rail).child(
            div()
                .flex()
                .flex_col()
                .items_start()
                .child(handle_text(
                    "RAIL_FULL_HANDLE",
                    "@abcdefghijklmnopqrstuvwxyz012345",
                ))
                .child(handle_text("RAIL_VISIBLE_PREFIX", "@a…"))
                .child(handle_text("RAIL_SHORT_HANDLE", "@qa"))
                .child(handle_text("RAIL_SHORT_LEAD_HANDLE", "@dev"))
                .child(handle_text("RAIL_SHORTEST_LEAD_HANDLE", "@a"))
                .child(
                    div()
                        .debug_selector(|| "RAIL_KEY_PREFIX".into())
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_caption())
                        .line_height(gpui::rems(14. / 16.))
                        .child("01…"),
                )
                .child(
                    div()
                        .debug_selector(|| "RAIL_TWO_LINE_KEY".into())
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_caption())
                        .line_height(gpui::rems(14. / 16.))
                        .child("01a119c9\n-4714-70"),
                )
                .child(
                    div()
                        .debug_selector(|| "RAIL_FULL_RUNTIME".into())
                        .flex_none()
                        .text_size(theme::text_meta())
                        .line_height(gpui::rems(14. / 16.))
                        .child("Codex · a-very-long-model-name-that-must-truncate · Extra high"),
                )
                .child(
                    div()
                        .debug_selector(|| "RAIL_TWO_LINE_MISSION_ID".into())
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_meta())
                        .child("01M4CWJGAR56H\nCBK0RNRQS8DV0"),
                ),
        )
    }
}

#[test]
fn mission_rail_long_values_truncate_without_overlapping_fixed_controls() {
    use gpui::{TestAppContext, VisualTestContext};
    use runner_daemon::{db, session, shell_path};
    use std::sync::RwLock;

    let session_key = "01a119c9-4714-7091-8b75-81cec4ff360c";
    for width in [
        app_settings::MISSION_RAIL_MIN,
        208.,
        220.,
        224.,
        225.,
        239.,
        240.,
        app_settings::MISSION_RAIL_DEFAULT,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let pool = Arc::new(db::open_fixture_pool(&temp.path().join("runner.db")).unwrap());
        pool.get()
            .unwrap()
            .execute_batch(
                "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('crew', 'Crew', '2026-10-08T00:00:00Z', '2026-10-08T00:00:00Z');
             INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('mission', 'crew', 'Rail test', 'running', '2026-10-08T00:00:00Z');",
            )
            .unwrap();
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = crate::test_support::core(
            pool,
            temp.path().to_owned(),
            session::SessionManager::new(
                runtime_shell_env.clone(),
                runtime_discovery.clone(),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            runtime_shell_env,
            runtime_discovery,
        );
        let mut cx = TestAppContext::single();
        let store = cx.new(|cx| {
            AppStore::new(
                core.clone(),
                None,
                None,
                temp.path().join("settings.json"),
                AppSettings {
                    mission_rail_width: width,
                    ..AppSettings::default()
                },
                None,
                cx,
            )
        });
        let host = cx.add_window(|window, cx| {
            window.set_rem_size(px(16.));
            let workspace = cx.new(|cx| {
                let mut workspace = MissionWorkspace::new(
                    "rail-test".into(),
                    WeakEntity::new_invalid(),
                    store,
                    window,
                    cx,
                );
                workspace.mission =
                    Some(runner_daemon::ops::mission::mission_get(&core, "mission").unwrap());
                workspace.sessions = [
                    ("lead", "abcdefghijklmnopqrstuvwxyz012345", true),
                    ("worker", "abcdefghijklmnopqrstuvwxyz012346", false),
                    ("short", "qa", false),
                ]
                .into_iter()
                .map(|(id, handle, lead)| SessionRow {
                    session: runner_core::protocol::model::Session {
                        id: id.into(),
                        mission_id: Some("mission".into()),
                        role_id: "role".into(),
                        slot_id: None,
                        cwd: None,
                        status: SessionStatus::Running,
                        pid: None,
                        started_at: None,
                        stopped_at: None,
                    },
                    handle: handle.into(),
                    role_handle: "role".into(),
                    lead,
                    runtime: "codex".into(),
                    agent_model: (id != "short")
                        .then(|| "a-very-long-model-name-that-must-truncate".into()),
                    agent_effort: (id != "short").then(|| "xhigh".into()),
                    live_title: None,
                    agent_session_key: (id != "short").then(|| session_key.into()),
                })
                .collect();
                workspace.sync_mission_copy_entities(cx);
                assert_eq!(
                    workspace.session_title_tooltip(
                        &crate::surfaces::front_page::model::MissionMemberDisplay::from(
                            &workspace.sessions[0]
                        ),
                        cx
                    ),
                    "@abcdefghijklmnopqrstuvwxyz012345"
                );
                workspace
            });
            MissionRailLayoutTest {
                workspace,
                width: None,
            }
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(960.), px(600.)));
        cx.run_until_parked();
        let full = visual.debug_bounds("RAIL_FULL_HANDLE").unwrap();
        let prefix = visual.debug_bounds("RAIL_VISIBLE_PREFIX").unwrap();
        let full_runtime = visual.debug_bounds("RAIL_FULL_RUNTIME").unwrap();
        for (
            card_selector,
            handle_selector,
            status_selector,
            controls_selector,
            runtime_selector,
            runtime_mark_selector,
            runtime_text_selector,
            key_selector,
        ) in [
            (
                "MISSION_CARD lead",
                "MISSION_CARD_HANDLE lead",
                "MISSION_CARD_STATUS lead",
                "MISSION_CARD_CONTROLS lead",
                "MISSION_CARD_RUNTIME lead",
                "MISSION_CARD_RUNTIME_MARK lead",
                "MISSION_CARD_RUNTIME_TEXT lead",
                "MISSION_CARD_KEY lead",
            ),
            (
                "MISSION_CARD worker",
                "MISSION_CARD_HANDLE worker",
                "MISSION_CARD_STATUS worker",
                "MISSION_CARD_CONTROLS worker",
                "MISSION_CARD_RUNTIME worker",
                "MISSION_CARD_RUNTIME_MARK worker",
                "MISSION_CARD_RUNTIME_TEXT worker",
                "MISSION_CARD_KEY worker",
            ),
        ] {
            let card = visual.debug_bounds(card_selector).unwrap();
            let handle = visual.debug_bounds(handle_selector).unwrap();
            let status = visual.debug_bounds(status_selector).unwrap();
            let controls = visual.debug_bounds(controls_selector).unwrap();
            let runtime = visual.debug_bounds(runtime_selector).unwrap();
            let runtime_mark = visual.debug_bounds(runtime_mark_selector).unwrap();
            let runtime_text = visual.debug_bounds(runtime_text_selector).unwrap();
            let key = visual.debug_bounds(key_selector).unwrap();
            assert!(
                handle.size.width >= prefix.size.width,
                "handle cannot retain its first character at rail width {width}: {handle:?}, prefix {prefix:?}"
            );
            assert!(
                handle.size.width < full.size.width,
                "handle did not truncate at rail width {width}: {handle:?}"
            );
            assert!(
                handle.right() <= controls.left(),
                "handle overlaps controls at rail width {width}"
            );
            assert!(
                status.top() >= handle.bottom(),
                "status should sit below handle at rail width {width}: handle {handle:?}, status {status:?}"
            );
            assert!(
                status.right() <= controls.left(),
                "status overlaps controls at rail width {width}: status {status:?}, controls {controls:?}"
            );
            assert!(
                controls.right() <= card.right(),
                "controls overflow at rail width {width}"
            );
            assert_eq!(
                controls.size.width,
                px(80.),
                "controls shrank at rail width {width}"
            );
            assert!(
                runtime.top() >= status.bottom(),
                "runtime line should sit below header status at rail width {width}: status {status:?}, runtime {runtime:?}"
            );
            assert!(
                runtime.bottom() <= key.top(),
                "runtime line should sit above session_key at rail width {width}: runtime {runtime:?}, key {key:?}"
            );
            assert!(
                runtime.right() <= card.right(),
                "runtime line overflows card at rail width {width}: runtime {runtime:?}, card {card:?}"
            );
            assert_eq!(
                runtime_mark.size,
                size(px(12.), px(12.)),
                "runtime mark should stay 12px at rail width {width}: {runtime_mark:?}"
            );
            assert!(
                runtime_text.size.width < full_runtime.size.width,
                "long runtime line did not truncate at rail width {width}: {runtime_text:?}"
            );
            assert_eq!(
                runtime_text.size.height, full_runtime.size.height,
                "runtime line wrapped to multiple lines at rail width {width}: {runtime_text:?}"
            );
            assert!(
                runtime_text.right() <= card.right(),
                "runtime text overflows card at rail width {width}: {runtime_text:?}, card {card:?}"
            );
            if handle_selector == "MISSION_CARD_HANDLE lead" {
                let avatar = visual.debug_bounds("MISSION_CARD_AVATAR lead").unwrap();
                assert!(
                    avatar.left() >= card.left(),
                    "avatar overflows at rail width {width}"
                );
                assert_eq!(
                    avatar.size.width,
                    px(25.),
                    "avatar shrank at rail width {width}"
                );
                assert!(
                    status.left() >= avatar.right(),
                    "status should sit beside avatar at rail width {width}: avatar {avatar:?}, status {status:?}"
                );
            }
        }
        assert_eq!(
            visual
                .debug_bounds("MISSION_CARD_HANDLE short")
                .unwrap()
                .size
                .width,
            visual.debug_bounds("RAIL_SHORT_HANDLE").unwrap().size.width,
            "short handle changed at rail width {width}"
        );
        for (key_selector, label_selector, copy_selector, card_selector) in [
            (
                "MISSION_CARD_KEY lead",
                "MISSION_CARD_KEY_LABEL lead",
                "MISSION_CARD_KEY_COPY lead",
                "MISSION_CARD lead",
            ),
            (
                "MISSION_CARD_KEY worker",
                "MISSION_CARD_KEY_LABEL worker",
                "MISSION_CARD_KEY_COPY worker",
                "MISSION_CARD worker",
            ),
        ] {
            let key = visual.debug_bounds(key_selector).unwrap();
            assert_eq!(
                key.size.height,
                visual
                    .debug_bounds("RAIL_TWO_LINE_KEY")
                    .unwrap()
                    .size
                    .height,
                "key must use two lines at rail width {width}: {key:?}"
            );
            assert!(
                key.size.width >= visual.debug_bounds("RAIL_KEY_PREFIX").unwrap().size.width,
                "key collapsed to ellipsis only at rail width {width}: {key:?}"
            );
            let label = visual.debug_bounds(label_selector).unwrap();
            let copy = visual.debug_bounds(copy_selector).unwrap();
            let card = visual.debug_bounds(card_selector).unwrap();
            assert_eq!(label.size.width, px(72.));
            assert_eq!(copy.size, size(px(20.), px(20.)));
            assert!(key.right() <= copy.left());
            assert!(copy.right() <= card.right());
            visual.simulate_click(copy.center(), gpui::Modifiers::default());
            cx.run_until_parked();
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some(session_key.into()),
                "copy lost the full key at rail width {width}"
            );
        }
        assert_eq!(
            visual
                .debug_bounds("MISSION_CARD_KEY short")
                .unwrap()
                .size
                .height,
            px(14.),
            "NULL gained a line at rail width {width}"
        );
        assert_eq!(
            visual
                .debug_bounds("MISSION_CARD short")
                .unwrap()
                .size
                .height,
            px(108.),
            "single-line NULL key card should match the spec's 108 px height at rail width {width}"
        );
        for status in [MissionStatus::Running, MissionStatus::Completed] {
            let (short_handle, reference) = if status == MissionStatus::Running && width < 239. {
                ("a", "RAIL_SHORTEST_LEAD_HANDLE")
            } else {
                ("dev", "RAIL_SHORT_LEAD_HANDLE")
            };
            host.update(&mut visual, |host, _, cx| {
                host.workspace.update(cx, |workspace, _| {
                    workspace.sessions[0].handle = short_handle.into();
                    workspace.mission.as_mut().unwrap().status = status;
                });
                cx.notify();
            })
            .unwrap();
            cx.run_until_parked();
            let handle = visual.debug_bounds("MISSION_CARD_HANDLE lead").unwrap();
            let avatar = visual.debug_bounds("MISSION_CARD_AVATAR lead").unwrap();
            assert!(visual.debug_bounds("MISSION_CARD_LEAD lead").is_none());
            assert_eq!(
                handle.size.width,
                visual.debug_bounds(reference).unwrap().size.width,
                "short lead handle changed at rail width {width}, status {status:?}"
            );
            let expected_gap = if status == MissionStatus::Running && width < 208. {
                2.
            } else {
                8.
            };
            assert_eq!(
                handle.left() - avatar.right(),
                px(expected_gap),
                "short lead avatar gap changed at rail width {width}, status {status:?}"
            );
            assert_eq!(
                visual.debug_bounds("MISSION_CARD_CONTROLS lead").is_some(),
                status == MissionStatus::Running
            );
        }
    }
}

#[test]
fn focused_mission_session_actions_match_the_active_tab_and_status() {
    let session = MissionTab::Session("coder".into());
    assert_eq!(
        focused_slot_action_target(
            &session,
            Some(SessionStatus::Running),
            SessionControlKind::Stop,
        ),
        Some("coder")
    );
    assert_eq!(
        focused_slot_action_target(
            &session,
            Some(SessionStatus::Stopped),
            SessionControlKind::Resume,
        ),
        Some("coder")
    );
    assert_eq!(
        focused_slot_action_target(
            &session,
            Some(SessionStatus::Crashed),
            SessionControlKind::Resume,
        ),
        Some("coder")
    );
    assert_eq!(
        focused_slot_action_target(
            &session,
            Some(SessionStatus::Running),
            SessionControlKind::Resume,
        ),
        None
    );
    assert_eq!(
        focused_slot_action_target(
            &session,
            Some(SessionStatus::Stopped),
            SessionControlKind::Stop,
        ),
        None
    );
    assert_eq!(
        focused_slot_action_target(
            &MissionTab::Feed,
            Some(SessionStatus::Running),
            SessionControlKind::Stop,
        ),
        None
    );
}

#[test]
#[cfg(target_os = "macos")]
fn slot_control_titles_follow_effective_bindings() {
    let mut overrides = keymap::KeymapOverrides::new();
    assert_eq!(
        slot_control_title(SessionControlKind::Stop, &overrides),
        "Stop · ⇧⌘X"
    );
    assert_eq!(
        slot_control_title(SessionControlKind::Resume, &overrides),
        "Resume · ⇧⌘R"
    );

    overrides.insert(
        "stop-session".into(),
        Some(keymap::entry("resume-session").unwrap().default.clone()),
    );
    overrides.insert("resume-session".into(), None);
    assert_eq!(
        slot_control_title(SessionControlKind::Stop, &overrides),
        "Stop · ⇧⌘R"
    );
    assert_eq!(
        slot_control_title(SessionControlKind::Resume, &overrides),
        "Resume"
    );
}

#[test]
fn restart_confirmation_names_other_members() {
    assert_eq!(
        restart_confirm_body("worker"),
        "Its conversation so far is discarded. @worker comes back with its brief, the same first turn a cold start gives it, and the other members get a note that it starts over."
    );
}

#[test]
fn restarting_preserves_its_transition_and_starting_overlay() {
    assert_eq!(
        transition_to_begin_on_spawn(Some(MissionTransitionKind::Restarting)),
        None
    );
    for status in [SessionStatus::Stopped, SessionStatus::Running] {
        assert_eq!(
            resolve_slot_overlay(false, Some(MissionTransitionKind::Restarting), status),
            SlotOverlayState::Starting
        );
    }
}

#[test]
fn concurrent_restart_errors_match_the_backend_contract() {
    assert!(is_concurrent_resume_error(
        "session_restart: session abc is already being resumed"
    ));
    assert!(!is_concurrent_resume_error(
        "session_restart: mission router is not mounted"
    ));
}

#[test]
fn stop_all_confirm_names_the_slot_count_and_preserves_recovery_copy() {
    assert_eq!(stop_all_title(3), "Stop all 3 running slots?");
    assert_eq!(stop_all_title(2), "Stop all 2 running slots?");
    assert_eq!(stop_all_title(1), "Stop the running slot?");
    assert_eq!(crate::i18n::text(STOP_ALL_BODY), "Every slot's PTY is killed and whatever turn it is on is cut off. The mission stays open; each slot can be resumed with its conversation, or restarted with its brief.");
}

#[test]
fn signal_rows_show_router_and_runner_as_system_without_changing_the_event() {
    for (from, kind, seed, name) in [
        ("router", "mission_warning", "system", "system"),
        ("runner", "mission_warning", "system", "system"),
        ("worker", "mission_warning", "worker", "@worker"),
        ("human", "slot_restarted", "human", "you"),
        ("human", "ask_lead", "human", "@human"),
    ] {
        let mut event = signal(kind, serde_json::json!({"message":"queued"}));
        event.from = from.into();
        let (author_seed, author_name) = mission_signal_author(&event);
        assert_eq!(author_seed, seed);
        assert_eq!(author_name, name);
        assert_eq!(event.from, from);
    }
}

#[test]
fn slot_restarted_feed_row_uses_payload_handle_and_describes_fresh_brief() {
    let event = signal(
        "slot_restarted",
        serde_json::json!({"handle": "worker", "session_id": "sid", "prior_agent_session_key": "old"}),
    );
    assert_eq!(
        slot_restart_signal_summary(&event).as_deref(),
        Some("signal · slot_restarted → @worker · fresh conversation, brief re-sent")
    );
    assert!(slot_restart_signal_summary(&signal("mission_goal", serde_json::json!({}))).is_none());
}
#[test]
fn stopped_slot_copy_agrees_with_live_sibling_count() {
    assert!(stopped_slot_description("worker", 1)
        .starts_with("@worker's PTY is closed; 1 other slot is still running."));
    assert!(stopped_slot_description("worker", 2)
        .starts_with("@worker's PTY is closed; 2 other slots are still running."));
}

#[test]
fn completed_and_archived_missions_offer_no_slot_actions() {
    assert!(mission_slot_actions_available(
        Some(MissionStatus::Running),
        false,
        false
    ));
    for status in [
        None,
        Some(MissionStatus::Completed),
        Some(MissionStatus::Aborted),
    ] {
        assert!(!mission_slot_actions_available(status, false, false));
    }
    assert!(!mission_slot_actions_available(
        Some(MissionStatus::Running),
        true,
        false
    ));
    assert!(!mission_slot_actions_available(
        Some(MissionStatus::Running),
        false,
        true
    ));
}

#[test]
fn roster_changes_are_dividers_with_person_attribution() {
    for (kind, verb, action) in [
        ("role_joined", "joined", "added"),
        ("role_removed", "left", "removed"),
    ] {
        let event = signal(kind, serde_json::json!({"handle":"worker-2"}));
        let blocks = crate::surfaces::mission_workspace::feed_model::group_feed_blocks(
            std::slice::from_ref(&event),
        );
        assert!(matches!(blocks.as_slice(), [FeedBlock::Divider(_)]));
        assert!(mission_divider_label(&event)
            .starts_with(&format!("@worker-2 {verb} · {action} by you · ")));
    }
}

#[test]
fn an_interruption_is_a_feed_warning_that_names_the_member() {
    let mut event = signal(
        "session_interrupted",
        serde_json::json!({"at": "2026-11-05T07:50:17Z"}),
    );
    event.from = "coder".into();
    let blocks = crate::surfaces::mission_workspace::feed_model::group_feed_blocks(
        std::slice::from_ref(&event),
    );
    assert!(matches!(blocks.as_slice(), [FeedBlock::Signal(_)]));
    assert_eq!(
        mission_signal_body(&event),
        "Runner restarted while @coder was mid-turn. That turn was lost; message @coder to continue."
    );
}

#[cfg(unix)]
#[test]
fn notification_clicks_select_slot_and_feed_and_visible_slots_stay_silent() {
    use gpui::{SystemNotificationResponse, TestAppContext, VisualTestContext};
    use runner_core::protocol::status::{AgentStatus, HumanInteraction, Lifecycle, WaitReason};

    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store);
        store.settings.notifications.missions = crate::app_settings::MissionNotifications {
            question: true,
            input: true,
            idle: true,
            failed: true,
        };
    });
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        crate::notifications::install(cx);
    });
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "main".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = AppRoute::Mission("test-mission".into());
        root.mission_workspace.update(cx, |workspace, _| {
            workspace.active = true;
            workspace.mission_id = Some("test-mission".into());
            workspace.mission =
                Some(runner_daemon::ops::mission::mission_get(&core, "test-mission").unwrap());
            workspace.sessions =
                runner_daemon::ops::session::session_list(&core, "test-mission").unwrap();
            workspace.loading = false;
            workspace.active_tab = MissionTab::Feed;
        });
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let change = |cx: &mut TestAppContext, id: &str| {
        cx.update(|cx| {
            store.update(cx, |store, cx| {
                store.session_statuses.insert(
                    "mission-agent".into(),
                    AgentStatus {
                        lifecycle: Lifecycle::Running,
                        observation: runner_core::protocol::status::AgentObservation {
                            interactions: vec![HumanInteraction {
                                id: id.into(),
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
            })
        })
    };
    visual.deactivate_window();
    cx.run_until_parked();
    change(&mut cx, "first");
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    cx.simulate_system_notification_response(SystemNotificationResponse {
        tag: "mission-agent".into(),
        action_id: None,
    });
    cx.run_until_parked();
    host.read_with(&cx, |root, cx| {
        assert_eq!(root.route, AppRoute::Mission("test-mission".into()));
        assert_eq!(
            root.mission_workspace.read(cx).active_tab,
            MissionTab::Session("mission-agent".into())
        );
    })
    .unwrap();
    assert!(cx.delivered_system_notifications().is_empty());
    let count = cx.shown_system_notifications().len();
    change(&mut cx, "second");
    assert_eq!(cx.shown_system_notifications().len(), count);
    visual.deactivate_window();
    cx.run_until_parked();
    let events = core.events.clone();
    events.emit("event/appended", &serde_json::json!({"mission_id":"test-mission", "event": Event {
        id: "q".into(), ts: chrono::Utc::now(), crew_id: Some("test-crew".into()), mission_id: "test-mission".into(), kind: EventKind::Signal, from: "app".into(), to: Default::default(), signal_type: Some(runner_core::model::SignalType::new("human_question")), payload: serde_json::json!({"prompt":"Ship?"}),
    }, "next_offset":1}));
    cx.run_until_parked();
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    cx.simulate_system_notification_response(SystemNotificationResponse {
        tag: "question:q".into(),
        action_id: None,
    });
    cx.run_until_parked();
    host.read_with(&cx, |root, cx| {
        assert_eq!(root.mission_workspace.read(cx).active_tab, MissionTab::Feed);
    })
    .unwrap();
    assert!(cx.delivered_system_notifications().is_empty());
    host.update(&mut visual, |root, _, cx| {
        root.mission_workspace.update(cx, |workspace, _| {
            workspace.active_tab = MissionTab::Session("mission-agent".into());
        });
        store.update(cx, |store, _| {
            store
                .settings
                .last_mission_terminal_ids
                .insert("test-mission".into(), "mission-agent".into());
        });
    })
    .unwrap();
    visual.deactivate_window();
    cx.run_until_parked();
    for activity in [
        runner_core::protocol::mission::MissionActivityState::Busy,
        runner_core::protocol::mission::MissionActivityState::Idle,
    ] {
        cx.update(|cx| {
            store.update(cx, |store, cx| {
                store.missions[0].activity = Some(activity);
                store.revisions.missions += 1;
                cx.notify();
            })
        });
    }
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    assert_eq!(
        cx.delivered_system_notifications()[0].tag.as_ref(),
        "mission:test-mission:idle"
    );
    cx.simulate_system_notification_response(SystemNotificationResponse {
        tag: "mission:test-mission:idle".into(),
        action_id: None,
    });
    cx.run_until_parked();
    host.read_with(&cx, |root, cx| {
        assert_eq!(root.mission_workspace.read(cx).active_tab, MissionTab::Feed);
        assert!(!store
            .read(cx)
            .settings
            .last_mission_terminal_ids
            .contains_key("test-mission"));
    })
    .unwrap();
    assert!(cx.delivered_system_notifications().is_empty());
    host.update(&mut visual, |root, _, cx| {
        root.mission_workspace.update(cx, |workspace, _| {
            workspace.active_tab = MissionTab::Session("mission-agent".into());
        });
    })
    .unwrap();
    cx.simulate_system_notification_response(SystemNotificationResponse {
        tag: "mission:test-mission:idle".into(),
        action_id: None,
    });
    cx.run_until_parked();
    host.read_with(&cx, |root, cx| {
        assert_eq!(root.mission_workspace.read(cx).active_tab, MissionTab::Feed);
    })
    .unwrap();
}

#[cfg(unix)]
#[test]
fn asynchronous_slot_removal_withdraws_a_question_when_it_reveals_the_feed() {
    use gpui::{TestAppContext, VisualTestContext};

    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store);
        store.settings.notifications.missions = crate::app_settings::MissionNotifications {
            question: true,
            input: true,
            idle: true,
            failed: true,
        };
    });
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        crate::notifications::install(cx);
    });
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "main".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = AppRoute::Mission("test-mission".into());
        root.mission_workspace.update(cx, |workspace, _| {
            workspace.active = true;
            workspace.mission_id = Some("test-mission".into());
            workspace.mission =
                Some(runner_daemon::ops::mission::mission_get(&core, "test-mission").unwrap());
            workspace.sessions =
                runner_daemon::ops::session::session_list(&core, "test-mission").unwrap();
            workspace.loading = false;
            workspace.active_tab = MissionTab::Session("mission-agent".into());
        });
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.update(|window, _| window.activate_window());
    cx.run_until_parked();
    visual.deactivate_window();
    cx.run_until_parked();
    core.events.emit("event/appended", &serde_json::json!({"mission_id":"test-mission", "event": Event {
        id: "removed-slot-question".into(), ts: chrono::Utc::now(), crew_id: Some("test-crew".into()), mission_id: "test-mission".into(), kind: EventKind::Signal, from: "app".into(), to: Default::default(), signal_type: Some(runner_core::model::SignalType::new("human_question")), payload: serde_json::json!({"prompt":"Ship?"}),
    }, "next_offset":1}));
    cx.run_until_parked();
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    visual.update(|window, _| window.activate_window());
    cx.run_until_parked();
    core.db
        .get()
        .unwrap()
        .execute(
            "UPDATE sessions SET archived_at = '2026-10-13T00:00:00Z' WHERE id = 'mission-agent'",
            [],
        )
        .unwrap();
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.session_statuses.remove("mission-agent");
            for mission in &mut store.missions {
                mission
                    .session_statuses
                    .retain(|(id, _)| id != "mission-agent");
            }
            store.revisions.activity += 1;
            cx.notify();
        })
    });
    host.read_with(&cx, |root, cx| {
        assert_eq!(
            root.mission_workspace.read(cx).active_tab,
            MissionTab::Session("mission-agent".into())
        )
    })
    .unwrap();
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    host.update(&mut visual, |root, window, cx| {
        root.mission_workspace.update(cx, |workspace, cx| {
            workspace.refresh_open_mission(window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    host.read_with(&cx, |root, cx| {
        assert_eq!(root.mission_workspace.read(cx).active_tab, MissionTab::Feed)
    })
    .unwrap();
    assert!(cx.delivered_system_notifications().is_empty());
    assert!(cx
        .dismissed_system_notifications()
        .iter()
        .any(|tag| tag.as_ref() == "question:removed-slot-question"));
}

#[cfg(debug_assertions)]
#[test]
fn saved_action_failure_translates_its_wrapper_and_keeps_backend_details_literal() {
    use crate::i18n::{self, Locale};
    i18n::testing::with_locale(Locale::English, || {
        let failure = action_failure(
            i18n::message("action.resume_mission"),
            "backend_error: literal {action} 中文",
        );
        assert_eq!(
            failure.resolve(),
            "Couldn't resume the mission: literal {action} 中文"
        );
        i18n::set_active_locale(Locale::SimplifiedChinese);
        assert_eq!(failure.resolve(), "无法恢复任务：literal {action} 中文");
        i18n::set_active_locale(Locale::English);
        assert_eq!(
            failure.resolve(),
            "Couldn't resume the mission: literal {action} 中文"
        );
    });
}

fn long_feed_events() -> Vec<Event> {
    (0..2000).map(|index| Event {
        id: format!("{index:026}"),
        ts: chrono::DateTime::from_timestamp(1_700_000_000 + index, 0).unwrap(),
        crew_id: Some("crew".into()),
        mission_id: "mission".into(),
        kind: EventKind::Message,
        from: if index % 2 == 0 { "coder" } else { "reviewer" }.into(),
        to: Default::default(),
        signal_type: None,
        payload: serde_json::json!({"text": "A **message** with `inline code`.\n\n- First item\n- Second item\n\n```rust\nlet value = 42;\n```"}),
    }).collect()
}

fn assert_feed_projection(workspace: &MissionWorkspace) {
    assert_eq!(
        format!("{:?}", workspace.feed_blocks),
        format!(
            "{:?}",
            crate::surfaces::mission_workspace::feed_model::group_feed_blocks(&workspace.events)
        ),
    );
    assert_eq!(
        workspace.feed_list.item_count(),
        workspace.feed_item_count()
    );
}

#[test]
fn feed_appends_match_full_projection_and_deduplicate_sorted_insertion() {
    let (_temp, mut cx, host, workspace) = mission_workspace_test();
    let mut events = long_feed_events();
    events.truncate(20);
    events[1].from = "coder".into();
    events[2].from = "coder".into();
    events[2].ts = events[1].ts + chrono::Duration::minutes(5);
    events[3].from = "coder".into();
    events[3].ts = events[2].ts + chrono::Duration::seconds(301);
    events[4] = Event {
        id: events[4].id.clone(),
        ..signal("mission_start", serde_json::json!({}))
    };
    events[5] = Event {
        id: events[5].id.clone(),
        ..signal(
            "human_question",
            serde_json::json!({"prompt":"Continue?", "choices":["yes","no"]}),
        )
    };
    events[6] = Event {
        id: events[6].id.clone(),
        ..signal("session_status", serde_json::json!({}))
    };
    events[7] = Event {
        id: events[7].id.clone(),
        from: "human".into(),
        ..signal("human_said", serde_json::json!({"text":"hello"}))
    };
    events[8].payload["source_event"] = events[7].id.clone().into();
    events[9] = Event {
        id: events[9].id.clone(),
        from: "human".into(),
        ..signal(
            "human_response",
            serde_json::json!({"question_id":events[5].id, "choice":"yes"}),
        )
    };
    events[10].payload["question_id"] = events[5].id.clone().into();
    events[11] = Event {
        id: events[11].id.clone(),
        ..signal("inbox_read", serde_json::json!({}))
    };
    events[12] = Event {
        id: events[12].id.clone(),
        ..signal("ask_human", serde_json::json!({}))
    };
    events[13] = Event {
        id: events[13].id.clone(),
        ..signal("mission_goal", serde_json::json!({"text":"goal"}))
    };
    events[14].to = Some("qa".into()).into();
    events[17].from = "coder".into();
    events[17].to = Some("qa".into()).into();
    events[15] = Event {
        id: events[15].id.clone(),
        ..signal("mission_stopped", serde_json::json!({}))
    };
    workspace.update(&mut cx, |this, _| {
        for event in &events {
            assert!(this.append_event(event.clone()));
            assert_feed_projection(this);
        }
        let count = this.events.len();
        assert!(!this.append_event(events[5].clone()));
        assert_eq!(this.events.len(), count);
        let mut earlier = events[0].clone();
        earlier.id = "-earlier".into();
        assert!(this.append_event(earlier));
        assert_eq!(this.events.first().unwrap().id, "-earlier");
        assert_feed_projection(this);
        let mut middle = events[2].clone();
        middle.id.push('a');
        assert!(this.append_event(middle));
        assert!(this.events.windows(2).all(|pair| pair[0].id < pair[1].id));
        assert_feed_projection(this);
    });
    host.update(&mut cx, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn long_feed_builds_visible_blocks_caches_markdown_and_preserves_scroll_and_selection() {
    use crate::ui::markdown::{Block, Position, HIGHLIGHT_COUNT, PARSE_COUNT};
    use gpui::{ListOffset, VisualTestContext};
    let (_temp, mut cx, host, workspace) = mission_workspace_test();
    HIGHLIGHT_COUNT.set(0);
    workspace.update(&mut cx, |this, _| {
        this.events = long_feed_events();
        this.rebuild_event_projection();
    });
    assert_eq!(
        HIGHLIGHT_COUNT.get(),
        0,
        "initial projection of 2,000 short code blocks must do zero tree-sitter work"
    );
    host.update(&mut cx, |host, _, cx| {
        host.show_feed = true;
        cx.notify();
    })
    .unwrap();
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    let initial_highlights = HIGHLIGHT_COUNT.get();
    assert!(
        initial_highlights > 0 && initial_highlights < 50,
        "first draw highlights only visible code blocks, got {initial_highlights}"
    );
    workspace.read_with(&visual, |this, _| {
        assert_feed_projection(this);
        assert!(this.feed_list.is_following_tail());
        assert!(this
            .feed_list
            .bounds_for_item(this.feed_rows.len() - 1)
            .is_some());
        assert!(this.feed_block_builds.get() > 0);
        assert!(this.feed_block_builds.get() < 100);
        let feed_md = this.feed_markdown.borrow();
        let last_md = &feed_md[&this.events[1999].id];
        let Block::Code(last_code) = last_md.page().blocks.last().unwrap() else {
            panic!("last message code block");
        };
        assert!(
            last_code.highlights().is_some(),
            "visible bottom message code block is highlighted"
        );
        let first_md = &feed_md[&this.events[0].id];
        let Block::Code(first_code) = first_md.page().blocks.last().unwrap() else {
            panic!("first message code block");
        };
        assert!(
            first_code.highlights().is_none(),
            "off-screen first message stays unhighlighted until scrolled into view"
        );
    });
    PARSE_COUNT.set(0);
    HIGHLIGHT_COUNT.set(0);
    workspace.update(&mut visual, |this, cx| {
        this.feed_block_builds.set(0);
        cx.notify();
    });
    visual.run_until_parked();
    assert_eq!(PARSE_COUNT.get(), 0);
    assert_eq!(HIGHLIGHT_COUNT.get(), 0);
    visual.simulate_input("typing in a long feed");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        // Each 5-row message spans two 4-row feed chunks, so ~6 visible chunks
        // build per frame across typing renders instead of the 4,000 total rows.
        let builds = this.feed_block_builds.get();
        assert!(builds < 200, "feed_block_builds={builds}");
        assert_eq!(this.composer.draft, "typing in a long feed");
    });
    assert_eq!(PARSE_COUNT.get(), 0);
    assert_eq!(HIGHLIGHT_COUNT.get(), 0);
    workspace.update(&mut visual, |this, cx| {
        let mut event = this.events.last().unwrap().clone();
        event.id = "2000".into();
        this.append_event(event);
        cx.notify();
    });
    visual.run_until_parked();
    assert!(PARSE_COUNT.get() <= 1);
    HIGHLIGHT_COUNT.set(0);
    workspace.update(&mut visual, |this, cx| {
        assert!(this.feed_list.is_following_tail());
        this.feed_list.scroll_by(px(-40.));
        assert!(this.feed_is_near_bottom());
        let mut event = this.events.last().unwrap().clone();
        event.id = "20001".into();
        this.append_event(event);
        assert!(this.feed_list.is_following_tail());
        this.feed_list.scroll_to(ListOffset {
            item_ix: 1900,
            offset_in_item: px(30.),
        });
        this.feed_selection = Some(FeedSelection {
            event_id: this.events[1900].id.clone(),
            anchor: Position { row: 0, offset: 0 },
            head: Position { row: 0, offset: 3 },
        });
        cx.notify();
    });
    visual.run_until_parked();
    let scrolled_highlights = HIGHLIGHT_COUNT.get();
    assert!(
        scrolled_highlights > 0 && scrolled_highlights < 50,
        "scrolling highlights only newly visible code blocks, got {scrolled_highlights}"
    );
    let top = workspace.read_with(&visual, |this, _| this.feed_list.logical_scroll_top());
    workspace.update(&mut visual, |this, cx| {
        assert!(!this.feed_is_near_bottom());
        let mut event = this.events.last().unwrap().clone();
        event.id = "2001".into();
        this.append_event(event);
        assert!(this.feed_has_new_messages);
        assert_eq!(this.feed_list.logical_scroll_top().item_ix, top.item_ix);
        assert_eq!(
            this.feed_list.logical_scroll_top().offset_in_item,
            top.offset_in_item
        );
        cx.notify();
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.feed_has_new_messages);
        assert_eq!(this.feed_list.logical_scroll_top().item_ix, top.item_ix);
        assert_eq!(
            this.feed_list.logical_scroll_top().offset_in_item,
            top.offset_in_item
        );
    });
    assert!(visual.debug_bounds("mission-feed-new-messages").is_some());
    let pill = visual.debug_bounds("mission-feed-new-messages").unwrap();
    visual.simulate_click(pill.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.update(&mut visual, |this, cx| {
        assert!(!this.feed_has_new_messages);
        assert!(this.feed_selection.is_some());
        this.feed_list.scroll_to(top);
        cx.notify();
    });
    visual.run_until_parked();
    workspace.update(&mut visual, |this, cx| {
        let mut event = this.events.last().unwrap().clone();
        event.id = "20015".into();
        this.append_event(event);
        assert!(this.feed_has_new_messages);
        cx.notify();
    });
    visual.run_until_parked();
    workspace.update(&mut visual, |this, cx| {
        this.feed_list.scroll_to(ListOffset {
            item_ix: this.feed_rows.len() - 1,
            offset_in_item: px(0.),
        });
        cx.notify();
    });
    visual.run_until_parked();
    let viewport = workspace.read_with(&visual, |this, _| this.feed_list.viewport_bounds());
    for _ in 0..8 {
        visual.simulate_event(gpui::ScrollEvent {
            position: viewport.center(),
            delta: gpui::ScrollDelta::Lines(point(0., -100000.)),
            ..Default::default()
        });
        visual.run_until_parked();
    }
    workspace.update(&mut visual, |this, cx| {
        assert!(!this.feed_has_new_messages);
        assert!(this.feed_list.is_following_tail());
        this.feed_list.scroll_to(top);
        assert_eq!(
            this.feed_selection.as_ref().unwrap().event_id,
            this.events[1900].id
        );
        let mut event = this.events.last().unwrap().clone();
        event.id = "2002".into();
        event.from = "human".into();
        this.append_event(event);
        assert!(this.feed_list.is_following_tail());
        assert!(!this.feed_has_new_messages);
        cx.notify();
    });
    visual.run_until_parked();
    host.update(&mut visual, |_, window, cx| {
        let focus = workspace.read(cx).root_focus.clone();
        focus.focus(window, cx);
    })
    .unwrap();
    visual.dispatch_action(Copy);
    assert_eq!(
        visual
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("A m")
    );
    workspace.update(&mut visual, |this, cx| {
        let selected = this.feed_selection.as_ref().unwrap().event_id.clone();
        this.events.retain(|event| event.id != selected);
        this.rebuild_event_projection();
        assert!(this.feed_selection.is_none());
        assert!(!this.feed_markdown.borrow().is_empty());
        this.prepare_mission("another-mission".into());
        assert!(this.feed_markdown.borrow().is_empty());
        assert!(this.event_ids.is_empty());
        assert!(this.feed_selection.is_none());
        assert!(this.feed_list.is_following_tail());
        assert_eq!(this.feed_list.item_count(), 1);
        cx.notify();
    });
    visual.run_until_parked();
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn feed_remeasures_growing_groups_ask_cards_and_text_settings() {
    use gpui::{ListOffset, VisualTestContext};
    let (_temp, mut cx, host, workspace) = mission_workspace_test();
    workspace.update(&mut cx, |this, _| {
        let mut question = signal(
            "human_question",
            serde_json::json!({"prompt":"Continue?", "choices":["yes","no"]}),
        );
        question.id = "0000".into();
        this.append_event(question);
        let mut message = long_feed_events().remove(0);
        message.id = "0001".into();
        message.payload["text"] = "A tall group.\n\n".repeat(100).into();
        this.append_event(message);
    });
    host.update(&mut cx, |host, _, cx| {
        host.show_feed = true;
        cx.notify();
    })
    .unwrap();
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    workspace.update(&mut visual, |this, cx| {
        this.feed_list.scroll_to(ListOffset::default());
        cx.notify();
    });
    visual.run_until_parked();
    workspace.update(&mut visual, |this, cx| {
        assert!(this.feed_list.bounds_for_item(0).is_some());
        assert!(this.feed_list.bounds_for_item(1).is_some());
        this.feed_list.scroll_to(ListOffset {
            item_ix: 1,
            offset_in_item: px(25.),
        });
        let mut message = this.events.last().unwrap().clone();
        message.id = "0002".into();
        // Appending to a group leaves its existing presentation rows measured and anchored.
        message.payload["text"] = "More text.\n\n".repeat(100).into();
        this.feed_list.pause_following_tail();
        let top = this.feed_list.logical_scroll_top();
        this.append_event(message);
        assert!(this.feed_list.bounds_for_item(1).is_some());
        assert_eq!(
            this.feed_list.logical_scroll_top().offset_in_item,
            top.offset_in_item
        );
        this.feed_list.scroll_to(ListOffset::default());
        cx.notify();
    });
    visual.run_until_parked();
    let ask_height = workspace.read_with(&visual, |this, _| {
        this.feed_list.bounds_for_item(0).unwrap().size.height
    });
    workspace.update(&mut visual, |this, cx| {
        let mut answer = signal(
            "human_response",
            serde_json::json!({"question_id":"0000", "choice":"yes"}),
        );
        answer.id = "0003".into();
        this.append_event(answer);
        this.feed_list.scroll_to(ListOffset::default());
        assert!(this.feed_list.bounds_for_item(0).is_none());
        cx.notify();
    });
    visual.run_until_parked();
    workspace.update(&mut visual, |this, cx| {
        assert!(this.feed_list.bounds_for_item(0).unwrap().size.height > ask_height);
        this.pending_ask_choices.insert("0000".into(), "yes".into());
        this.invalidate_feed_ask("0000");
        assert!(this.feed_list.bounds_for_item(0).is_none());
        cx.notify();
    });
    visual.run_until_parked();
    let before_zoom = workspace.read_with(&visual, |this, _| {
        this.feed_list.bounds_for_item(0).unwrap().size.height
    });
    workspace.update(&mut visual, |this, cx| {
        this.update_app_settings(cx, false, |settings| {
            settings.app_zoom = 1.5;
            true
        });
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.feed_list.bounds_for_item(0).unwrap().size.height > before_zoom);
    });
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn empty_feed_panel_fills_remaining_space_and_shrinks_for_new_content() {
    use gpui::VisualTestContext;
    let (_temp, mut cx, host, workspace) = mission_workspace_test();
    workspace.update(&mut cx, |this, _| {
        this.roster.clear();
        this.mission = Some(runner_core::protocol::model::Mission {
            id: "mission".into(),
            crew_id: Some("crew".into()),
            title: "empty".into(),
            project_id: None,
            goal_override: None,
            cwd: None,
            pinned_at: None,
            status: MissionStatus::Running,
            started_at: Utc::now(),
            stopped_at: None,
            archived_at: None,
            linked_chat: None,
        });
    });
    host.update(&mut cx, |host, _, cx| {
        host.show_feed = true;
        cx.notify();
    })
    .unwrap();
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    let before = visual.debug_bounds("MISSION_NO_ROLES").unwrap();
    let viewport = workspace.read_with(&visual, |this, _| this.feed_list.viewport_bounds());
    assert_eq!(before.bottom(), viewport.bottom() - px(24.));
    assert!(before.size.height > px(220.));
    workspace.update(&mut visual, |this, cx| {
        this.append_event(long_feed_events().remove(0));
        cx.notify();
    });
    visual.run_until_parked();
    let after = visual.debug_bounds("MISSION_NO_ROLES").unwrap();
    assert!(after.size.height < before.size.height);
    assert_eq!(after.bottom(), viewport.bottom() - px(24.));
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn feed_wheel_reaches_unmeasured_history_and_preserves_horizontal_padding() {
    use gpui::VisualTestContext;
    let (_temp, mut cx, host, workspace) = mission_workspace_test();
    workspace.update(&mut cx, |this, _| {
        this.events = long_feed_events();
        this.rebuild_event_projection();
    });
    host.update(&mut cx, |host, _, cx| {
        host.show_feed = true;
        cx.notify();
    })
    .unwrap();
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    let (initial_top, viewport) = workspace.read_with(&visual, |this, _| {
        (
            this.feed_list.logical_scroll_top().item_ix,
            this.feed_list.viewport_bounds(),
        )
    });
    assert_eq!(viewport.left(), px(24.));
    assert_eq!(viewport.right(), px(576.));
    let group = visual
        .debug_bounds("MISSION_MESSAGE_GROUP 00000000000000000000001999")
        .unwrap();
    assert_eq!(group.left(), viewport.left());
    assert_eq!(group.size.width, viewport.size.width);
    for _ in 0..60 {
        visual.simulate_event(gpui::ScrollEvent {
            position: viewport.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(100.))),
            ..Default::default()
        });
        visual.run_until_parked();
    }
    workspace.read_with(&visual, |this, _| {
        let top = this.feed_list.logical_scroll_top().item_ix;
        assert!(
            top + 10 < initial_top,
            "wheel scrolling stuck at {top} after starting at {initial_top}"
        );
        let (event_id, _) = this.feed_row_key(top).unwrap();
        assert!(this.feed_markdown.borrow().contains_key(event_id));
    });
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn feed_near_bottom_boundary_includes_vertical_padding_and_zoom() {
    use gpui::VisualTestContext;
    let (_temp, mut cx, host, workspace) = mission_workspace_test();
    workspace.update(&mut cx, |this, _| {
        this.events = long_feed_events();
        this.rebuild_event_projection();
    });
    host.update(&mut cx, |host, _, cx| {
        host.show_feed = true;
        cx.notify();
    })
    .unwrap();
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    for (index, zoom) in [1., 1.5].into_iter().enumerate() {
        workspace.update(&mut visual, |this, cx| {
            this.update_app_settings(cx, false, |settings| {
                settings.app_zoom = zoom;
                true
            });
            this.scroll_feed_to_bottom();
            cx.notify();
        });
        visual.run_until_parked();
        workspace.update(&mut visual, |this, cx| {
            this.feed_list.scroll_by(px(-100.));
            assert!(
                !this.feed_is_near_bottom(),
                "100px from bottom at zoom {zoom}"
            );
            let top = this.feed_list.logical_scroll_top();
            let mut event = this.events.last().unwrap().clone();
            event.id = format!("{:026}", 2000 + index * 2);
            this.append_event(event);
            assert!(this.feed_has_new_messages);
            assert!(!this.feed_list.is_following_tail());
            assert_eq!(this.feed_list.logical_scroll_top().item_ix, top.item_ix);
            assert_eq!(
                this.feed_list.logical_scroll_top().offset_in_item,
                top.offset_in_item
            );
            cx.notify();
        });
        visual.run_until_parked();
        workspace.update(&mut visual, |this, cx| {
            assert!(this.feed_has_new_messages);
            this.scroll_feed_to_bottom();
            cx.notify();
        });
        visual.run_until_parked();
        workspace.update(&mut visual, |this, cx| {
            this.feed_list.scroll_by(px(-40.));
            assert!(
                this.feed_is_near_bottom(),
                "40px from bottom at zoom {zoom}"
            );
            let mut event = this.events.last().unwrap().clone();
            event.id = format!("{:026}", 2001 + index * 2);
            this.append_event(event);
            assert!(!this.feed_has_new_messages);
            assert!(this.feed_list.is_following_tail());
            cx.notify();
        });
        visual.run_until_parked();
    }
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn long_feed_same_author_group_builds_only_visible_messages() {
    use crate::ui::markdown::{PARSE_COUNT, RENDER_COUNT};
    use gpui::{ListOffset, VisualTestContext};
    let (_temp, mut cx, host, workspace) = mission_workspace_test();
    workspace.update(&mut cx, |this, _| {
        this.events = long_feed_events()
            .into_iter()
            .map(|mut event| {
                event.from = "coder".into();
                event
            })
            .collect();
        this.rebuild_event_projection();
        assert_eq!(this.feed_blocks.len(), 1);
        assert_eq!(this.feed_rows.len(), 4000);
    });
    host.update(&mut cx, |host, _, cx| {
        host.show_feed = true;
        cx.notify();
    })
    .unwrap();
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    PARSE_COUNT.set(0);
    RENDER_COUNT.set(0);
    workspace.update(&mut visual, |this, cx| {
        this.feed_block_builds.set(0);
        cx.notify();
    });
    visual.run_until_parked();
    visual.simulate_input("x");
    visual.run_until_parked();
    assert_eq!(PARSE_COUNT.get(), 0);
    assert!(
        RENDER_COUNT.get() < 100,
        "built {} markdown blocks",
        RENDER_COUNT.get()
    );
    workspace.update(&mut visual, |this, cx| {
        assert!(this.feed_block_builds.get() < 100);
        this.feed_list.scroll_to(ListOffset {
            item_ix: 1000,
            offset_in_item: px(15.),
        });
        cx.notify();
    });
    visual.run_until_parked();
    workspace.update(&mut visual, |this, cx| {
        let top = this.feed_list.logical_scroll_top();
        let mut event = this.events.last().unwrap().clone();
        event.id = format!("{:026}", 2000);
        this.append_event(event);
        assert_eq!(this.feed_blocks.len(), 1);
        assert_eq!(this.feed_rows.len(), 4002);
        assert_eq!(this.feed_list.logical_scroll_top().item_ix, top.item_ix);
        assert_eq!(
            this.feed_list.logical_scroll_top().offset_in_item,
            top.offset_in_item
        );
        assert!(this.feed_has_new_messages);
        cx.notify();
    });
    visual.run_until_parked();
    assert_eq!(PARSE_COUNT.get(), 1);
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn long_feed_single_post_builds_visible_sections_and_keeps_selection_offsets() {
    use crate::ui::markdown::{Position, PARSE_COUNT, RENDER_COUNT};
    use gpui::{ListOffset, VisualTestContext};
    let (_temp, mut cx, host, workspace) = mission_workspace_test();
    workspace.update(&mut cx, |this, _| {
        let mut event = long_feed_events().remove(0);
        event.payload["text"] = (0..2000)
            .map(|i| format!("Paragraph {i}.\n\n"))
            .collect::<String>()
            .into();
        this.events = vec![event];
        this.rebuild_event_projection();
        assert_eq!(this.feed_blocks.len(), 1);
        assert_eq!(this.feed_rows.len(), 500);
    });
    host.update(&mut cx, |host, _, cx| {
        host.show_feed = true;
        cx.notify();
    })
    .unwrap();
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    PARSE_COUNT.set(0);
    RENDER_COUNT.set(0);
    visual.simulate_input("x");
    visual.run_until_parked();
    assert_eq!(PARSE_COUNT.get(), 0);
    assert!(
        RENDER_COUNT.get() < 100,
        "built {} markdown blocks",
        RENDER_COUNT.get()
    );
    workspace.update(&mut visual, |this, cx| {
        this.feed_selection = Some(FeedSelection {
            event_id: this.events[0].id.clone(),
            anchor: Position {
                row: 1996,
                offset: 0,
            },
            head: Position {
                row: 1997,
                offset: 15,
            },
        });
        this.feed_list.scroll_to(ListOffset::default());
        cx.notify();
    });
    visual.run_until_parked();
    workspace.update(&mut visual, |this, cx| {
        this.feed_list.scroll_to(ListOffset {
            item_ix: 499,
            offset_in_item: px(0.),
        });
        cx.notify();
    });
    visual.run_until_parked();
    host.update(&mut visual, |_, window, cx| {
        let focus = workspace.read(cx).root_focus.clone();
        focus.focus(window, cx);
    })
    .unwrap();
    visual.dispatch_action(Copy);
    assert_eq!(
        visual
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("Paragraph 1996.\nParagraph 1997.")
    );
    workspace.update(&mut visual, |this, _| {
        this.feed_list.pause_following_tail();
        let top = this.feed_list.logical_scroll_top();
        this.rebuild_event_projection();
        assert_eq!(this.feed_list.logical_scroll_top().item_ix, top.item_ix);
        assert!(this.feed_selection.is_some());
    });
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn feed_message_and_section_continuations_align_at_non_default_zoom() {
    use gpui::{ListOffset, VisualTestContext};
    let (_temp, mut cx, host, workspace) = mission_workspace_test();
    workspace.update(&mut cx, |this, cx| {
        this.update_app_settings(cx, false, |settings| {
            settings.app_zoom = 1.5;
            true
        });
        let mut events = long_feed_events();
        events.truncate(2);
        events[0].payload["text"] = "First message.".into();
        events[1].from = events[0].from.clone();
        events[1].payload["text"] = "Continuation paragraph.\n\n".repeat(20).into();
        this.events = events;
        this.rebuild_event_projection();
    });
    host.update(&mut cx, |host, _, cx| {
        host.show_feed = true;
        cx.notify();
    })
    .unwrap();
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    workspace.update(&mut visual, |this, cx| {
        this.feed_list.scroll_to(ListOffset::default());
        cx.notify();
    });
    visual.run_until_parked();
    let header = visual
        .debug_bounds("MISSION_MESSAGE_BODY 00000000000000000000000000 0")
        .unwrap();
    for selector in [
        "MISSION_MESSAGE_BODY 00000000000000000000000001 0",
        "MISSION_MESSAGE_BODY 00000000000000000000000001 1",
    ] {
        let continuation = visual.debug_bounds(selector).unwrap();
        assert_eq!(continuation.left(), header.left());
        assert_eq!(continuation.size.width, header.size.width);
    }
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn long_feed_multiline_paragraph_and_table_build_only_visible_lines_and_cells() {
    use crate::ui::markdown::{Position, LINE_COUNT, PARSE_COUNT};
    use gpui::{ListOffset, VisualTestContext};
    for table in [false, true] {
        let (_temp, mut cx, host, workspace) = mission_workspace_test();
        workspace.update(&mut cx, |this, _| {
            let mut event = long_feed_events().remove(0);
            event.payload["text"] = if table {
                format!(
                    "| Left | Right |\n| --- | --- |\n{}",
                    (0..2000)
                        .map(|i| format!("| L{i} | R{i} |\n"))
                        .collect::<String>()
                )
            } else {
                (0..2000)
                    .map(|i| format!("Line {i}.\n"))
                    .collect::<String>()
            }
            .into();
            this.events = vec![event];
            this.rebuild_event_projection();
            assert_eq!(this.feed_blocks.len(), 1);
            assert!(this.feed_rows.len() >= 500);
        });
        host.update(&mut cx, |host, _, cx| {
            host.show_feed = true;
            cx.notify();
        })
        .unwrap();
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.run_until_parked();
        PARSE_COUNT.set(0);
        LINE_COUNT.set(0);
        visual.simulate_input("x");
        visual.run_until_parked();
        assert_eq!(PARSE_COUNT.get(), 0);
        assert!(
            LINE_COUNT.get() > 0 && LINE_COUNT.get() < 200,
            "table={table}: built {} lines/cells",
            LINE_COUNT.get()
        );
        workspace.update(&mut visual, |this, cx| {
            this.feed_selection = Some(FeedSelection {
                event_id: this.events[0].id.clone(),
                anchor: if table {
                    Position {
                        row: 2000,
                        offset: 0,
                    }
                } else {
                    Position {
                        row: 1998,
                        offset: 0,
                    }
                },
                head: if table {
                    Position {
                        row: 2000,
                        offset: 11,
                    }
                } else {
                    Position {
                        row: 1999,
                        offset: 10,
                    }
                },
            });
            this.feed_list.scroll_to(ListOffset::default());
            cx.notify();
        });
        visual.run_until_parked();
        workspace.update(&mut visual, |this, cx| {
            this.scroll_feed_to_bottom();
            cx.notify();
        });
        visual.run_until_parked();
        host.update(&mut visual, |_, window, cx| {
            let focus = workspace.read(cx).root_focus.clone();
            focus.focus(window, cx);
        })
        .unwrap();
        PARSE_COUNT.set(0);
        visual.dispatch_action(Copy);
        assert_eq!(PARSE_COUNT.get(), 0, "copy reads cached FeedMarkdown");
        assert_eq!(
            visual
                .read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some(if table {
                "L1999\tR1999"
            } else {
                "Line 1998.\nLine 1999."
            })
        );
        if table {
            host.update(&mut visual, |host, _, cx| {
                host.width = 300.;
                cx.notify();
            })
            .unwrap();
            visual.run_until_parked();
            let viewport = workspace.read_with(&visual, |this, _| this.feed_list.viewport_bounds());
            let last = visual
                .debug_bounds("COMPACT_MD_TABLE_ROW 00000000000000000000000000 2000")
                .unwrap();
            assert!(
                last.right() <= viewport.right(),
                "narrow table stays inside viewport instead of overflowing: {last:?} vs {viewport:?}"
            );
        }
        let ask_row = workspace.update(&mut visual, |this, cx| {
            let row = this.feed_rows.len();
            let mut question = signal(
                "human_question",
                serde_json::json!({"prompt":"Continue?", "choices":["yes","no"]}),
            );
            question.id = "split-post-question".into();
            this.append_event(question);
            cx.notify();
            row
        });
        visual.run_until_parked();
        workspace.update(&mut visual, |this, cx| {
            assert!(this.feed_list.bounds_for_item(ask_row).is_some());
            this.pending_ask_choices
                .insert("split-post-question".into(), "yes".into());
            this.invalidate_feed_ask("split-post-question");
            assert!(this.feed_list.bounds_for_item(ask_row).is_none());
            cx.notify();
        });
        visual.run_until_parked();
        host.update(&mut visual, |_, window, _| window.remove_window())
            .unwrap();
    }
}

#[test]
fn long_feed_code_block_builds_only_visible_lines() {
    use crate::ui::markdown::{Position, LINE_COUNT, PARSE_COUNT, RENDER_COUNT};
    use gpui::{ListOffset, VisualTestContext};
    let (_temp, mut cx, host, workspace) = mission_workspace_test();
    workspace.update(&mut cx, |this, _| {
        let mut event = long_feed_events().remove(0);
        event.payload["text"] = format!(
            "```rust\n{}```\n",
            (0..2000)
                .map(|i| format!("let line_{i} = {i};\n"))
                .collect::<String>()
        )
        .into();
        this.events = vec![event];
        this.rebuild_event_projection();
        assert_eq!(this.feed_blocks.len(), 1);
        assert_eq!(this.feed_rows.len(), 501);
    });
    host.update(&mut cx, |host, _, cx| {
        host.show_feed = true;
        cx.notify();
    })
    .unwrap();
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    PARSE_COUNT.set(0);
    RENDER_COUNT.set(0);
    LINE_COUNT.set(0);
    visual.simulate_input("x");
    visual.run_until_parked();
    assert_eq!(PARSE_COUNT.get(), 0);
    assert!(
        RENDER_COUNT.get() > 0 && RENDER_COUNT.get() < 100,
        "built {} code rows",
        RENDER_COUNT.get()
    );
    assert!(
        LINE_COUNT.get() > 0 && LINE_COUNT.get() < 200,
        "built {} code lines",
        LINE_COUNT.get()
    );
    workspace.update(&mut visual, |this, cx| {
        this.feed_selection = Some(FeedSelection {
            event_id: this.events[0].id.clone(),
            anchor: Position {
                row: 1999,
                offset: 0,
            },
            head: Position {
                row: 2000,
                offset: 19,
            },
        });
        this.feed_list.scroll_to(ListOffset::default());
        cx.notify();
    });
    visual.run_until_parked();
    workspace.update(&mut visual, |this, cx| {
        this.scroll_feed_to_bottom();
        cx.notify();
    });
    visual.run_until_parked();
    host.update(&mut visual, |_, window, cx| {
        let focus = workspace.read(cx).root_focus.clone();
        focus.focus(window, cx);
    })
    .unwrap();
    PARSE_COUNT.set(0);
    visual.dispatch_action(Copy);
    assert_eq!(PARSE_COUNT.get(), 0);
    assert_eq!(
        visual
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("let line_1998 = 1998;\nlet line_1999 = 199")
    );
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn feed_selection_across_paragraph_table_code_scroll_escape_and_link_drag() {
    use crate::ui::markdown::{Position, PARSE_COUNT};
    use gpui::{ListOffset, Modifiers, MouseButton, VisualTestContext};
    let (_temp, mut cx, host, workspace) = mission_workspace_test();
    let long_code = "z".repeat(540);
    workspace.update(&mut cx, |this, _| {
        let mut events: Vec<Event> = (0..40)
            .map(|index| Event {
                id: format!("{index:026}"),
                ts: chrono::DateTime::from_timestamp(1_700_000_000 + index, 0).unwrap(),
                crew_id: Some("crew".into()),
                mission_id: "mission".into(),
                kind: EventKind::Message,
                from: if index % 2 == 0 { "coder" } else { "reviewer" }.into(),
                to: Default::default(),
                signal_type: None,
                payload: serde_json::json!({"text": format!("Filler message {index}")}),
            })
            .collect();
        events.push(Event {
            id: format!("{:026}", 40),
            ts: chrono::DateTime::from_timestamp(1_700_000_040, 0).unwrap(),
            crew_id: Some("crew".into()),
            mission_id: "mission".into(),
            kind: EventKind::Message,
            from: "coder".into(),
            to: Default::default(),
            signal_type: None,
            payload: serde_json::json!({
                "text": format!(
                    "Check [the docs](https://example.com) and [local](file:///etc/hosts)\n\n| ColA | ColB |\n| --- | --- |\n| one | two |\n\n```\n{long_code}\nend\n```\n"
                )
            }),
        });
        let mut question = signal(
            "human_question",
            serde_json::json!({
                "prompt": "Approve **plan**?\nSecond line",
                "choices": ["yes", "no"]
            }),
        );
        question.id = format!("{:026}", 41);
        events.push(question);
        this.events = events;
        this.rebuild_event_projection();
    });
    host.update(&mut cx, |host, _, cx| {
        host.show_feed = true;
        cx.notify();
    })
    .unwrap();
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();

    let message_id = format!("{:026}", 40);
    let question_id = format!("{:026}", 41);

    // Select across paragraph (row 0), table header & row (rows 1..=2), code header (row 3), and code lines (rows 4..=6).
    workspace.update(&mut visual, |this, cx| {
        this.feed_selection = Some(FeedSelection {
            event_id: message_id.clone(),
            anchor: Position { row: 0, offset: 0 },
            head: Position { row: 6, offset: 3 },
        });
        this.feed_list.scroll_to(ListOffset::default());
        cx.notify();
    });
    visual.run_until_parked();
    workspace.update(&mut visual, |this, cx| {
        this.scroll_feed_to_bottom();
        cx.notify();
    });
    visual.run_until_parked();
    host.update(&mut visual, |_, window, cx| {
        let focus = workspace.read(cx).root_focus.clone();
        focus.focus(window, cx);
    })
    .unwrap();
    PARSE_COUNT.set(0);
    visual.dispatch_action(Copy);
    assert_eq!(PARSE_COUNT.get(), 0, "copy must not re-parse");
    assert_eq!(
        visual
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some(format!("Check the docs and local\nColA\tColB\none\ttwo\n{long_code}\nend").as_str())
    );

    // Escape clears the selection.
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.feed_selection.is_none());
    });

    // Question card prompt renders and copies from the cache without re-parsing.
    workspace.update(&mut visual, |this, cx| {
        this.feed_selection = Some(FeedSelection {
            event_id: question_id.clone(),
            anchor: Position { row: 0, offset: 0 },
            head: Position { row: 1, offset: 6 },
        });
        cx.notify();
    });
    visual.run_until_parked();
    PARSE_COUNT.set(0);
    visual.dispatch_action(Copy);
    assert_eq!(PARSE_COUNT.get(), 0);
    assert_eq!(
        visual
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("Approve plan?\nSecond")
    );
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();

    // Drag ending on a link opens nothing; a plain click opens https: only.
    let link_row = visual
        .debug_bounds("COMPACT_MD_TEXT 00000000000000000000000040 0")
        .unwrap();
    let start = point(link_row.left() + px(2.), link_row.center().y);
    let on_link = point(link_row.left() + px(65.), link_row.center().y);
    visual.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(on_link, Some(MouseButton::Left), Modifiers::default());
    visual.simulate_mouse_up(on_link, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    assert_eq!(visual.cx.opened_url(), None);

    // Drag across paragraph -> table row -> code line and copy from cache.
    let table_row = visual
        .debug_bounds("COMPACT_MD_TABLE_ROW 00000000000000000000000040 2")
        .unwrap();
    let code_row = visual
        .debug_bounds("COMPACT_MD_TEXT 00000000000000000000000040 6")
        .unwrap();
    let drag_end = point(code_row.left() + px(30.), code_row.center().y);
    visual.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(
        table_row.center(),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    visual.simulate_mouse_move(drag_end, Some(MouseButton::Left), Modifiers::default());
    visual.simulate_mouse_up(drag_end, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    PARSE_COUNT.set(0);
    visual.dispatch_action(Copy);
    assert_eq!(PARSE_COUNT.get(), 0);
    let dragged_copy = visual
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("dragged selection copied");
    assert!(
        dragged_copy.contains("ColA\tColB\none\ttwo\n") && dragged_copy.contains(&long_code),
        "unexpected dragged copy: {dragged_copy:?}"
    );
    assert_eq!(visual.cx.opened_url(), None);

    // Dragging starting and ending inside the https: link opens nothing;
    // clicking the file: link opens nothing; clicking the https: link opens it.
    let link_start = point(link_row.left() + px(48.), link_row.center().y);
    let link_end = point(link_row.left() + px(82.), link_row.center().y);
    visual.simulate_mouse_down(link_start, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(link_end, Some(MouseButton::Left), Modifiers::default());
    visual.simulate_mouse_up(link_end, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    assert_eq!(visual.cx.opened_url(), None);

    visual.simulate_click(
        point(link_row.left() + px(140.), link_row.center().y),
        Modifiers::default(),
    );
    visual.run_until_parked();
    assert_eq!(visual.cx.opened_url(), None);

    visual.simulate_click(
        point(link_row.left() + px(60.), link_row.center().y),
        Modifiers::default(),
    );
    visual.run_until_parked();
    assert_eq!(
        visual.cx.opened_url().as_deref(),
        Some("https://example.com")
    );

    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn feed_scrollbar_hides_for_fitting_content_and_resets_with_the_mission() {
    use gpui::VisualTestContext;
    let (_temp, cx, host, workspace) = mission_workspace_test();
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    host.update(&mut visual, |host, _, cx| {
        host.show_feed = true;
        cx.notify();
    })
    .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("MISSION_FEED_SCROLLBAR").is_none());
    workspace.update(&mut visual, |this, cx| {
        this.append_event(long_feed_events().remove(0));
        cx.notify();
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("MISSION_FEED_SCROLLBAR").is_none());
    workspace.update(&mut visual, |this, cx| {
        this.events = long_feed_events();
        this.rebuild_event_projection();
        cx.notify();
    });
    visual.run_until_parked();
    let gutter = visual.debug_bounds("MISSION_FEED_SCROLLBAR").unwrap();
    assert_eq!(gutter.size.width, px(10.));
    assert_eq!(gutter.right(), px(600.));
    workspace.update(&mut visual, |this, cx| {
        this.prepare_mission("short-mission".into());
        this.events = long_feed_events().into_iter().take(1).collect();
        this.rebuild_event_projection();
        cx.notify();
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("MISSION_FEED_SCROLLBAR").is_none());
    workspace.read_with(&visual, |this, _| {
        let metrics = this.feed_scrollbar_metrics.get();
        assert!(metrics.content < metrics.viewport);
        assert_eq!(metrics.position, 0.);
    });
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn feed_scrollbar_reaches_blocks_appended_while_reading_history() {
    use gpui::{Modifiers, MouseButton, VisualTestContext};
    let (_temp, mut cx, host, workspace) = mission_workspace_test();
    workspace.update(&mut cx, |this, _| {
        this.events = long_feed_events().into_iter().take(50).collect();
        this.rebuild_event_projection();
    });
    host.update(&mut cx, |host, _, cx| {
        host.show_feed = true;
        cx.notify();
    })
    .unwrap();
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    let gutter = visual.debug_bounds("MISSION_FEED_SCROLLBAR").unwrap();
    let track_top = point(gutter.center().x, gutter.top() + px(2.));
    visual.simulate_click(track_top, Modifiers::default());
    visual.run_until_parked();
    workspace.update(&mut visual, |this, cx| {
        let top = this.feed_list.logical_scroll_top();
        assert_eq!(top.item_ix, 0);
        let old_height = this.feed_list.max_offset_for_scrollbar().y;
        let mut joined = this.events.last().unwrap().clone();
        joined.id.push('a');
        this.append_event(joined);
        assert_eq!(this.feed_list.max_offset_for_scrollbar().y, old_height);
        for event in long_feed_events().into_iter().skip(50).take(1000) {
            this.append_event(event);
        }
        assert_eq!(this.feed_blocks.len(), 1050);
        assert!(this.feed_needs_height_hints.get());
        assert!(this.feed_has_new_messages);
        assert_eq!(this.feed_list.logical_scroll_top().item_ix, top.item_ix);
        assert_eq!(
            this.feed_list.logical_scroll_top().offset_in_item,
            top.offset_in_item
        );
        this.feed_block_builds.set(0);
        cx.notify();
    });
    visual.run_until_parked();
    let late = point(gutter.center().x, gutter.top() + gutter.size.height * 0.9);
    visual.simulate_click(late, Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(!this.feed_needs_height_hints.get());
        assert!(this.feed_list.logical_scroll_top().item_ix > 800);
        assert!(this.feed_scrollbar_metrics.get().content > 90_000.);
        assert!(this.feed_block_builds.get() < 100);
    });
    visual.simulate_click(track_top, Modifiers::default());
    visual.run_until_parked();
    let grab = point(gutter.center().x, gutter.top() + px(10.));
    visual.simulate_mouse_down(grab, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(late, Some(MouseButton::Left), Modifiers::default());
    visual.simulate_mouse_move(
        late + point(px(0.), px(2.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.feed_list.is_scrollbar_dragging());
        assert!(this.feed_list.logical_scroll_top().item_ix > 800);
        assert!(this.feed_has_new_messages);
    });
    visual.simulate_mouse_up(late, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn feed_scrollbar_track_drag_and_wheel_reach_unmeasured_history() {
    use gpui::{Modifiers, MouseButton, VisualTestContext};
    let (_temp, mut cx, host, workspace) = mission_workspace_test();
    workspace.update(&mut cx, |this, _| {
        this.events = long_feed_events();
        this.rebuild_event_projection();
    });
    host.update(&mut cx, |host, _, cx| {
        host.show_feed = true;
        cx.notify();
    })
    .unwrap();
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    let gutter = visual.debug_bounds("MISSION_FEED_SCROLLBAR").unwrap();
    workspace.read_with(&visual, |this, _| {
        let metrics = this.feed_scrollbar_metrics.get();
        assert!(metrics.content > 100_000.);
        assert!((metrics.position - (metrics.content - metrics.viewport)).abs() < 1.);
        assert!(this.feed_block_builds.get() < 100);
    });
    host.update(&mut visual, |host, _, cx| {
        host.width = 800.;
        cx.notify();
    })
    .unwrap();
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.feed_list.viewport_bounds().size.width, px(752.));
        assert!(this.feed_scrollbar_metrics.get().content > 100_000.);
    });
    host.update(&mut visual, |host, _, cx| {
        host.width = 600.;
        cx.notify();
    })
    .unwrap();
    visual.run_until_parked();
    visual.simulate_click(gutter.center(), Modifiers::default());
    visual.run_until_parked();
    let top = workspace.read_with(&visual, |this, _| {
        let top = this.feed_list.logical_scroll_top();
        let mid_low = this.feed_rows.len() * 4 / 10;
        let mid_high = this.feed_rows.len() * 6 / 10;
        assert!(
            (mid_low..mid_high).contains(&top.item_ix),
            "track click reached {:?}",
            top
        );
        assert!(!this.feed_list.is_following_tail());
        assert!(!this.feed_list.is_scrollbar_dragging());
        top
    });
    visual.simulate_event(gpui::ScrollEvent {
        position: gutter.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(100.))),
        ..Default::default()
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        let next = this.feed_list.logical_scroll_top();
        assert!(
            next.item_ix < top.item_ix
                || (next.item_ix == top.item_ix && next.offset_in_item < top.offset_in_item)
        );
    });
    let track_top = point(gutter.center().x, gutter.top() + px(2.));
    visual.simulate_click(track_top, Modifiers::default());
    visual.run_until_parked();
    workspace.update(&mut visual, |this, _| {
        assert_eq!(this.feed_list.logical_scroll_top().item_ix, 0);
        this.feed_selection = Some(FeedSelection {
            event_id: this.events[0].id.clone(),
            anchor: crate::ui::markdown::Position { row: 0, offset: 0 },
            head: crate::ui::markdown::Position { row: 0, offset: 3 },
        });
    });
    let grab = point(gutter.center().x, gutter.top() + px(10.));
    visual.simulate_mouse_down(grab, MouseButton::Left, Modifiers::default());
    let content = workspace.read_with(&visual, |this, _| {
        assert!(this.feed_list.is_scrollbar_dragging());
        this.measure_feed_scrollbar().content
    });
    visual.simulate_mouse_move(
        gutter.center(),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    visual.simulate_mouse_move(
        gutter.center() + point(px(0.), px(10.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!((this.measure_feed_scrollbar().content - content).abs() < 1.);
        assert!(this.feed_list.logical_scroll_top().item_ix > 100);
        // Each 5-row message spans two 4-row chunks plus one coalesced
        // background highlight repaint per newly visited region across the
        // initial draw, two resizes, track clicks, wheel and drag (~215 vs
        // 4,000 total rows).
        let builds = this.feed_block_builds.get();
        assert!(builds < 300, "feed_block_builds={builds}");
    });
    let bottom = point(gutter.center().x, gutter.bottom() - px(10.));
    visual.simulate_mouse_move(bottom, Some(MouseButton::Left), Modifiers::default());
    visual.run_until_parked();
    visual.simulate_mouse_up(
        bottom - point(px(30.), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(!this.feed_list.is_scrollbar_dragging());
        assert!(this.feed_list.is_following_tail());
        assert!(this.feed_selection.is_some());
    });
    visual.simulate_click(track_top, Modifiers::default());
    visual.run_until_parked();
    let top = workspace.read_with(&visual, |this, _| this.feed_list.logical_scroll_top());
    workspace.update(&mut visual, |this, cx| {
        let mut event = this.events.last().unwrap().clone();
        event.id = format!("{:026}", 2000);
        this.append_event(event);
        assert!(this.feed_has_new_messages);
        assert_eq!(this.feed_list.logical_scroll_top().item_ix, top.item_ix);
        assert_eq!(
            this.feed_list.logical_scroll_top().offset_in_item,
            top.offset_in_item
        );
        cx.notify();
    });
    visual.run_until_parked();
    visual.simulate_click(bottom + point(px(0.), px(8.)), Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.feed_list.is_following_tail());
        assert!(!this.feed_has_new_messages);
    });
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn remote_control_covers_the_mission_below_tabs_then_returns_to_pane_cards() {
    use crate::terminal::control::tests::{attached, change, mirror_named, remote};
    use gpui::Focusable;
    use runner_core::protocol::terminal::TerminalController;
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store)
    });
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    let (left_host, left, left_tx, left_ack) = mirror_named("mission-agent", remote());
    let (right_host, right, right_tx, right_ack) = mirror_named("mission-shell", remote());
    let (drawer_host, drawer, drawer_tx, drawer_ack) = mirror_named("busy-shell", remote());
    let window = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "mission-control-test".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = AppRoute::Mission("test-mission".into());
        root.mission_workspace.update(cx, |workspace, cx| {
            workspace.active = true;
            workspace.warning = Some("Visible warning above the member tabs".into());
            workspace.mission_id = Some("test-mission".into());
            workspace.mission =
                Some(runner_daemon::ops::mission::mission_get(&core, "test-mission").unwrap());
            workspace.roster =
                runner_daemon::ops::mission::mission_roster(&core, "test-mission").unwrap();
            workspace.sessions =
                runner_daemon::ops::session::session_list(&core, "test-mission").unwrap();
            workspace.open_tabs = vec!["mission-agent".into(), "mission-shell".into()];
            workspace
                .attached
                .insert("mission-agent".into(), attached(left, cx, 101, 30));
            workspace
                .attached
                .insert("mission-shell".into(), attached(right, cx, 72, 45));
            workspace.layout.drawer.add("busy-shell".into());
            workspace.layout.drawer.set_open(true);
            workspace
                .attached
                .insert("busy-shell".into(), attached(drawer, cx, 80, 20));
        });
        root
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
    let overlay = visual
        .debug_bounds("REMOTE_CONTROL_OVERLAY_take-back-mission")
        .unwrap();
    let header = visual.debug_bounds("MISSION_HEADER").unwrap();
    let notice = visual.debug_bounds("NOTICE_BANNER").unwrap();
    let tabs = visual.debug_bounds("MISSION_TABS").unwrap();
    assert!(notice.top() >= header.bottom());
    assert!(tabs.top() >= notice.bottom());
    assert!(overlay.top() >= tabs.bottom());
    assert!(left_host.sizes.lock().unwrap().is_empty());
    assert!(right_host.sizes.lock().unwrap().is_empty());
    assert!(drawer_host.sizes.lock().unwrap().is_empty());
    let drawer = visual.debug_bounds("MISSION_DRAWER_ISLAND").unwrap();
    let drawer_cover = visual
        .debug_bounds("REMOTE_CONTROL_OVERLAY_take-back-mission-drawer")
        .unwrap();
    assert!(overlay.bottom() <= drawer.top());
    assert!(drawer_cover.top() >= drawer.top());
    assert!(drawer_cover.bottom() <= drawer.bottom());
    assert!((drawer_cover.size.height - drawer.size.height).abs() <= px(2.));
    window
        .update(&mut visual, |root, window, cx| {
            root.mission_workspace.update(cx, |workspace, cx| {
                workspace
                    .action_menu
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            });
        })
        .unwrap();
    visual.simulate_keystrokes("enter");
    visual.simulate_keystrokes("down");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    window
        .update(&mut visual, |root, window, cx| {
            root.mission_workspace.update(cx, |workspace, cx| {
                assert!(workspace.rename_modal.is_some());
                workspace.rename_modal = None;
                workspace.root_focus.focus(window, cx);
                cx.notify();
            });
        })
        .unwrap();
    for host in [&left_host, &right_host, &drawer_host] {
        assert!(!host.calls.lock().unwrap().iter().any(|request| matches!(
            request,
            runner_core::protocol::Request::terminal_take_back { .. }
        )));
    }
    for zoom in [1., 1.3] {
        visual.simulate_resize(gpui::size(px(1600.), px(800.)));
        window
            .update(&mut visual, |root, window, cx| {
                root.set_zoom(zoom, window, cx);
                root.mission_workspace.update(cx, |workspace, cx| {
                    workspace.layout.drawer.set_height(600.);
                    cx.notify();
                });
            })
            .unwrap();
        visual.run_until_parked();
        let column = visual.debug_bounds("MISSION_COLUMN").unwrap();
        let tabs = visual.debug_bounds("MISSION_TABS").unwrap();
        let card = visual.debug_bounds("REMOTE_CONTROL_CARD").unwrap();
        let action = visual.debug_bounds("REMOTE_CONTROL_ACTION").unwrap();
        assert!(
            card.top() >= tabs.bottom(),
            "card obscures tabs at zoom {zoom}"
        );
        assert!(column.contains(&card.origin));
        assert!(column.contains(&card.bottom_right()));
        assert!(card.contains(&action.origin));
        assert!(card.contains(&action.bottom_right()));
        let hosts = [&left_host, &right_host, &drawer_host];
        let before = hosts.map(|host| {
            host.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|request| {
                    matches!(
                        request,
                        runner_core::protocol::Request::terminal_take_back { .. }
                    )
                })
                .count()
        });
        visual.simulate_click(action.center(), gpui::Modifiers::default());
        for (host, before) in hosts.into_iter().zip(before) {
            assert_eq!(
                host.calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|request| matches!(
                        request,
                        runner_core::protocol::Request::terminal_take_back { .. }
                    ))
                    .count(),
                before + 1
            );
        }
    }
    visual.simulate_resize(gpui::size(px(1600.), px(1000.)));
    window
        .update(&mut visual, |root, window, cx| {
            root.set_zoom(1., window, cx);
            root.mission_workspace.update(cx, |workspace, cx| {
                workspace.layout.drawer.set_height(280.);
                cx.notify();
            });
        })
        .unwrap();
    visual.run_until_parked();
    window
        .update(&mut cx, |root, window, cx| {
            root.mission_workspace.update(cx, |workspace, cx| {
                workspace.composer.draft = "Keep this draft while covered".into();
                workspace.composer_input.update(cx, |input, cx| {
                    input.reset("Keep this draft while covered", cx)
                });
                workspace.on_mission_composer_key_down("enter", false, window, cx);
                assert!(!workspace.composer_posting);
                assert_eq!(workspace.composer.draft, "Keep this draft while covered");
            })
        })
        .unwrap();
    change(&left_tx, &left_ack, 1, TerminalController::None);
    change(&right_tx, &right_ack, 1, TerminalController::None);
    change(&drawer_tx, &drawer_ack, 1, TerminalController::None);
    window
        .update(&mut cx, |root, _, cx| {
            root.mission_workspace.update(cx, |_, cx| cx.notify())
        })
        .unwrap();
    cx.run_until_parked();
    assert!(visual
        .debug_bounds("REMOTE_CONTROL_OVERLAY_take-back-mission")
        .is_some());
    assert!(visual
        .debug_bounds("REMOTE_CONTROL_OVERLAY_take-back-mission-drawer")
        .is_some());
    window
        .update(&mut visual, |root, window, cx| {
            root.mission_workspace.update(cx, |workspace, cx| {
                workspace.drawer_focus.focus(window, cx);
            });
        })
        .unwrap();
    visual.simulate_keystrokes("enter");
    for host in [&left_host, &right_host, &drawer_host] {
        assert!(host.calls.lock().unwrap().iter().any(|request| matches!(
            request,
            runner_core::protocol::Request::terminal_take_back { .. }
        )));
    }
    change(&left_tx, &left_ack, 2, TerminalController::Desktop);
    window
        .update(&mut cx, |root, _, cx| {
            root.mission_workspace.update(cx, |workspace, cx| {
                workspace.active_tab = MissionTab::Session("mission-shell".into());
                cx.notify();
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert!(visual
        .debug_bounds("REMOTE_CONTROL_OVERLAY_take-back-mission")
        .is_none());
    assert!(visual
        .debug_bounds("REMOTE_CONTROL_OVERLAY_take-back-slot-mission-shell")
        .is_some());
    assert!(visual
        .debug_bounds("REMOTE_CONTROL_OVERLAY_take-back-mission-drawer")
        .is_none());
    change(&right_tx, &right_ack, 2, TerminalController::Desktop);
    change(&drawer_tx, &drawer_ack, 2, TerminalController::Desktop);
    store.update(&mut cx, |store, cx| {
        store
            .terminal_updates
            .update(cx, |_, cx| cx.emit("mission-shell".to_string()))
    });
    window
        .update(&mut cx, |root, _, cx| {
            root.mission_workspace.update(cx, |_, cx| cx.notify())
        })
        .unwrap();
    cx.run_until_parked();
    assert!(visual.debug_bounds("REMOTE_CONTROL_CARD").is_none());
    assert!(!right_host.sizes.lock().unwrap().is_empty());
}

struct MissionEntryTrace {
    base: Arc<crate::surfaces::remote_client::tests::Fake>,
    calls: std::sync::Mutex<Vec<serde_json::Value>>,
    armed: std::sync::atomic::AtomicBool,
}

impl runner_core::protocol::Transport for MissionEntryTrace {
    fn call(
        &self,
        request: runner_core::protocol::Request,
    ) -> runner_core::protocol::ClientResult<runner_core::protocol::Response> {
        use std::sync::atomic::Ordering;
        let value = serde_json::to_value(&request).unwrap();
        self.calls.lock().unwrap().push(value.clone());
        let local = [
            "mission_stop_impl",
            "session_list",
            "mission_archive_impl",
            "mission_rename_impl",
            "session_kill",
        ]
        .iter()
        .any(|method| value.get(method).is_some());
        let remote = value["remote_request"]["request"]["type"]
            .as_str()
            .is_some_and(|method| {
                matches!(
                    method,
                    "stopMission" | "resumeMission" | "archive" | "rename" | "stop"
                )
            });
        if self.armed.load(Ordering::SeqCst) && (local || remote) {
            return Err(runner_core::protocol::ClientError::msg(
                "mission entry failure",
            ));
        }
        self.base.call(request)
    }
    fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
        self.base.subscribe()
    }
    fn attach_remote(
        &self,
        config: runner_core::remote::RemoteTerminal,
    ) -> runner_core::protocol::ClientResult<runner_core::protocol::terminal::TerminalAttachment>
    {
        self.base.attach_remote(config)
    }
}

#[test]
fn mission_lifecycle_entry_points_dispatch_once_for_both_targets() {
    use crate::surfaces::remote_client::{tests as remote_test, RemoteItem};
    use runner_core::protocol::DaemonClient;
    use std::sync::atomic::{AtomicBool, Ordering};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for remote in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
        remote_test::enable_missions(&fake);
        fake.set_overview(serde_json::from_value(serde_json::json!({"projects":[],"tabs":[],"missions":[{"id":"remote-mission","project":null,"title":"Host mission","crew":"Duo","state":"running","pinned":false,"members":[{"handle":"coder","session":"member","lead":false}],"asks":[],"shells":[]}],"sessions":{"member":{"kind":"missionSlot","title":"Member","runtime":"codex","status":"stopped","unread":false}}})).unwrap());
        let trace = Arc::new(MissionEntryTrace {
            base: fake,
            calls: Default::default(),
            armed: AtomicBool::new(false),
        });
        store.update(&mut cx, |store, _| {
            store.client = DaemonClient::new(trace.clone());
            if !remote {
                crate::app_store::seed_mixed_working_sessions(store);
            }
        });
        let window = remote_test::add_window(&mut cx, &store, temp.path(), "mission-entry-points");
        if remote {
            window
                .update(&mut cx, |root, window, cx| {
                    remote_test::choose_host(root, window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            window
                .update(&mut cx, |root, _, cx| {
                    root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
                })
                .unwrap();
        }
        cx.run_until_parked();
        let workspace = window
            .read_with(&cx, |root, _| {
                if remote {
                    root.remote_client
                        .workspace()
                        .unwrap()
                        .mission_workspace
                        .clone()
                } else {
                    root.mission_workspace.clone()
                }
            })
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
        for (command, local_method, remote_method) in [
            ("stop", "mission_stop_impl", "stopMission"),
            ("resume", "session_list", "resumeMission"),
            ("archive", "mission_archive_impl", "archive"),
            ("rename", "mission_rename_impl", "rename"),
            ("slot", "session_kill", "stop"),
        ] {
            workspace.update(&mut cx, |workspace, cx| {
                if !remote {
                    let core = &workspace.app_store.read(cx).test_core;
                    workspace.mission = Some(
                        runner_daemon::ops::mission::mission_get(core, "test-mission").unwrap(),
                    );
                    workspace.sessions =
                        runner_daemon::ops::session::session_list(core, "test-mission").unwrap();
                    for row in &mut workspace.sessions {
                        row.session.status = SessionStatus::Stopped;
                    }
                    workspace.mission_id = Some("test-mission".into());
                    workspace.active = true;
                }
                workspace.stopping = false;
                workspace.resuming = false;
                workspace.archiving = false;
                workspace.slot_actions.clear();
                workspace.transitions.clear();
                workspace.error = None;
                if command == "rename" {
                    let input =
                        cx.new(|cx| TextField::new(cx.focus_handle(), "New title", "", false));
                    workspace.rename_modal = Some(MissionRenameModal {
                        mission_id: workspace.mission_id.clone().unwrap(),
                        original: "Old title".into(),
                        input,
                        close_focus: cx.focus_handle(),
                        cancel_focus: cx.focus_handle(),
                        submit_focus: cx.focus_handle(),
                        submitting: false,
                        error: None,
                    });
                }
            });
            trace.calls.lock().unwrap().clear();
            trace.armed.store(true, Ordering::SeqCst);
            visual.update(|window, cx| {
                workspace.update(cx, |workspace, cx| match command {
                    "stop" => workspace.stop_open_mission(window, cx),
                    "resume" => workspace.resume_open_mission(window, cx),
                    "archive" => workspace.archive_open_mission(window, cx),
                    "rename" => workspace.submit_mission_rename(window, cx),
                    _ => workspace.act_on_slot(
                        if remote { "member" } else { "mission-agent" },
                        SessionControlKind::Stop,
                        window,
                        cx,
                    ),
                })
            });
            cx.run_until_parked();
            trace.armed.store(false, Ordering::SeqCst);
            let calls = trace.calls.lock().unwrap();
            let count = calls
                .iter()
                .filter(|value| {
                    if remote {
                        value["remote_request"]["request"]["type"] == remote_method
                    } else {
                        value.get(local_method).is_some()
                    }
                })
                .count();
            assert_eq!(count, 1, "{remote}/{command}: {calls:?}");
            workspace.read_with(&cx, |workspace, _| {
                let error = if command == "rename" {
                    workspace
                        .rename_modal
                        .as_ref()
                        .and_then(|modal| modal.error.clone())
                } else {
                    workspace
                        .error
                        .as_ref()
                        .map(|error| error.resolve().to_string())
                };
                assert!(
                    error
                        .as_deref()
                        .is_some_and(|error| error.contains("mission entry failure")),
                    "{remote}/{command}: {error:?}"
                );
            });
        }
    }
}

#[test]
fn leaving_the_mission_closes_find_in_its_slot_and_follows_late_output() {
    use crate::terminal::control::tests::attached;
    use crate::terminal::find::tests::{count, injecting_mirror, needle_output, open_with_query};
    use std::sync::atomic::Ordering;
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store)
    });
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    let (slot, armed) = injecting_mirror();
    slot.test_feed(1, &needle_output());
    let slot_terminal = slot.clone();
    let window = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "find-mission-leave".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = AppRoute::Mission("test-mission".into());
        root.mission_workspace.update(cx, |workspace, cx| {
            workspace.active = true;
            workspace.mission_id = Some("test-mission".into());
            workspace.mission =
                Some(runner_daemon::ops::mission::mission_get(&core, "test-mission").unwrap());
            workspace.roster =
                runner_daemon::ops::mission::mission_roster(&core, "test-mission").unwrap();
            workspace.sessions =
                runner_daemon::ops::session::session_list(&core, "test-mission").unwrap();
            workspace.open_tabs = vec!["mission-shell".into()];
            workspace
                .attached
                .insert("mission-shell".into(), attached(slot_terminal, cx, 80, 24));
        });
        root
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
    visual.run_until_parked();
    let find = window
        .update(&mut cx, |root, window, cx| {
            let find = root.mission_workspace.read(cx).attached["mission-shell"]
                .terminal_find
                .clone();
            open_with_query(&find, "needle", window, cx);
            find
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(count(&mut visual, &find).as_deref(), Some("1 of 1"));
    assert!(slot.scroll_state().display_offset > 0);
    let mark = slot.scroll_mark();
    let weak = find.downgrade();
    drop(find);
    armed.store(true, Ordering::SeqCst);
    window
        .update(&mut cx, |root, _, cx| root.set_route(AppRoute::Roles, cx))
        .unwrap();
    visual.run_until_parked();
    window
        .read_with(&cx, |root, cx| {
            assert!(root.mission_workspace.read(cx).attached.is_empty());
        })
        .unwrap();
    assert!(weak.upgrade().is_none());
    assert_eq!(mark.moved_since(slot.scroll_mark()), Some(3));
    assert_eq!(slot.scroll_state().display_offset, 0);
    window
        .update(&mut cx, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn leaving_a_slot_tab_closes_find_in_its_terminal() {
    use crate::terminal::control::tests::attached;
    use crate::terminal::find::tests::{count, injecting_mirror, needle_output, open_with_query};
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store)
    });
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    let (slot, _) = injecting_mirror();
    slot.test_feed(1, &needle_output());
    let (other, _) = injecting_mirror();
    let (slot_terminal, other_terminal) = (slot.clone(), other.clone());
    let window = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "find-slot-switch".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = AppRoute::Mission("test-mission".into());
        root.mission_workspace.update(cx, |workspace, cx| {
            workspace.active = true;
            workspace.mission_id = Some("test-mission".into());
            workspace.mission =
                Some(runner_daemon::ops::mission::mission_get(&core, "test-mission").unwrap());
            workspace.roster =
                runner_daemon::ops::mission::mission_roster(&core, "test-mission").unwrap();
            workspace.sessions =
                runner_daemon::ops::session::session_list(&core, "test-mission").unwrap();
            workspace.open_tabs = vec!["mission-shell".into(), "mission-agent".into()];
            workspace.active_tab = MissionTab::Session("mission-shell".into());
            workspace
                .attached
                .insert("mission-shell".into(), attached(slot_terminal, cx, 80, 24));
            workspace
                .attached
                .insert("mission-agent".into(), attached(other_terminal, cx, 80, 24));
        });
        root
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
    visual.run_until_parked();
    let (workspace, find) = window
        .read_with(&cx, |root, cx| {
            let workspace = root.mission_workspace.clone();
            let find = workspace.read(cx).attached["mission-shell"]
                .terminal_find
                .clone();
            (workspace, find)
        })
        .unwrap();

    for leave_to_feed in [false, true] {
        workspace.update_in(&mut visual, |workspace, window, cx| {
            workspace.select_mission_session("mission-shell", window, cx);
            open_with_query(&find, "needle", window, cx);
        });
        visual.run_until_parked();
        assert_eq!(count(&mut visual, &find).as_deref(), Some("1 of 1"));
        assert!(slot.scroll_state().display_offset > 0);

        workspace.update_in(&mut visual, |workspace, window, cx| {
            if leave_to_feed {
                workspace.select_mission_feed(window, cx);
            } else {
                workspace.select_mission_session("mission-agent", window, cx);
            }
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |workspace, _| {
            assert!(workspace.attached.contains_key("mission-shell"));
        });
        assert_eq!(count(&mut visual, &find), None);
        assert_eq!(slot.scroll_state().display_offset, 0);
    }
    window
        .update(&mut cx, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn osc52_visibility_follows_the_mission_terminals_a_window_draws() {
    use gpui::{TestAppContext, WeakEntity};
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store)
    });
    let mission = store.read_with(&cx, |store, _| {
        runner_daemon::ops::mission::mission_get(&store.test_core, "test-mission").unwrap()
    });
    let host = cx.add_window(|_, _| gpui::Empty);
    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    let visual = &mut visual;
    let workspace = visual.update(|window, cx| {
        cx.new(|cx| {
            let mut workspace = MissionWorkspace::new(
                "osc52".into(),
                WeakEntity::new_invalid(),
                store.clone(),
                window,
                cx,
            );
            workspace.active = true;
            workspace.loading = false;
            workspace.mission_id = Some("test-mission".into());
            workspace.mission = Some(mission);
            workspace.sessions = vec![SessionRow {
                session: runner_core::protocol::model::Session {
                    id: "member".into(),
                    mission_id: Some("test-mission".into()),
                    role_id: "test-shell-role".into(),
                    slot_id: None,
                    cwd: None,
                    status: SessionStatus::Running,
                    pid: None,
                    started_at: None,
                    stopped_at: None,
                },
                handle: "coder".into(),
                role_handle: String::new(),
                lead: true,
                runtime: "shell".into(),
                agent_model: None,
                agent_effort: None,
                live_title: None,
                agent_session_key: None,
            }];
            workspace.active_tab = MissionTab::Session("member".into());
            workspace.layout.drawer.add("shell".into());
            workspace.layout.drawer.set_open(true);
            workspace
        })
    });
    let mut shown = |change: &dyn Fn(&mut MissionWorkspace), mission: &str| {
        workspace.update(visual, |workspace, _| change(workspace));
        workspace.read_with(visual, |workspace, cx| {
            workspace.shown_terminals(mission, cx)
        })
    };

    assert_eq!(shown(&|_| {}, "test-mission"), ["member", "shell"]);
    assert!(shown(&|_| {}, "other-mission").is_empty());
    assert!(
        shown(&|workspace| workspace.secondary = true, "test-mission").is_empty(),
        "a secondary window draws the feed and no drawer, whatever tab it remembers"
    );
    assert_eq!(
        shown(
            &|workspace| {
                workspace.secondary = false;
                workspace.active_tab = MissionTab::Session("departed".into());
            },
            "test-mission"
        ),
        ["shell"],
        "a missing member falls back to the feed"
    );
    assert!(shown(
        &|workspace| {
            workspace.active_tab = MissionTab::Feed;
            workspace.layout.drawer.set_open(false);
        },
        "test-mission"
    )
    .is_empty());
    assert_eq!(
        shown(
            &|workspace| {
                workspace.active_tab = MissionTab::Session("member".into());
                workspace.layout.drawer.set_open(true);
            },
            "test-mission"
        ),
        ["member", "shell"]
    );
    assert!(shown(&|workspace| workspace.loading = true, "test-mission").is_empty());
    assert!(shown(
        &|workspace| {
            workspace.loading = false;
            workspace.active = false;
        },
        "test-mission"
    )
    .is_empty());
}

#[test]
fn a_file_link_in_a_mission_terminal_opens_the_rails_preview() {
    use crate::file_links::{FileLinkTarget, LAUNCHED};
    use crate::surfaces::side_panel::PanelSubject;
    use gpui::{TestAppContext, VisualTestContext};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("worktree");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("lib.rs"), "one\ntwo\nthree\n").unwrap();
    let outside = temp.path().join("outside.txt");
    std::fs::write(&outside, "x\n").unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store);
        store
            .test_core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE missions SET files_root = ?1 WHERE id = 'test-mission'",
                [root.to_string_lossy()],
            )
            .unwrap();
    });
    let mission = store.read_with(&cx, |store, _| {
        runner_daemon::ops::mission::mission_get(&store.test_core, "test-mission").unwrap()
    });
    let host = cx.add_window(|window, cx| {
        let workspace = cx.new(|cx| {
            let mut workspace = MissionWorkspace::new(
                "link-preview".into(),
                WeakEntity::new_invalid(),
                store.clone(),
                window,
                cx,
            );
            workspace.mission = Some(mission);
            workspace
        });
        MissionRailLayoutTest {
            workspace,
            width: None,
        }
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    let workspace = host
        .read_with(&visual, |host, _| host.workspace.clone())
        .unwrap();
    workspace.update(&mut visual, |workspace, cx| {
        workspace.set_rail_open(false, cx)
    });
    assert!(!workspace.read_with(&visual, |workspace, cx| workspace.rail_is_open(cx)));
    let wait = |visual: &mut VisualTestContext, ready: &dyn Fn(&mut VisualTestContext) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready(visual) {
            assert!(std::time::Instant::now() < deadline, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(20));
            visual.run_until_parked();
        }
    };
    let click = |visual: &mut VisualTestContext, path: std::path::PathBuf| {
        LAUNCHED.with(|launched| launched.borrow_mut().clear());
        let route = workspace.update(visual, |workspace, cx| workspace.file_link_route(cx));
        let target = FileLinkTarget {
            path,
            line: Some(2),
            column: None,
        };
        visual.update(|_, cx| route.open(target, cx));
        visual.run_until_parked();
    };
    let launched = || LAUNCHED.with(|launched| launched.borrow().len());

    // The member terminals and the drawer's shell share the rail's route.
    click(&mut visual, outside.clone());
    wait(&mut visual, &|_| launched() == 1);
    assert!(!workspace.read_with(&visual, |workspace, cx| workspace.rail_is_open(cx)));

    click(&mut visual, root.join("lib.rs"));
    let subject = PanelSubject::Mission("test-mission".into());
    wait(&mut visual, &|visual| {
        workspace.read_with(visual, |workspace, cx| {
            workspace
                .rail_files
                .view(&subject)
                .and_then(|view| view.read(cx).active_tab(cx))
                == Some("lib.rs".into())
        })
    });
    assert_eq!(launched(), 0);
    assert!(workspace.read_with(&visual, |workspace, cx| workspace.rail_is_open(cx)));
    assert_eq!(
        workspace.read_with(&visual, |workspace, _| workspace.rail_surfaces.selection),
        crate::surfaces::side_panel::PanelSelection::new(PanelViewId::Files),
        "opening a file link selects Files"
    );

    // #1421: Opening a file widens the panel once; switching the right rail to
    // Sessions keeps the unified viewer open and widened (0 px width jump).
    let widened = |visual: &mut gpui::VisualTestContext| {
        workspace.read_with(visual, |workspace, cx| {
            workspace.rail_layout(cx).widened.is_some()
        })
    };
    let layout_width = |visual: &mut gpui::VisualTestContext| {
        workspace.read_with(visual, |workspace, cx| workspace.rail_layout(cx).width)
    };
    assert!(widened(&mut visual));
    let opened_width = layout_width(&mut visual);
    workspace.update(&mut visual, |workspace, _| {
        workspace.rail_surfaces.select(PanelViewId::Sessions)
    });
    assert!(
        widened(&mut visual),
        "Sessions keeps unified viewer open and widened"
    );
    assert_eq!(
        layout_width(&mut visual),
        opened_width,
        "switching Files -> Sessions must produce 0 px width jump"
    );
    workspace.update(&mut visual, |workspace, _| {
        workspace.rail_surfaces.select(PanelViewId::Files);
    });
    assert!(
        widened(&mut visual),
        "returning to Files keeps unified viewer widened"
    );
    assert_eq!(layout_width(&mut visual), opened_width);
    workspace.update(&mut visual, |workspace, _| {
        workspace.rail_surfaces.reset(Default::default());
    });
    assert!(
        widened(&mut visual),
        "reset to Sessions keeps open viewer widened"
    );
    assert!(workspace.read_with(&visual, |workspace, cx| {
        workspace
            .rail_files
            .view(&subject)
            .and_then(|view| view.read(cx).active_tab(cx))
            == Some("lib.rs".into())
    }));
    workspace.update(&mut visual, |workspace, _| {
        workspace.rail_surfaces.select(PanelViewId::Files)
    });
    assert!(widened(&mut visual), "Files reopened, with its file");

    // ⇧⌘O opens the collapsed rail on Files, with its finder.
    workspace.update(&mut visual, |workspace, cx| {
        workspace.set_rail_open(false, cx)
    });
    assert!(!workspace.read_with(&visual, |workspace, cx| workspace.rail_is_open(cx)));
    visual.run_until_parked();
    workspace.update_in(&mut visual, |workspace, window, cx| {
        workspace.search_files(window, cx)
    });
    wait(&mut visual, &|visual| {
        visual.debug_bounds("FILES_FINDER_ITEM 0").is_some()
    });
    assert!(workspace.read_with(&visual, |workspace, cx| workspace.rail_is_open(cx)));
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();

    // Files off, or the panel off: the editor.
    for off in [
        (|settings: &mut crate::app_settings::AppSettings| settings.files_view = false)
            as fn(&mut crate::app_settings::AppSettings),
        |settings| settings.show_side_panel = false,
    ] {
        store.update(&mut visual, |store, cx| {
            store.update_settings(
                |settings| {
                    settings.files_view = true;
                    settings.show_side_panel = true;
                    off(settings);
                    true
                },
                false,
                cx,
            )
        });
        click(&mut visual, root.join("lib.rs"));
        assert_eq!(launched(), 1);
    }
}

#[test]
fn remote_mission_refusals_keep_resume_and_restart_distinct_through_update() {
    use crate::surfaces::remote_client::{tests as remote_test, RemoteItem};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for action in ["resume", "restart", "resumeMission"] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
        remote_test::enable_missions(&fake);
        fake.set_features(&[
            "catalog",
            "feed",
            "follow",
            "unfollow",
            "answer",
            "post",
            "resume",
            "restart",
            "resumeMission",
            "runtimeTaskStart",
            "runtimeTask",
        ]);
        fake.set_overview(serde_json::from_value(serde_json::json!({"projects":[],"tabs":[],"missions":[{"id":"remote-mission","project":null,"title":"Host mission","crew":"Duo","state":"running","pinned":false,"members":[{"handle":"coder","session":"member","lead":false}],"asks":[],"shells":[]}],"sessions":{"member":{"kind":"missionSlot","title":"Member","runtime":"codex","status":"stopped","unread":false}}})).unwrap());
        fake.queue_result(action, Err(remote_test::update_gate_error()));
        fake.queue_result("runtimeTaskStart", Ok(remote_test::completed_update()));
        let window = remote_test::add_window(&mut cx, &store, temp.path(), action);
        window
            .update(&mut cx, |root, window, cx| {
                remote_test::choose_host(root, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, _, cx| {
                root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
            })
            .unwrap();
        cx.run_until_parked();
        let workspace = window
            .read_with(&cx, |root, _| {
                root.remote_client
                    .workspace()
                    .unwrap()
                    .mission_workspace
                    .clone()
            })
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
        visual.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                if action == "resumeMission" {
                    workspace.resume_open_mission(window, cx);
                } else {
                    workspace.act_on_slot(
                        "member",
                        if action == "restart" {
                            SessionControlKind::Restart
                        } else {
                            SessionControlKind::Resume
                        },
                        window,
                        cx,
                    );
                }
            })
        });
        visual.run_until_parked();
        window
            .read_with(&visual, |root, cx| {
                let host = root.front_page_client(cx).target;
                let gate = root
                    .agent_tasks
                    .gates
                    .get(&(host, "member".into()))
                    .expect("validated remote refusal reaches recovery");
                assert_eq!(
                    gate.action == crate::surfaces::agent_recovery::RecoveryAction::Restart,
                    action == "restart"
                );
            })
            .unwrap();
        // Select the slot so its recovery card is shown.
        visual.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.select_mission_session("member", window, cx)
            })
        });
        visual.run_until_parked();
        let button = visual
            .debug_bounds("AGENT_RECOVERY_UPDATE:member")
            .expect("mission recovery action");
        visual.simulate_click(button.center(), Default::default());
        visual.run_until_parked();
        assert_eq!(fake.request_count("runtimeTaskStart"), 1);
        if action == "restart" {
            assert_eq!(fake.request_count("restart"), 2);
            assert_eq!(fake.request_count("resume"), 0);
        } else {
            assert_eq!(
                fake.request_count("resume"),
                if action == "resume" { 2 } else { 1 }
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn maximized_mission_rail_hides_center_toggles_on_shortcut_and_restores_when_selecting_session() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, host, _fake) = suspended_mission_test();
    host.update(&mut cx, |root, window, cx| {
        root.open_mission("test-mission".into(), window, cx)
    })
    .unwrap();
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();

    // Compact 300 px Sessions hides Maximize until widened or maximized via ⇧⌘↩.
    assert!(visual.debug_bounds("SIDE_PANEL_MAXIMIZE").is_none());
    host.update(&mut visual, |root, window, cx| {
        let focus = root.mission_workspace.read(cx).rail_focus.clone();
        focus.focus(window, cx);
    })
    .unwrap();
    visual.dispatch_action(crate::ToggleMaximizePane);
    visual.run_until_parked();

    host.read_with(&visual, |root, cx| {
        let ws = root.mission_workspace.read(cx);
        assert!(ws.rail_maximized);
        assert!(ws.shown_terminals("test-mission", cx).is_empty());
    })
    .unwrap();
    assert!(
        visual.debug_bounds("SIDE_PANEL_MAXIMIZE active").is_some(),
        "Restore chat button stays active while rail is maximized"
    );

    // Clicking a session card in maximized Sessions exits rail maximize, reveals center,
    // selects that session tab, and focuses its visible terminal.
    let card = visual
        .debug_bounds("MISSION_CARD mission-agent")
        .expect("session card in maximized Sessions");
    visual.simulate_click(card.center(), Default::default());
    visual.run_until_parked();

    host.update(&mut visual, |root, window, cx| {
        let ws = root.mission_workspace.read(cx);
        assert!(!ws.rail_maximized);
        assert_eq!(ws.active_tab, MissionTab::Session("mission-agent".into()));
        assert_eq!(ws.shown_terminals("test-mission", cx), ["mission-agent"]);
        let terminal_focus = &ws
            .attached
            .get("mission-agent")
            .expect("attached mission-agent")
            .terminal_focus;
        assert!(terminal_focus.is_focused(window));
    })
    .unwrap();

    // Focusing the mission rail and dispatching ToggleMaximizePane (⇧⌘↩) maximizes again,
    // and pressing Enter on a session card also restores the center and focuses its terminal.
    host.update(&mut visual, |root, window, cx| {
        let focus = root.mission_workspace.read(cx).rail_focus.clone();
        focus.focus(window, cx);
    })
    .unwrap();
    visual.dispatch_action(crate::ToggleMaximizePane);
    visual.run_until_parked();
    host.read_with(&visual, |root, cx| {
        assert!(root.mission_workspace.read(cx).rail_maximized);
    })
    .unwrap();

    host.update(&mut visual, |root, window, cx| {
        root.mission_workspace.update(cx, |ws, cx| {
            ws.select_mission_session("mission-shell", window, cx);
        });
        let ws = root.mission_workspace.read(cx);
        assert!(!ws.rail_maximized);
        assert_eq!(ws.active_tab, MissionTab::Session("mission-shell".into()));
        assert!(ws
            .attached
            .get("mission-shell")
            .expect("attached mission-shell")
            .terminal_focus
            .is_focused(window));
    })
    .unwrap();

    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[cfg(unix)]
#[test]
fn mission_rail_progressively_widens_on_file_or_diff_and_shrinks_back_to_300px() {
    use std::process::Command;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (temp, mut cx, host, _fake) = suspended_mission_test();
    let repo = temp.path().join("mission-repo");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "core.autocrlf", "false"][..],
    ] {
        assert!(Command::new("git")
            .current_dir(&repo)
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .unwrap()
            .success());
    }
    std::fs::write(repo.join("lib.rs"), "pub fn mission() {}\n").unwrap();
    std::fs::write(repo.join("main.rs"), "fn main() {}\n").unwrap();
    for args in [
        &["add", "-A"][..],
        &[
            "-c",
            "user.name=Runner",
            "-c",
            "user.email=runner@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "init",
        ][..],
    ] {
        assert!(Command::new("git")
            .current_dir(&repo)
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .unwrap()
            .success());
    }
    // Modify lib.rs so Changes has an uncommitted diff to open.
    std::fs::write(
        repo.join("lib.rs"),
        "pub fn mission() { println!(\"ok\"); }\n",
    )
    .unwrap();
    let canonical = repo.canonicalize().unwrap();
    host.update(&mut cx, |root, window, cx| {
        root.app_store.update(cx, |store, cx| {
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute(
                    "UPDATE missions SET cwd = ?1, files_root = ?1 WHERE id = 'test-mission'",
                    [canonical.to_string_lossy().as_ref()],
                )
                .unwrap();
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        root.open_mission("test-mission".into(), window, cx);
    })
    .unwrap();
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1600.), px(900.)));
    visual.run_until_parked();

    let rail_width = |visual: &mut gpui::VisualTestContext| -> f32 {
        f32::from(visual.debug_bounds("MISSION_RAIL").unwrap().size.width)
    };
    let click = |visual: &mut gpui::VisualTestContext, selector: &'static str| {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        visual.simulate_click(bounds.center(), gpui::Modifiers::default());
        visual.run_until_parked();
    };
    let wait = |visual: &mut gpui::VisualTestContext,
                what: &str,
                ready: &dyn Fn(&mut gpui::VisualTestContext) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready(visual) {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            std::thread::yield_now();
            visual.update(|window, _| window.refresh());
            visual.run_until_parked();
        }
    };
    // The rail's square fills stay inside its 1 px border and clear of its 11 px bottom curves,
    // and a column at the right edge meets the border without a gap (#1440).
    let assert_inside_card =
        |visual: &mut gpui::VisualTestContext, column: &'static str, at_right_edge: bool| {
            let rail = visual.debug_bounds("MISSION_RAIL").unwrap();
            let bounds = visual.debug_bounds(column).unwrap();
            assert!(
                bounds.right() <= rail.right() - px(0.5),
                "{column} {bounds:?} crosses the rail's right border {rail:?}"
            );
            assert!(
                bounds.bottom() <= rail.bottom() - px(8.5),
                "{column} {bounds:?} reaches the rail's bottom corners {rail:?}"
            );
            assert!(
                !at_right_edge || bounds.right() >= rail.right() - px(1.01),
                "{column} {bounds:?} leaves a gap before the rail's right border {rail:?}"
            );
        };

    // Fresh mission rail starts compact at 300 px on Sessions (single-column session cards).
    assert!((rail_width(&mut visual) - 300.).abs() <= 1.);
    // Mission activity bar is 40 px tall, and a fresh mission opened on Sessions probes the git
    // checkout directly so PANEL_CHANGES_DIRTY_DOT appears without ever opening Files/Changes.
    let rail_header = visual.debug_bounds("MISSION_RAIL_HEADER").unwrap();
    assert!(
        (f32::from(rail_header.size.height) - 40.).abs() <= 1.,
        "mission activity bar height must be 40 px: {rail_header:?}"
    );
    wait(&mut visual, "fresh mission dirty dot on Sessions", &|v| {
        v.debug_bounds("PANEL_CHANGES_DIRTY_DOT").is_some()
    });
    let card_agent = visual.debug_bounds("MISSION_CARD mission-agent").unwrap();
    let card_shell = visual.debug_bounds("MISSION_CARD mission-shell").unwrap();
    assert!(card_agent.top() < card_shell.top());

    // Clicking Changes stays compact at 300 px (list-only) until a changed file is clicked,
    // which widens smoothly once to the unified 700 px (460 viewer + 240 right rail).
    click(&mut visual, "PANEL_ACTIVITY Changes");
    wait(&mut visual, "Changes list loaded", &|v| {
        v.debug_bounds("CHANGES_ROW lib.rs").is_some()
    });
    assert!((rail_width(&mut visual) - 300.).abs() <= 1.);
    click(&mut visual, "CHANGES_ROW lib.rs");
    wait(&mut visual, "Changes diff widens rail to 700 px", &|v| {
        v.debug_bounds("FILE_TAB_CLOSE lib.rs").is_some()
            && v.debug_bounds("CHANGES_DIFF_CODE").is_some()
            && (rail_width(v) - 700.).abs() <= 1.
    });
    assert_inside_card(&mut visual, "CHANGES_LIST_COLUMN", true);
    assert_inside_card(&mut visual, "CHANGES_DIFF_CODE", false);
    // Switching right rail to Sessions and Files while the diff is open produces 0 px jump!
    click(&mut visual, "PANEL_ACTIVITY Sessions");
    assert!(
        (rail_width(&mut visual) - 700.).abs() <= 1.,
        "switching Changes -> Sessions with viewer open must keep 700 px width"
    );
    assert!(visual.debug_bounds("CHANGES_DIFF_CODE").is_some());
    // Sessions cards stay cleanly single-column in the 240 px rail beside the viewer.
    let card_agent_beside = visual.debug_bounds("MISSION_CARD mission-agent").unwrap();
    let card_shell_beside = visual.debug_bounds("MISSION_CARD mission-shell").unwrap();
    assert!(card_agent_beside.top() < card_shell_beside.top());

    click(&mut visual, "PANEL_ACTIVITY Files");
    wait(&mut visual, "Files tree loaded beside open diff", &|v| {
        v.debug_bounds("FILES_ROW lib.rs").is_some()
    });
    assert!(
        (rail_width(&mut visual) - 700.).abs() <= 1.,
        "switching Sessions -> Files with viewer open must keep 700 px width"
    );
    click(&mut visual, "FILE_TAB_CLOSE lib.rs");
    wait(
        &mut visual,
        "Closing last tab shrinks back to 300 px",
        &|v| {
            v.debug_bounds("FILE_TAB_CLOSE lib.rs").is_none() && (rail_width(v) - 300.).abs() <= 1.
        },
    );

    // Opening lib.rs from Files widens smoothly to 700 px (460 + 240);
    // hiding rail narrows to 460 px; dragging the rail gutter saves unified file_viewer_width;
    // closing the last file tab shrinks back to 300 px.
    click(&mut visual, "FILES_ROW lib.rs");
    wait(
        &mut visual,
        "lib.rs opens and widens rail to 700 px",
        &|v| v.debug_bounds("FILE_CODE").is_some() && (rail_width(v) - 700.).abs() <= 1.,
    );
    click(&mut visual, "FILE_TREE_TOGGLE");
    wait(&mut visual, "hiding rail narrows rail to 460 px", &|v| {
        (rail_width(v) - 460.).abs() <= 1.
    });
    click(&mut visual, "FILE_TREE_TOGGLE");
    wait(&mut visual, "showing tree widens rail to 700 px", &|v| {
        (rail_width(v) - 700.).abs() <= 0.5
            && host
                .read_with(v, |r, cx| {
                    r.mission_workspace
                        .read(cx)
                        .rail_width_anim
                        .started_at
                        .is_none()
                })
                .unwrap()
    });
    for scale in [2., 1.5] {
        visual.simulate_scale_factor_change(scale);
        visual.update(|window, _| window.refresh());
        visual.run_until_parked();
        assert_inside_card(&mut visual, "FILES_TREE_COLUMN", true);
        assert_inside_card(&mut visual, "FILE_CODE", false);
    }
    visual.simulate_scale_factor_change(2.);
    visual.run_until_parked();

    let start = visual.debug_bounds("MISSION_RAIL_GUTTER").unwrap().center();
    visual.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
    for _ in 0..3 {
        visual.simulate_mouse_move(
            start + gpui::point(px(-40.), px(0.)),
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        visual.run_until_parked();
    }
    visual.simulate_mouse_up(
        start + gpui::point(px(-40.), px(0.)),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    assert!((rail_width(&mut visual) - 740.).abs() <= 1.);

    // Narrowing the window preserves MAIN_MIN_WIDTH = 420 px for the mission column.
    visual.simulate_resize(gpui::size(px(1260.), px(900.)));
    wait(
        &mut visual,
        "mission column preserves MAIN_MIN_WIDTH",
        &|v| {
            v.debug_bounds("MISSION_COLUMN")
                .is_some_and(|c| f32::from(c.size.width) >= 419.5)
        },
    );
    assert!(rail_width(&mut visual) > 528.);

    // Closing the last file tab shrinks smoothly back to 300 px.
    click(&mut visual, "FILE_TAB_CLOSE lib.rs");
    wait(&mut visual, "closing file shrinks rail to 300 px", &|v| {
        (rail_width(v) - 300.).abs() <= 1.
    });

    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn mission_unified_viewer_keeps_aligned_header_promotes_preview_from_files_and_maintains_single_watch(
) {
    use std::process::Command;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (temp, mut cx, host, _fake) = suspended_mission_test();
    let repo = temp.path().join("mission-unified-1421");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "core.autocrlf", "false"][..],
    ] {
        assert!(Command::new("git")
            .current_dir(&repo)
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .unwrap()
            .success());
    }
    std::fs::write(repo.join("lib.rs"), "pub fn mission() {}\n").unwrap();
    std::fs::write(repo.join("main.rs"), "fn main() {}\n").unwrap();
    for args in [
        &["add", "-A"][..],
        &[
            "-c",
            "user.name=Runner",
            "-c",
            "user.email=runner@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "init",
        ][..],
    ] {
        assert!(Command::new("git")
            .current_dir(&repo)
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .unwrap()
            .success());
    }
    std::fs::write(
        repo.join("lib.rs"),
        "pub fn mission() { println!(\"changed\"); }\n",
    )
    .unwrap();
    let canonical = if cfg!(windows) {
        repo.clone()
    } else {
        repo.canonicalize().unwrap()
    };
    let store = host
        .read_with(&cx, |root, _| root.app_store.clone())
        .unwrap();
    host.update(&mut cx, |root, window, cx| {
        root.app_store.update(cx, |store, cx| {
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute(
                    "UPDATE missions SET cwd = ?1, files_root = ?1 WHERE id = 'test-mission'",
                    [canonical.to_string_lossy().as_ref()],
                )
                .unwrap();
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        root.open_mission("test-mission".into(), window, cx);
    })
    .unwrap();
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1600.), px(900.)));
    visual.run_until_parked();

    let rail_width = |visual: &mut gpui::VisualTestContext| -> f32 {
        f32::from(visual.debug_bounds("MISSION_RAIL").unwrap().size.width)
    };
    let click = |visual: &mut gpui::VisualTestContext, selector: &'static str| {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        visual.simulate_click(bounds.center(), gpui::Modifiers::default());
        visual.run_until_parked();
    };
    let wait = |visual: &mut gpui::VisualTestContext,
                what: &str,
                ready: &dyn Fn(&mut gpui::VisualTestContext) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready(visual) {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            std::thread::yield_now();
            visual.update(|window, _| window.refresh());
            visual.run_until_parked();
        }
    };
    let watch_count = |visual: &mut gpui::VisualTestContext| -> usize {
        store.read_with(visual, |s, _| s.test_core.files.watched_roots().len())
    };
    let subject = crate::surfaces::side_panel::PanelSubject::Mission("test-mission".into());
    let files_watching = |visual: &mut gpui::VisualTestContext| -> bool {
        host.read_with(visual, |r, cx| {
            r.mission_workspace
                .read(cx)
                .rail_files
                .view(&subject)
                .is_some_and(|v| v.read(cx).watched().is_some())
        })
        .unwrap()
    };
    let changes_watching = |visual: &mut gpui::VisualTestContext| -> bool {
        host.read_with(visual, |r, cx| {
            r.mission_workspace
                .read(cx)
                .rail_files
                .changes_view(&subject)
                .is_some_and(|v| v.read(cx).watched().is_some())
        })
        .unwrap()
    };
    let viewer_watching = |visual: &mut gpui::VisualTestContext| -> bool {
        host.read_with(visual, |r, cx| {
            r.mission_workspace
                .read(cx)
                .rail_files
                .viewer(&subject)
                .is_some_and(|v| v.read(cx).watched().is_some())
        })
        .unwrap()
    };
    let watcher_owners = |visual: &mut gpui::VisualTestContext| -> (bool, bool, bool) {
        (
            files_watching(visual),
            changes_watching(visual),
            viewer_watching(visual),
        )
    };
    #[cfg(unix)]
    let dirty_active = |visual: &mut gpui::VisualTestContext| -> bool {
        host.read_with(visual, |r, cx| {
            r.mission_workspace
                .read(cx)
                .rail_files
                .dirty_tracker_active(cx)
        })
        .unwrap()
    };
    let rail_focused = |visual: &mut gpui::VisualTestContext| -> bool {
        host.update(visual, |r, window, cx| {
            r.mission_workspace.read(cx).rail_focus.is_focused(window)
        })
        .unwrap()
    };

    // 1. Sessions rail starts at 300 px with DirtyTracker exclusively owning the single watch.
    wait(
        &mut visual,
        "initial dirty dot and single watch on Sessions",
        &|v| {
            v.debug_bounds("PANEL_CHANGES_DIRTY_DOT").is_some()
                && watch_count(v) == 1
                && watcher_owners(v) == (false, false, false)
        },
    );
    #[cfg(unix)]
    assert!(dirty_active(&mut visual));

    // 2. Opening a diff from Changes widens to 700 px with aligned 40 px viewer + rail headers
    // and ChangesView exclusively owning the single watch.
    click(&mut visual, "PANEL_ACTIVITY Changes");
    wait(&mut visual, "Changes list loaded with single watch", &|v| {
        v.debug_bounds("CHANGES_ROW lib.rs").is_some()
            && watch_count(v) == 1
            && watcher_owners(v) == (false, true, false)
    });
    #[cfg(unix)]
    assert!(!dirty_active(&mut visual));
    click(&mut visual, "CHANGES_ROW lib.rs");
    wait(&mut visual, "lib.rs preview diff opens at 700 px", &|v| {
        v.debug_bounds("CHANGES_DIFF_CODE").is_some()
            && v.debug_bounds("FILE_TAB_PREVIEW lib.rs").is_some()
            && (rail_width(v) - 700.).abs() <= 1.
            && watch_count(v) == 1
            && watcher_owners(v) == (false, true, false)
    });
    let viewer_hdr = visual.debug_bounds("FILE_TAB_STRIP").unwrap();
    let rail_hdr = visual.debug_bounds("MISSION_RAIL_HEADER").unwrap();
    assert!(
        (f32::from(viewer_hdr.size.height) - 40.).abs() <= 1.
            && (f32::from(rail_hdr.size.height) - 40.).abs() <= 1.
            && (f32::from(viewer_hdr.origin.y) - f32::from(rail_hdr.origin.y)).abs() <= 0.5,
        "viewer header and rail activity bar must form one aligned 40 px row: {viewer_hdr:?} vs {rail_hdr:?}"
    );

    // 3. Switching to Sessions keeps 700 px (0 px jump), hands the single watch exclusively to
    // UnifiedViewer, keeps DirtyTracker inactive, and routes Escape from the focused viewer back
    // to the Sessions rail focus handle.
    click(&mut visual, "PANEL_ACTIVITY Sessions");
    wait(
        &mut visual,
        "UnifiedViewer owns single watch on Sessions",
        &|v| watcher_owners(v) == (false, false, true) && watch_count(v) == 1,
    );
    assert!((rail_width(&mut visual) - 700.).abs() <= 1.);
    assert!(visual.debug_bounds("CHANGES_DIFF_CODE").is_some());
    #[cfg(unix)]
    assert!(
        !dirty_active(&mut visual),
        "DirtyTracker must stay inactive while UnifiedViewer owns the watch"
    );
    host.update(&mut visual, |r, window, cx| {
        let viewer = r
            .mission_workspace
            .read(cx)
            .rail_files
            .viewer(&subject)
            .unwrap();
        viewer.update(cx, |v, cx| v.focus_active(window, cx));
    })
    .unwrap();
    visual.run_until_parked();
    assert!(
        !rail_focused(&mut visual),
        "viewer must hold focus before Escape"
    );
    visual.simulate_keystrokes("escape");
    wait(
        &mut visual,
        "Escape from viewer focuses Sessions rail",
        &|v| rail_focused(v),
    );

    // 4. Switching to Files keeps 700 px (0 px jump), hands watch exclusively to FilesView, and
    // opening the same file from Files promotes the Changes italic preview tab to pinned Code mode.
    click(&mut visual, "PANEL_ACTIVITY Files");
    wait(
        &mut visual,
        "Files rail owns single watch beside open diff",
        &|v| {
            watcher_owners(v) == (true, false, false)
                && watch_count(v) == 1
                && v.debug_bounds("FILES_ROW lib.rs").is_some()
        },
    );
    #[cfg(unix)]
    assert!(!dirty_active(&mut visual));
    assert!((rail_width(&mut visual) - 700.).abs() <= 1.);
    click(&mut visual, "FILES_ROW lib.rs");
    wait(
        &mut visual,
        "Files open promotes preview tab to pinned Code mode",
        &|v| {
            v.debug_bounds("FILE_CODE").is_some()
                && v.debug_bounds("FILE_TAB_PINNED lib.rs").is_some()
                && v.debug_bounds("FILE_TAB_PREVIEW lib.rs").is_none()
        },
    );

    // 5. Hiding rail narrows to 460 px, keeps PANEL_COLLAPSE reachable, and UnifiedViewer
    // exclusively owns the single watch without duplicates.
    click(&mut visual, "FILE_TREE_TOGGLE");
    wait(
        &mut visual,
        "hidden rail transfers single watch to viewer",
        &|v| {
            (rail_width(v) - 460.).abs() <= 1.
                && v.debug_bounds("PANEL_COLLAPSE").is_some()
                && watcher_owners(v) == (false, false, true)
                && watch_count(v) == 1
        },
    );
    #[cfg(unix)]
    assert!(!dirty_active(&mut visual));

    // 6. Restoring rail, switching to Sessions, closing last tab shrinks to 300 px and
    // hands the single watch back to DirtyTracker; collapsing releases all watches (0).
    click(&mut visual, "FILE_TREE_TOGGLE");
    wait(&mut visual, "rail restored at 700 px", &|v| {
        (rail_width(v) - 700.).abs() <= 1.
            && v.debug_bounds("PANEL_ACTIVITY Sessions").is_some()
            && watcher_owners(v) == (true, false, false)
            && watch_count(v) == 1
    });
    click(&mut visual, "PANEL_ACTIVITY Sessions");
    click(&mut visual, "FILE_TAB_CLOSE lib.rs");
    wait(
        &mut visual,
        "closing last tab shrinks to 300 px and keeps dirty dot",
        &|v| {
            (rail_width(v) - 300.).abs() <= 1.
                && v.debug_bounds("PANEL_CHANGES_DIRTY_DOT").is_some()
                && watcher_owners(v) == (false, false, false)
                && watch_count(v) == 1
        },
    );
    #[cfg(unix)]
    assert!(dirty_active(&mut visual));
    click(&mut visual, "PANEL_COLLAPSE");
    wait(
        &mut visual,
        "collapsing rail releases every daemon watch",
        &|v| watch_count(v) == 0 && watcher_owners(v) == (false, false, false),
    );
    #[cfg(unix)]
    assert!(!dirty_active(&mut visual));

    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn mission_sessions_rail_refreshes_pinned_branch_diff_when_comparison_ref_moves() {
    use runner_core::protocol::files::{
        FilesChanged, FilesStatusChanged, FILES_CHANGED_EVENT, FILES_STATUS_EVENT,
    };
    use std::process::Command;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (temp, mut cx, host, _fake) = suspended_mission_test();
    let repo = temp.path().join("mission-branch-ref-move");
    std::fs::create_dir_all(&repo).unwrap();
    let run_git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(&repo)
            .args([
                "-c",
                "user.name=Runner",
                "-c",
                "user.email=runner@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run_git(&["init", "-q", "-b", "main"]);
    run_git(&["config", "core.autocrlf", "false"]);
    std::fs::write(repo.join("lib.rs"), "pub fn base() {}\n").unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-q", "-m", "init"]);
    // Local comparison branch `cmp-base` starts at `main`, while feature branch `feat/live`
    // has 2 commits ahead modifying `lib.rs` and adding `extra.rs`.
    run_git(&["branch", "cmp-base", "main"]);
    run_git(&["checkout", "-q", "-b", "feat/live"]);
    std::fs::write(repo.join("lib.rs"), "pub fn updated() -> i32 { 42 }\n").unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-q", "-m", "feat: update lib"]);
    std::fs::write(repo.join("extra.rs"), "pub fn extra() {}\n").unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-q", "-m", "feat: add extra"]);

    let canonical = if cfg!(windows) {
        repo.clone()
    } else {
        repo.canonicalize().unwrap()
    };
    let store = host
        .read_with(&cx, |root, _| root.app_store.clone())
        .unwrap();
    let events = store.read_with(&cx, |s, _| s.test_core.events.clone());
    let mut sent = events.subscribe();

    host.update(&mut cx, |root, window, cx| {
        root.app_store.update(cx, |store, cx| {
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute(
                    "UPDATE missions SET cwd = ?1, files_root = ?1 WHERE id = 'test-mission'",
                    [canonical.to_string_lossy().as_ref()],
                )
                .unwrap();
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        root.open_mission("test-mission".into(), window, cx);
    })
    .unwrap();
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1600.), px(900.)));
    visual.run_until_parked();

    let click = |visual: &mut gpui::VisualTestContext, selector: &'static str| {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        visual.simulate_click(bounds.center(), gpui::Modifiers::default());
        visual.run_until_parked();
    };
    let wait = |visual: &mut gpui::VisualTestContext,
                what: &str,
                ready: &dyn Fn(&mut gpui::VisualTestContext) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready(visual) {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            std::thread::yield_now();
            visual.update(|window, _| window.refresh());
            visual.run_until_parked();
        }
    };
    let subject = crate::surfaces::side_panel::PanelSubject::Mission("test-mission".into());
    let viewer_watching = |visual: &mut gpui::VisualTestContext| -> bool {
        host.read_with(visual, |r, cx| {
            r.mission_workspace
                .read(cx)
                .rail_files
                .viewer(&subject)
                .is_some_and(|v| v.read(cx).watched().is_some())
        })
        .unwrap()
    };

    // 1. Open Changes, choose `cmp-base` as Base, open & pin `lib.rs` on All changes.
    click(&mut visual, "PANEL_ACTIVITY Changes");
    wait(&mut visual, "Changes ready with main base", &|v| {
        v.debug_bounds("CHANGES_BASE_CHIP Auto · main").is_some()
            && v.debug_bounds("CHANGES_ROW lib.rs").is_some()
    });
    click(&mut visual, "CHANGES_BASE_CHIP Auto · main");
    wait(&mut visual, "base picker lists cmp-base", &|v| {
        v.debug_bounds("CHANGES_BASE_PICKER_ITEM 1 cmp-base")
            .is_some()
    });
    click(&mut visual, "CHANGES_BASE_PICKER_ITEM 1 cmp-base");
    wait(&mut visual, "cmp-base active with lib.rs listed", &|v| {
        v.debug_bounds("CHANGES_BASE_CHIP cmp-base").is_some()
            && v.debug_bounds("CHANGES_ROW lib.rs").is_some()
    });
    click(&mut visual, "CHANGES_ROW lib.rs");
    wait(&mut visual, "lib.rs diff opened", &|v| {
        v.debug_bounds("CHANGES_DIFF_CODE").is_some() && v.debug_bounds("VIEWER_PIN_TAB").is_some()
    });
    click(&mut visual, "VIEWER_PIN_TAB");
    wait(&mut visual, "lib.rs pinned", &|v| {
        v.debug_bounds("FILE_TAB_PINNED lib.rs").is_some()
    });

    // 2. Switch to Sessions so Changes is hidden and UnifiedViewer owns the watch, then prove
    // real OS watcher delivery via an independent sentinel before moving the Git ref.
    click(&mut visual, "PANEL_ACTIVITY Sessions");
    wait(&mut visual, "UnifiedViewer owns watch on Sessions", &|v| {
        viewer_watching(v) && v.debug_bounds("CHANGES_DIFF_CODE").is_some()
    });
    while matches!(
        sent.try_recv(),
        Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
    ) {}
    let sentinel = canonical.join(".watch-ready");
    let mut probe = 0u32;
    let mut last_probe = std::time::Instant::now() - std::time::Duration::from_secs(2);
    let mut saw_changed = false;
    let mut saw_status = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline && !(saw_changed && saw_status) {
        if last_probe.elapsed() >= std::time::Duration::from_secs(1) {
            probe += 1;
            std::fs::write(&sentinel, format!("ready-{probe}\n")).unwrap();
            last_probe = std::time::Instant::now();
        }
        visual.update(|window, _| window.refresh());
        visual.run_until_parked();
        loop {
            match sent.try_recv() {
                Ok(event) if event.name == FILES_CHANGED_EVENT => {
                    if serde_json::from_value::<FilesChanged>(event.payload).is_ok_and(|c| {
                        runner_core::app_paths::same_checkout_path(&c.root, &canonical)
                            && (c.paths.iter().any(|p| p == ".watch-ready")
                                || (c.paths.is_empty() && c.folders == [""]))
                    }) {
                        saw_changed = true;
                    }
                }
                Ok(event) if event.name == FILES_STATUS_EVENT => {
                    if serde_json::from_value::<FilesStatusChanged>(event.payload).is_ok_and(|u| {
                        runner_core::app_paths::same_checkout_path(&u.root, &canonical)
                            && u.status.entries.iter().any(|e| e.path == ".watch-ready")
                    }) {
                        saw_status = true;
                    }
                }
                Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
        if !(saw_changed && saw_status) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    assert!(
        saw_changed && saw_status,
        "timed out waiting for viewer watch readiness on Sessions"
    );
    std::fs::remove_file(&sentinel).unwrap();
    wait(&mut visual, "sentinel cleaned and dirty dot clear", &|v| {
        v.debug_bounds("PANEL_CHANGES_DIRTY_DOT").is_none()
            && v.debug_bounds("CHANGES_DIFF_CODE").is_some()
    });

    // 3. Move ONLY `cmp-base` to HEAD while staying on Sessions: pinned All changes diff must
    // settle to `FILE_DIFF_NO_CHANGES` without switching rails.
    run_git(&["branch", "-f", "cmp-base", "HEAD"]);
    wait(
        &mut visual,
        "moving cmp-base to HEAD settles pinned Branch diff to NoChanges on Sessions",
        &|v| {
            v.debug_bounds("FILE_DIFF_NO_CHANGES").is_some()
                && v.debug_bounds("CHANGES_DIFF_CODE").is_none()
                && v.debug_bounds("PANEL_ACTIVITY Sessions active").is_some()
        },
    );

    // 4. Move `cmp-base` back to `main` while STILL on Sessions: settled NoChanges tab must
    // reload its diff automatically without switching rails.
    run_git(&["branch", "-f", "cmp-base", "main"]);
    wait(
        &mut visual,
        "moving cmp-base back to main reloads settled NoChanges Branch diff on Sessions",
        &|v| {
            v.debug_bounds("CHANGES_DIFF_CODE").is_some()
                && v.debug_bounds("FILE_DIFF_NO_CHANGES").is_none()
                && v.debug_bounds("PANEL_ACTIVITY Sessions active").is_some()
        },
    );

    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn mission_sessions_rail_commit_viewed_does_not_amber_uncommitted_and_clear_clears_amber() {
    use runner_core::protocol::changes::ChangesTarget;
    use runner_core::protocol::files::{
        FilesChanged, FilesStatusChanged, FILES_CHANGED_EVENT, FILES_STATUS_EVENT,
    };
    use std::process::Command;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (temp, mut cx, host, _fake) = suspended_mission_test();
    let repo = temp.path().join("mission-commit-vs-wt-viewed");
    std::fs::create_dir_all(&repo).unwrap();
    let run_git = |args: &[&str]| -> String {
        let out = Command::new("git")
            .current_dir(&repo)
            .args([
                "-c",
                "user.name=Runner",
                "-c",
                "user.email=runner@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };
    run_git(&["init", "-q", "-b", "main"]);
    run_git(&["config", "core.autocrlf", "false"]);
    std::fs::write(repo.join("lib.rs"), "pub fn base() {}\n").unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-q", "-m", "init"]);
    run_git(&["checkout", "-q", "-b", "feat/viewed"]);
    std::fs::write(repo.join("lib.rs"), "pub fn commit_v() {}\n").unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-q", "-m", "feat: commit edit"]);
    let commit_short = run_git(&["rev-parse", "--short=7", "HEAD"]);
    // Also leave an uncommitted working-tree edit on the same file.
    std::fs::write(repo.join("lib.rs"), "pub fn wt_v1() {}\n").unwrap();

    let canonical = if cfg!(windows) {
        repo.clone()
    } else {
        repo.canonicalize().unwrap()
    };
    let store = host
        .read_with(&cx, |root, _| root.app_store.clone())
        .unwrap();
    let events = store.read_with(&cx, |s, _| s.test_core.events.clone());
    let mut sent = events.subscribe();

    host.update(&mut cx, |root, window, cx| {
        root.app_store.update(cx, |store, cx| {
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute(
                    "UPDATE missions SET cwd = ?1, files_root = ?1 WHERE id = 'test-mission'",
                    [canonical.to_string_lossy().as_ref()],
                )
                .unwrap();
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        root.open_mission("test-mission".into(), window, cx);
    })
    .unwrap();
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1600.), px(900.)));
    visual.run_until_parked();

    let click = |visual: &mut gpui::VisualTestContext, selector: &str| {
        let sel: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        let bounds = visual
            .debug_bounds(sel)
            .unwrap_or_else(|| panic!("missing {selector}"));
        visual.simulate_click(bounds.center(), gpui::Modifiers::default());
        visual.run_until_parked();
    };
    let wait = |visual: &mut gpui::VisualTestContext,
                what: &str,
                ready: &dyn Fn(&mut gpui::VisualTestContext) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready(visual) {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            std::thread::yield_now();
            visual.update(|window, _| window.refresh());
            visual.run_until_parked();
        }
    };
    let subject = crate::surfaces::side_panel::PanelSubject::Mission("test-mission".into());
    let viewer = |visual: &mut gpui::VisualTestContext| {
        host.read_with(visual, |r, cx| {
            r.mission_workspace.read(cx).rail_files.viewer(&subject)
        })
        .unwrap()
    };

    // 1. Open and pin BOTH Uncommitted `lib.rs` (tab 0) and historical Commit `lib.rs` (tab 1).
    click(&mut visual, "PANEL_ACTIVITY Changes");
    wait(&mut visual, "Uncommitted lib.rs listed", &|v| {
        v.debug_bounds("CHANGES_ROW lib.rs").is_some()
    });
    click(&mut visual, "CHANGES_ROW lib.rs");
    wait(&mut visual, "Uncommitted lib.rs diff loaded", &|v| {
        v.debug_bounds("CHANGES_DIFF_CODE").is_some() && v.debug_bounds("VIEWER_PIN_TAB").is_some()
    });
    click(&mut visual, "VIEWER_PIN_TAB");
    let v1_blobs = viewer(&mut visual)
        .unwrap()
        .read_with(&visual, |v, _| {
            v.loaded_diff_meta()
                .map(|(_, _, _, o, n)| (o.map(str::to_owned), n.map(str::to_owned)))
        })
        .unwrap();

    let commit_sel: &'static str =
        Box::leak(format!("CHANGES_COMMIT {commit_short}").into_boxed_str());
    wait(&mut visual, "commit row listed", &|v| {
        v.debug_bounds(commit_sel).is_some()
    });
    click(&mut visual, commit_sel);
    wait(&mut visual, "commit lib.rs listed", &|v| {
        v.debug_bounds("CHANGES_ROW lib.rs").is_some()
    });
    click(&mut visual, "CHANGES_ROW lib.rs");
    wait(&mut visual, "commit lib.rs preview diff opened", &|v| {
        v.debug_bounds("CHANGES_DIFF_CODE").is_some() && v.debug_bounds("VIEWER_PIN_TAB").is_some()
    });
    click(&mut visual, "VIEWER_PIN_TAB");
    wait(&mut visual, "both tabs pinned", &|v| {
        viewer(v)
            .unwrap()
            .read_with(v, |viewer, _| viewer.open_tabs().len() == 2)
    });

    // 2. Switch to Sessions (Changes hidden) and mark the historical commit tab viewed.
    click(&mut visual, "PANEL_ACTIVITY Sessions");
    wait(&mut visual, "Sessions active with viewer watch", &|v| {
        viewer(v)
            .unwrap()
            .read_with(v, |viewer, _| viewer.watched().is_some())
            && v.debug_bounds("CHANGES_HEADER_VIEWED").is_some()
    });
    click(&mut visual, "CHANGES_HEADER_VIEWED");
    wait(&mut visual, "commit tab marked viewed", &|v| {
        viewer(v).unwrap().read_with(v, |viewer, _| {
            viewer.active_diff_viewed() == Some((true, false))
        })
    });

    // Switch to tab 0 (Uncommitted `lib.rs`): historical commit mark must NEVER turn it amber!
    let v_ent = viewer(&mut visual).unwrap();
    v_ent.update(&mut visual, |v, cx| v.activate_tab(0, cx));
    visual.run_until_parked();
    wait(
        &mut visual,
        "Uncommitted diff stays unviewed and NOT amber after commit-only viewed mark",
        &|v| {
            v.debug_bounds("CHANGES_DIFF_CODE").is_some()
                && v_ent.read_with(v, |viewer, _| {
                    viewer.loaded_diff_meta().is_some_and(|(t, ..)| {
                        *t == crate::surfaces::changes::model::SelectedTarget::Uncommitted
                    }) && viewer.active_diff_viewed() == Some((false, false))
                })
        },
    );

    // 3. Prove watcher readiness on Sessions, mark Uncommitted v1 viewed, edit to v2 (becomes
    // amber `(false, true)`), then clear v1's tracked mark in daemon: amber must clear to
    // `(false, false)` while staying on Sessions.
    while matches!(
        sent.try_recv(),
        Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
    ) {}
    let sentinel = canonical.join(".watch-ready");
    let mut probe = 0u32;
    let mut last_probe = std::time::Instant::now() - std::time::Duration::from_secs(2);
    let mut saw_changed = false;
    let mut saw_status = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline && !(saw_changed && saw_status) {
        if last_probe.elapsed() >= std::time::Duration::from_secs(1) {
            probe += 1;
            std::fs::write(&sentinel, format!("ready-{probe}\n")).unwrap();
            last_probe = std::time::Instant::now();
        }
        visual.update(|window, _| window.refresh());
        visual.run_until_parked();
        loop {
            match sent.try_recv() {
                Ok(event) if event.name == FILES_CHANGED_EVENT => {
                    if serde_json::from_value::<FilesChanged>(event.payload).is_ok_and(|c| {
                        runner_core::app_paths::same_checkout_path(&c.root, &canonical)
                            && (c.paths.iter().any(|p| p == ".watch-ready")
                                || (c.paths.is_empty() && c.folders == [""]))
                    }) {
                        saw_changed = true;
                    }
                }
                Ok(event) if event.name == FILES_STATUS_EVENT => {
                    if serde_json::from_value::<FilesStatusChanged>(event.payload).is_ok_and(|u| {
                        runner_core::app_paths::same_checkout_path(&u.root, &canonical)
                            && u.status.entries.iter().any(|e| e.path == ".watch-ready")
                    }) {
                        saw_status = true;
                    }
                }
                Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
        if !(saw_changed && saw_status) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    assert!(saw_changed && saw_status, "sentinel watch readiness");
    std::fs::remove_file(&sentinel).unwrap();

    // Mark v1 viewed, wait for daemon write, then edit to v2.
    while matches!(
        sent.try_recv(),
        Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
    ) {}
    click(&mut visual, "CHANGES_HEADER_VIEWED");
    let mut saw_viewed = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !(saw_viewed
        && v_ent.read_with(&visual, |viewer, _| {
            viewer.active_diff_viewed() == Some((true, false))
        }))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for v1 viewed persisted"
        );
        visual.update(|window, _| window.refresh());
        visual.run_until_parked();
        while let Ok(ev) = sent.try_recv() {
            if ev.name == runner_core::protocol::changes::CHANGES_VIEWED_EVENT {
                saw_viewed = true;
            }
        }
        std::thread::yield_now();
    }

    std::fs::write(repo.join("lib.rs"), "pub fn wt_v2() {}\n").unwrap();
    wait(
        &mut visual,
        "v2 diff loads with amber changed_since_viewed",
        &|v| {
            v_ent.read_with(v, |viewer, _| {
                viewer
                    .loaded_diff_meta()
                    .and_then(|(_, _, _, _, new_blob)| new_blob)
                    .is_some_and(|blob| Some(blob) != v1_blobs.1.as_deref())
                    && viewer.active_diff_viewed() == Some((false, true))
            })
        },
    );

    // Mark v2 viewed (`(true, false)`), then uncheck v2 while staying on Sessions: daemon still
    // holds v1's tracked mark, so unchecked v2 must return to amber `(false, true)`.
    click(&mut visual, "CHANGES_HEADER_VIEWED");
    wait(&mut visual, "v2 marked viewed", &|v| {
        v.debug_bounds("CHANGES_HEADER_VIEWED checked").is_some()
            && v_ent.read_with(v, |viewer, _| {
                viewer.active_diff_viewed() == Some((true, false))
            })
    });
    click(&mut visual, "CHANGES_HEADER_VIEWED checked");
    wait(
        &mut visual,
        "unchecking v2 restores amber (false, true) because v1 is still tracked",
        &|v| {
            v_ent.read_with(v, |viewer, _| {
                viewer.active_diff_viewed() == Some((false, true))
            })
        },
    );

    // Clear the tracked v1 mark on daemon while staying on Sessions: amber on v2 must clear!
    store.read_with(&visual, |s, _| {
        s.client
            .changes_set_viewed(
                &canonical,
                &ChangesTarget::Uncommitted,
                "lib.rs",
                v1_blobs.0.as_deref(),
                v1_blobs.1.as_deref(),
                false,
            )
            .unwrap();
    });
    wait(
        &mut visual,
        "clearing tracked v1 mark clears amber on v2 while staying on Sessions",
        &|v| {
            v_ent.read_with(v, |viewer, _| {
                viewer.active_diff_viewed() == Some((false, false))
            })
        },
    );

    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn mission_sessions_rail_updates_pinned_branch_scope_and_base_on_branch_checkout() {
    use runner_core::protocol::files::{
        FilesChanged, FilesStatusChanged, FILES_CHANGED_EVENT, FILES_STATUS_EVENT,
    };
    use std::process::Command;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (temp, mut cx, host, _fake) = suspended_mission_test();
    let repo = temp.path().join("mission-branch-checkout-scope");
    std::fs::create_dir_all(&repo).unwrap();
    let run_git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(&repo)
            .args([
                "-c",
                "user.name=Runner",
                "-c",
                "user.email=runner@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run_git(&["init", "-q", "-b", "main"]);
    run_git(&["config", "core.autocrlf", "false"]);
    std::fs::write(repo.join("lib.rs"), "pub fn main_base() {}\n").unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-q", "-m", "init"]);

    // `branch-a` has 3 commits ahead of `main`; `base-b` sits at commit 1 (modifying `lib.rs` to
    // v1), and `branch-b` points at the EXACT SAME HEAD as `branch-a` (commit 3, `lib.rs` at v2).
    run_git(&["checkout", "-q", "-b", "branch-a"]);
    std::fs::write(repo.join("lib.rs"), "pub fn base_b_v1() {}\n").unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-q", "-m", "feat: v1"]);
    run_git(&["branch", "base-b", "HEAD"]);
    std::fs::write(repo.join("lib.rs"), "pub fn tip_v2() {}\n").unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-q", "-m", "feat: v2"]);
    std::fs::write(repo.join("extra.rs"), "pub fn extra() {}\n").unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-q", "-m", "feat: extra"]);
    run_git(&["branch", "branch-b", "HEAD"]);

    let canonical = if cfg!(windows) {
        repo.clone()
    } else {
        repo.canonicalize().unwrap()
    };
    let store = host
        .read_with(&cx, |root, _| root.app_store.clone())
        .unwrap();
    let events = store.read_with(&cx, |s, _| s.test_core.events.clone());
    let mut sent = events.subscribe();

    host.update(&mut cx, |root, window, cx| {
        root.app_store.update(cx, |store, cx| {
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute(
                    "UPDATE missions SET cwd = ?1, files_root = ?1 WHERE id = 'test-mission'",
                    [canonical.to_string_lossy().as_ref()],
                )
                .unwrap();
            // Save a distinct comparison base (`refs/heads/base-b`) for `branch-b`.
            store.update_settings(
                |settings| {
                    settings.set_changes_base(
                        &canonical,
                        Some("branch-b"),
                        Some("refs/heads/base-b"),
                    )
                },
                false,
                cx,
            );
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        root.open_mission("test-mission".into(), window, cx);
    })
    .unwrap();
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1600.), px(900.)));
    visual.run_until_parked();

    let click = |visual: &mut gpui::VisualTestContext, selector: &'static str| {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        visual.simulate_click(bounds.center(), gpui::Modifiers::default());
        visual.run_until_parked();
    };
    let wait = |visual: &mut gpui::VisualTestContext,
                what: &str,
                ready: &dyn Fn(&mut gpui::VisualTestContext) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready(visual) {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            std::thread::yield_now();
            visual.update(|window, _| window.refresh());
            visual.run_until_parked();
        }
    };
    let subject = crate::surfaces::side_panel::PanelSubject::Mission("test-mission".into());
    let viewer = |visual: &mut gpui::VisualTestContext| {
        host.read_with(visual, |r, cx| {
            r.mission_workspace.read(cx).rail_files.viewer(&subject)
        })
        .unwrap()
    };

    // 1. Open and pin `lib.rs` on `All changes` while on `branch-a` (Auto -> `main`).
    click(&mut visual, "PANEL_ACTIVITY Changes");
    wait(&mut visual, "Changes ready on branch-a", &|v| {
        v.debug_bounds("CHANGES_BASE_CHIP Auto · main").is_some()
            && v.debug_bounds("CHANGES_ROW lib.rs").is_some()
    });
    click(&mut visual, "CHANGES_ROW lib.rs");
    wait(&mut visual, "lib.rs diff opened on branch-a", &|v| {
        v.debug_bounds("CHANGES_DIFF_CODE").is_some() && v.debug_bounds("VIEWER_PIN_TAB").is_some()
    });
    click(&mut visual, "VIEWER_PIN_TAB");
    let v_ent = viewer(&mut visual).unwrap();
    let main_old_blob = v_ent
        .read_with(&visual, |v, _| {
            v.loaded_diff_meta()
                .and_then(|(_, _, _, old_blob, _)| old_blob.map(str::to_owned))
        })
        .expect("main old blob");
    let scope_a = v_ent
        .read_with(&visual, |v, _| v.active_diff_scope_label())
        .unwrap();
    assert!(
        scope_a.contains("branch-a") && scope_a.contains("main"),
        "expected branch-a vs main scope pill, got {scope_a:?}"
    );

    // 2. Switch to Sessions (Changes hidden) and prove real watcher readiness via sentinel.
    click(&mut visual, "PANEL_ACTIVITY Sessions");
    wait(&mut visual, "UnifiedViewer owns watch on Sessions", &|v| {
        v_ent.read_with(v, |viewer, _| viewer.watched().is_some())
            && v.debug_bounds("CHANGES_DIFF_CODE").is_some()
    });
    while matches!(
        sent.try_recv(),
        Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
    ) {}
    let sentinel = canonical.join(".watch-ready");
    let mut probe = 0u32;
    let mut last_probe = std::time::Instant::now() - std::time::Duration::from_secs(2);
    let mut saw_changed = false;
    let mut saw_status = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline && !(saw_changed && saw_status) {
        if last_probe.elapsed() >= std::time::Duration::from_secs(1) {
            probe += 1;
            std::fs::write(&sentinel, format!("ready-{probe}\n")).unwrap();
            last_probe = std::time::Instant::now();
        }
        visual.update(|window, _| window.refresh());
        visual.run_until_parked();
        loop {
            match sent.try_recv() {
                Ok(event) if event.name == FILES_CHANGED_EVENT => {
                    if serde_json::from_value::<FilesChanged>(event.payload).is_ok_and(|c| {
                        runner_core::app_paths::same_checkout_path(&c.root, &canonical)
                            && (c.paths.iter().any(|p| p == ".watch-ready")
                                || (c.paths.is_empty() && c.folders == [""]))
                    }) {
                        saw_changed = true;
                    }
                }
                Ok(event) if event.name == FILES_STATUS_EVENT => {
                    if serde_json::from_value::<FilesStatusChanged>(event.payload).is_ok_and(|u| {
                        runner_core::app_paths::same_checkout_path(&u.root, &canonical)
                            && u.status.entries.iter().any(|e| e.path == ".watch-ready")
                    }) {
                        saw_status = true;
                    }
                }
                Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
        if !(saw_changed && saw_status) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    assert!(saw_changed && saw_status, "sentinel watch readiness");
    std::fs::remove_file(&sentinel).unwrap();
    wait(&mut visual, "sentinel cleaned", &|v| {
        v.debug_bounds("PANEL_CHANGES_DIRTY_DOT").is_none()
    });

    // 3. Check out `branch-b` at the exact same HEAD while staying on Sessions: both the scope
    // pill AND the displayed diff comparison must switch from `(branch-a, main)` to
    // `(branch-b, refs/heads/base-b)` without reopening Changes!
    run_git(&["checkout", "-q", "branch-b"]);
    wait(
        &mut visual,
        "checking out branch-b at same HEAD updates scope pill and reloads against base-b on Sessions",
        &|v| {
            v.debug_bounds("PANEL_ACTIVITY Sessions active").is_some()
                && v.debug_bounds("CHANGES_DIFF_CODE").is_some()
                && v_ent.read_with(v, |viewer, _| {
                    let scope_ok = viewer
                        .active_diff_scope_label()
                        .is_some_and(|s| s.contains("branch-b") && s.contains("base-b"));
                    let diff_ok = viewer.loaded_diff_meta().is_some_and(|(_, base, _, old, _)| {
                        base == Some("refs/heads/base-b")
                            && old.is_some()
                            && old != Some(main_old_blob.as_str())
                    });
                    scope_ok && diff_ok
                })
        },
    );

    // 4. Detach HEAD while STILL on Sessions: scope pill must drop `branch-b` and must NOT fall
    // back to the original checkout branch name `branch-a`.
    run_git(&["checkout", "-q", "--detach", "HEAD"]);
    wait(
        &mut visual,
        "detached HEAD drops stale branch name from scope pill on Sessions",
        &|v| {
            v.debug_bounds("PANEL_ACTIVITY Sessions active").is_some()
                && v_ent.read_with(v, |viewer, _| {
                    viewer
                        .active_diff_scope_label()
                        .is_some_and(|s| !s.contains("branch-a") && !s.contains("branch-b"))
                })
        },
    );

    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn mission_sessions_clean_preview_clears_tab_badge_and_preserves_open_tab_and_width_on_changes_switch(
) {
    use runner_core::protocol::files::{
        FilesChanged, FilesStatusChanged, FILES_CHANGED_EVENT, FILES_STATUS_EVENT,
    };
    use std::process::Command;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (temp, mut cx, host, _fake) = suspended_mission_test();
    let repo = temp.path().join("mission-clean-preview-switch");
    std::fs::create_dir_all(&repo).unwrap();
    let run_git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(&repo)
            .args([
                "-c",
                "user.name=Runner",
                "-c",
                "user.email=runner@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run_git(&["init", "-q", "-b", "main"]);
    run_git(&["config", "core.autocrlf", "false"]);
    let original = "line_1 = \"original 1\"\nline_2 = \"original 2\"\n";
    std::fs::write(repo.join("alpha.py"), original).unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-q", "-m", "init"]);
    std::fs::write(
        repo.join("alpha.py"),
        "line_1 = \"edited 1\"\nline_2 = \"original 2\"\n",
    )
    .unwrap();

    let canonical = if cfg!(windows) {
        repo.clone()
    } else {
        repo.canonicalize().unwrap()
    };
    let store = host
        .read_with(&cx, |root, _| root.app_store.clone())
        .unwrap();
    let events = store.read_with(&cx, |s, _| s.test_core.events.clone());
    let mut sent = events.subscribe();

    host.update(&mut cx, |root, window, cx| {
        root.app_store.update(cx, |store, cx| {
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute(
                    "UPDATE missions SET cwd = ?1, files_root = ?1 WHERE id = 'test-mission'",
                    [canonical.to_string_lossy().as_ref()],
                )
                .unwrap();
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        root.open_mission("test-mission".into(), window, cx);
    })
    .unwrap();
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1600.), px(900.)));
    visual.run_until_parked();

    let click = |visual: &mut gpui::VisualTestContext, selector: &'static str| {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        visual.simulate_click(bounds.center(), gpui::Modifiers::default());
        visual.run_until_parked();
    };
    let wait = |visual: &mut gpui::VisualTestContext,
                what: &str,
                ready: &dyn Fn(&mut gpui::VisualTestContext) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready(visual) {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            std::thread::yield_now();
            visual.update(|window, _| window.refresh());
            visual.run_until_parked();
        }
    };
    let panel_width = |visual: &mut gpui::VisualTestContext| {
        visual
            .debug_bounds("MISSION_RAIL")
            .map(|b| f32::from(b.size.width))
    };
    let subject = crate::surfaces::side_panel::PanelSubject::Mission("test-mission".into());
    let viewer = |visual: &mut gpui::VisualTestContext| {
        host.read_with(visual, |r, cx| {
            r.mission_workspace.read(cx).rail_files.viewer(&subject)
        })
        .unwrap()
    };
    let changes_entity = |visual: &mut gpui::VisualTestContext| {
        host.read_with(visual, |r, cx| {
            r.mission_workspace
                .read(cx)
                .rail_files
                .changes_view(&subject)
        })
        .unwrap()
    };

    // 1. In mission Changes, open uncommitted `alpha.py` as the italic preview tab.
    click(&mut visual, "PANEL_ACTIVITY Changes");
    wait(&mut visual, "Changes lists alpha.py", &|v| {
        v.debug_bounds("CHANGES_ROW alpha.py").is_some()
    });
    click(&mut visual, "CHANGES_ROW alpha.py");
    wait(
        &mut visual,
        "alpha.py preview diff opened at 700 px",
        &|v| {
            v.debug_bounds("FILE_TAB_PREVIEW alpha.py").is_some()
                && v.debug_bounds("FILE_TAB_LETTER alpha.py M").is_some()
                && v.debug_bounds("CHANGES_DIFF_CODE").is_some()
                && panel_width(v).is_some_and(|w| (w - 700.).abs() < 2.)
        },
    );
    let widened_w = panel_width(&mut visual).unwrap();
    let v_ent = viewer(&mut visual).unwrap();
    let c_ent = changes_entity(&mut visual).unwrap();

    // 2. Switch to Sessions and prove live watcher readiness.
    click(&mut visual, "PANEL_ACTIVITY Sessions");
    wait(
        &mut visual,
        "Sessions rail active with viewer watching",
        &|v| {
            v.debug_bounds("PANEL_ACTIVITY Sessions active").is_some()
                && v.debug_bounds("CHANGES_DIFF_CODE").is_some()
                && v.debug_bounds("PANEL_CHANGES_DIRTY_DOT").is_some()
                && v_ent.read_with(v, |viewer, _| viewer.watched().is_some())
                && panel_width(v).is_some_and(|w| (w - widened_w).abs() < 0.5)
        },
    );
    while matches!(
        sent.try_recv(),
        Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
    ) {}
    let sentinel = canonical.join(".watch-ready");
    let mut probe = 0u32;
    let mut last_probe = std::time::Instant::now() - std::time::Duration::from_secs(2);
    let mut saw_changed = false;
    let mut saw_status = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline && !(saw_changed && saw_status) {
        if last_probe.elapsed() >= std::time::Duration::from_secs(1) {
            probe += 1;
            std::fs::write(&sentinel, format!("ready-{probe}\n")).unwrap();
            last_probe = std::time::Instant::now();
        }
        visual.update(|window, _| window.refresh());
        visual.run_until_parked();
        loop {
            match sent.try_recv() {
                Ok(event) if event.name == FILES_CHANGED_EVENT => {
                    if serde_json::from_value::<FilesChanged>(event.payload).is_ok_and(|c| {
                        runner_core::app_paths::same_checkout_path(&c.root, &canonical)
                            && (c.paths.iter().any(|p| p == ".watch-ready")
                                || (c.paths.is_empty() && c.folders == [""]))
                    }) {
                        saw_changed = true;
                    }
                }
                Ok(event) if event.name == FILES_STATUS_EVENT => {
                    if serde_json::from_value::<FilesStatusChanged>(event.payload).is_ok_and(|u| {
                        runner_core::app_paths::same_checkout_path(&u.root, &canonical)
                            && u.status.entries.iter().any(|e| e.path == ".watch-ready")
                    }) {
                        saw_status = true;
                    }
                }
                Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
        if !(saw_changed && saw_status) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    assert!(saw_changed && saw_status, "sentinel watch readiness");
    std::fs::remove_file(&sentinel).unwrap();

    // 3. Restore `alpha.py` to clean on disk while on Sessions: viewer falls back to Code,
    // dirty dot clears, stale M badge disappears, and width stays 700 px.
    std::fs::write(repo.join("alpha.py"), original).unwrap();
    wait(
        &mut visual,
        "alpha.py falls back to clean Code with no M badge and no dirty dot",
        &|v| {
            v.debug_bounds("FILE_CODE").is_some()
                && v.debug_bounds("FILE_TAB alpha.py").is_some()
                && v.debug_bounds("FILE_TAB_LETTER alpha.py M").is_none()
                && v.debug_bounds("VIEWER_MODE_DIFF").is_none()
                && v.debug_bounds("PANEL_CHANGES_DIRTY_DOT").is_none()
                && panel_width(v).is_some_and(|w| (w - widened_w).abs() < 0.5)
        },
    );

    // 4. Switch right rail back to Changes while `alpha.py` is visibly open:
    // Changes refreshes to 0 uncommitted files, yet `alpha.py` stays open at 700 px (0 px jump).
    click(&mut visual, "PANEL_ACTIVITY Changes");
    wait(
        &mut visual,
        "Changes settles empty while preserving open alpha.py and 700 px width",
        &|v| {
            v.debug_bounds("PANEL_ACTIVITY Changes active").is_some()
                && c_ent.read_with(v, |c, _| c.fresh_dirty_changes() == Some(false))
                && v.debug_bounds("CHANGES_ROW alpha.py").is_none()
                && v.debug_bounds("FILE_TAB alpha.py").is_some()
                && v.debug_bounds("FILE_CODE").is_some()
                && v.debug_bounds("FILE_TAB_LETTER alpha.py M").is_none()
                && panel_width(v).is_some_and(|w| (w - widened_w).abs() < 0.5)
        },
    );

    // 5. Re-dirty `alpha.py`, open it as preview on Changes, then restore it to clean while
    // staying directly on Changes: emptying Uncommitted must also transition the sole active
    // preview cleanly to Code at 700 px instead of collapsing the panel without a close action.
    click(&mut visual, "FILE_TAB_CLOSE alpha.py");
    wait(&mut visual, "explicit close shrinks back to 300 px", &|v| {
        v.debug_bounds("FILE_TAB alpha.py").is_none()
            && panel_width(v).is_some_and(|w| (w - 300.).abs() < 2.)
    });
    std::fs::write(
        repo.join("alpha.py"),
        "line_1 = \"edited again\"\nline_2 = \"original 2\"\n",
    )
    .unwrap();
    wait(&mut visual, "Changes lists alpha.py again", &|v| {
        v.debug_bounds("CHANGES_ROW alpha.py").is_some()
    });
    click(&mut visual, "CHANGES_ROW alpha.py");
    wait(&mut visual, "alpha.py preview reopened on Changes", &|v| {
        v.debug_bounds("FILE_TAB_PREVIEW alpha.py").is_some()
            && v.debug_bounds("CHANGES_DIFF_CODE").is_some()
            && panel_width(v).is_some_and(|w| (w - widened_w).abs() < 2.)
    });
    std::fs::write(repo.join("alpha.py"), original).unwrap();
    wait(
        &mut visual,
        "sole preview transitions to clean Code on Changes without collapsing panel",
        &|v| {
            v.debug_bounds("CHANGES_ROW alpha.py").is_none()
                && v.debug_bounds("FILE_TAB alpha.py").is_some()
                && v.debug_bounds("FILE_CODE").is_some()
                && v.debug_bounds("FILE_TAB_LETTER alpha.py M").is_none()
                && v.debug_bounds("VIEWER_MODE_DIFF").is_none()
                && panel_width(v).is_some_and(|w| (w - widened_w).abs() < 0.5)
        },
    );

    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn held_live_changes_snapshot_released_after_switching_to_sessions_does_not_replace_preview() {
    use runner_core::protocol::files::{
        FilesChanged, FilesStatusChanged, FILES_CHANGED_EVENT, FILES_STATUS_EVENT,
    };
    use std::process::Command;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (temp, mut cx, host, _fake) = suspended_mission_test();
    let repo = temp.path().join("mission-held-snapshot-hidden-changes");
    std::fs::create_dir_all(&repo).unwrap();
    let run_git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(&repo)
            .args([
                "-c",
                "user.name=Runner",
                "-c",
                "user.email=runner@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run_git(&["init", "-q", "-b", "main"]);
    run_git(&["config", "core.autocrlf", "false"]);
    std::fs::write(repo.join("a.rs"), "pub fn a() { 0 }\n").unwrap();
    std::fs::write(repo.join("b.rs"), "pub fn b() { 0 }\n").unwrap();
    std::fs::write(repo.join("c.rs"), "pub fn c() { 0 }\n").unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-q", "-m", "init"]);

    // `topic` has 3 commits ahead of `alt-base` (starting at `main`):
    // commit 1 changes `a.rs`, commit 2 changes `b.rs`, commit 3 changes `c.rs`.
    // Moving `alt-base` forward to commit 1 leaves `b.rs` and `c.rs` in `All changes`
    // while removing `a.rs`.
    run_git(&["branch", "alt-base", "main"]);
    run_git(&["checkout", "-q", "-b", "topic"]);
    std::fs::write(repo.join("a.rs"), "pub fn a() { 1 }\n").unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-q", "-m", "change a"]);
    std::fs::write(repo.join("b.rs"), "pub fn b() { 1 }\n").unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-q", "-m", "change b"]);
    std::fs::write(repo.join("c.rs"), "pub fn c() { 1 }\n").unwrap();
    run_git(&["add", "-A"]);
    run_git(&["commit", "-q", "-m", "change c"]);

    let canonical = if cfg!(windows) {
        repo.clone()
    } else {
        repo.canonicalize().unwrap()
    };
    let store = host
        .read_with(&cx, |root, _| root.app_store.clone())
        .unwrap();
    let events = store.read_with(&cx, |s, _| s.test_core.events.clone());
    let mut sent = events.subscribe();

    host.update(&mut cx, |root, window, cx| {
        root.app_store.update(cx, |store, cx| {
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute(
                    "UPDATE missions SET cwd = ?1, files_root = ?1 WHERE id = 'test-mission'",
                    [canonical.to_string_lossy().as_ref()],
                )
                .unwrap();
            store.update_settings(
                |settings| {
                    settings.set_changes_base(
                        &canonical,
                        Some("topic"),
                        Some("refs/heads/alt-base"),
                    )
                },
                false,
                cx,
            );
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        root.open_mission("test-mission".into(), window, cx);
    })
    .unwrap();
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1600.), px(900.)));
    visual.run_until_parked();

    let click = |visual: &mut gpui::VisualTestContext, selector: &'static str| {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        visual.simulate_click(bounds.center(), gpui::Modifiers::default());
        visual.run_until_parked();
    };
    let wait = |visual: &mut gpui::VisualTestContext,
                what: &str,
                ready: &dyn Fn(&mut gpui::VisualTestContext) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready(visual) {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            std::thread::yield_now();
            visual.update(|window, _| window.refresh());
            visual.run_until_parked();
        }
    };
    let subject = crate::surfaces::side_panel::PanelSubject::Mission("test-mission".into());
    let viewer = |visual: &mut gpui::VisualTestContext| {
        host.read_with(visual, |r, cx| {
            r.mission_workspace.read(cx).rail_files.viewer(&subject)
        })
        .unwrap()
    };
    let changes_entity = |visual: &mut gpui::VisualTestContext| {
        host.read_with(visual, |r, cx| {
            r.mission_workspace
                .read(cx)
                .rail_files
                .changes_view(&subject)
        })
        .unwrap()
    };

    // 1. Open Changes (`All changes` vs `alt-base`) and preview `a.rs` while `b.rs` is also changed.
    click(&mut visual, "PANEL_ACTIVITY Changes");
    wait(
        &mut visual,
        "Changes lists a.rs and b.rs under All changes",
        &|v| {
            v.debug_bounds("CHANGES_BASE_CHIP alt-base").is_some()
                && v.debug_bounds("CHANGES_ROW a.rs").is_some()
                && v.debug_bounds("CHANGES_ROW b.rs").is_some()
        },
    );
    click(&mut visual, "CHANGES_ROW a.rs");
    wait(&mut visual, "a.rs preview diff loaded", &|v| {
        v.debug_bounds("FILE_TAB_PREVIEW a.rs").is_some()
            && v.debug_bounds("CHANGES_DIFF_CODE").is_some()
    });
    let v_ent = viewer(&mut visual).unwrap();
    let c_ent = changes_entity(&mut visual).unwrap();

    // 2. Prove live OS watcher readiness while Changes is active.
    while matches!(
        sent.try_recv(),
        Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
    ) {}
    let sentinel = canonical.join(".watch-ready");
    let mut probe = 0u32;
    let mut last_probe = std::time::Instant::now() - std::time::Duration::from_secs(2);
    let mut saw_changed = false;
    let mut saw_status = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline && !(saw_changed && saw_status) {
        if last_probe.elapsed() >= std::time::Duration::from_secs(1) {
            probe += 1;
            std::fs::write(&sentinel, format!("ready-{probe}\n")).unwrap();
            last_probe = std::time::Instant::now();
        }
        visual.update(|window, _| window.refresh());
        visual.run_until_parked();
        loop {
            match sent.try_recv() {
                Ok(event) if event.name == FILES_CHANGED_EVENT => {
                    if serde_json::from_value::<FilesChanged>(event.payload).is_ok_and(|c| {
                        runner_core::app_paths::same_checkout_path(&c.root, &canonical)
                            && (c.paths.iter().any(|p| p == ".watch-ready")
                                || (c.paths.is_empty() && c.folders == [""]))
                    }) {
                        saw_changed = true;
                    }
                }
                Ok(event) if event.name == FILES_STATUS_EVENT => {
                    if serde_json::from_value::<FilesStatusChanged>(event.payload).is_ok_and(|u| {
                        runner_core::app_paths::same_checkout_path(&u.root, &canonical)
                            && u.status.entries.iter().any(|e| e.path == ".watch-ready")
                    }) {
                        saw_status = true;
                    }
                }
                Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
        if !(saw_changed && saw_status) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    assert!(saw_changed && saw_status, "sentinel watch readiness");
    std::fs::remove_file(&sentinel).unwrap();
    wait(&mut visual, "sentinel cleared from Changes", &|v| {
        v.debug_bounds("CHANGES_ROW .watch-ready").is_none()
            && v.debug_bounds("CHANGES_ROW a.rs").is_some()
            && v.debug_bounds("CHANGES_ROW b.rs").is_some()
    });

    // 3. Hold live Changes snapshots, move `alt-base` to commit 1 (so only `b.rs`/`c.rs` remain
    // under `All changes`), wait for the live snapshot (`allow_preview_replace = true`) to be
    // captured in flight, switch to Sessions, and then release the held Changes snapshot.
    c_ent.update(&mut visual, |c, _| c.set_hold_snapshots(true));
    run_git(&["branch", "-f", "alt-base", "HEAD~2"]);
    wait(
        &mut visual,
        "live Changes snapshot captured in flight",
        &|v| c_ent.read_with(v, |c, _| c.held_snapshots_len() > 0),
    );

    click(&mut visual, "PANEL_ACTIVITY Sessions");
    wait(&mut visual, "Sessions rail active", &|v| {
        v.debug_bounds("PANEL_ACTIVITY Sessions active").is_some()
    });

    c_ent.update(&mut visual, |c, _| {
        c.set_hold_snapshots(false);
        assert!(c.release_held_snapshots() > 0);
    });
    // Drive the released ChangesView snapshot completion task to completion before asserting.
    visual.run_until_parked();
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    // Wait for Sessions's own viewer refresh to settle `a.rs` to NoChanges; the released stale
    // Changes snapshot must NOT replace `a.rs` with `b.rs`.
    wait(
        &mut visual,
        "a.rs stays open as NoChanges on Sessions after releasing stale Changes snapshot",
        &|v| {
            v.debug_bounds("PANEL_ACTIVITY Sessions active").is_some()
                && v.debug_bounds("FILE_DIFF_NO_CHANGES").is_some()
                && v_ent.read_with(v, |viewer, _| {
                    viewer.open_tabs() == vec!["a.rs".to_string()]
                        && viewer.active_tab().as_deref() == Some("a.rs")
                })
        },
    );

    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[cfg(unix)]
#[test]
fn sessions_dirty_dot_watches_main_checkout_clean_dirty_clean_and_releases_watch() {
    use runner_core::protocol::files::{
        FilesChanged, FilesStatusChanged, FILES_CHANGED_EVENT, FILES_STATUS_EVENT,
    };
    use std::process::Command;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (temp, mut cx, host, _fake) = suspended_mission_test();
    let repo = temp.path().join("mission-main-watch");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "core.autocrlf", "false"][..],
    ] {
        assert!(Command::new("git")
            .current_dir(&repo)
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .unwrap()
            .success());
    }
    std::fs::write(repo.join("lib.rs"), "pub fn clean() {}\n").unwrap();
    for args in [
        &["add", "-A"][..],
        &[
            "-c",
            "user.name=Runner",
            "-c",
            "user.email=runner@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "init",
        ][..],
    ] {
        assert!(Command::new("git")
            .current_dir(&repo)
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .unwrap()
            .success());
    }
    let canonical = if cfg!(windows) {
        repo.clone()
    } else {
        repo.canonicalize().unwrap()
    };
    let store = host
        .read_with(&cx, |root, _| root.app_store.clone())
        .unwrap();
    let events = store.read_with(&cx, |store, _| store.test_core.events.clone());
    let mut sent = events.subscribe();

    host.update(&mut cx, |root, window, cx| {
        root.app_store.update(cx, |store, cx| {
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute(
                    "UPDATE missions SET cwd = ?1, files_root = ?1 WHERE id = 'test-mission'",
                    [canonical.to_string_lossy().as_ref()],
                )
                .unwrap();
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        root.open_mission("test-mission".into(), window, cx);
    })
    .unwrap();
    cx.run_until_parked();

    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1600.), px(900.)));
    visual.run_until_parked();

    let wait = |visual: &mut gpui::VisualTestContext,
                what: &str,
                ready: &dyn Fn(&mut gpui::VisualTestContext) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready(visual) {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            std::thread::yield_now();
            visual.update(|window, _| window.refresh());
            visual.run_until_parked();
        }
    };
    let watched_roots = |visual: &gpui::VisualTestContext| {
        store.read_with(visual, |store, _| store.test_core.files.watched_roots())
    };

    // 1. Mission opens on Sessions on a clean main checkout: DirtyTracker acquires a daemon watch
    // and delivers its startup status event (subscribed before open_mission).
    wait(
        &mut visual,
        "Sessions tracker watches main checkout",
        &|v| {
            watched_roots(v)
                .iter()
                .any(|p| runner_core::app_paths::same_checkout_path(p, &canonical))
        },
    );
    let mut saw_startup = false;
    let watcher_deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < watcher_deadline && !saw_startup {
        visual.run_until_parked();
        loop {
            match sent.try_recv() {
                Ok(event) if event.name == FILES_STATUS_EVENT => {
                    if serde_json::from_value::<FilesStatusChanged>(event.payload).is_ok_and(
                        |update| {
                            runner_core::app_paths::same_checkout_path(&update.root, &canonical)
                        },
                    ) {
                        saw_startup = true;
                        break;
                    }
                }
                Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
        if !saw_startup {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    assert!(
        saw_startup,
        "timed out waiting for startup FILES_STATUS_EVENT on main checkout"
    );
    assert!(
        visual.debug_bounds("PANEL_CHANGES_DIRTY_DOT").is_none(),
        "clean main checkout must not show dirty dot"
    );

    // Prove OS watcher delivery is live via an independent sentinel before touching `lib.rs`.
    let sentinel = repo.join(".watch-ready");
    std::fs::write(&sentinel, "ready\n").unwrap();
    let mut saw_changed = false;
    let mut saw_status = false;
    let sentinel_deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < sentinel_deadline && !(saw_changed && saw_status) {
        visual.run_until_parked();
        loop {
            match sent.try_recv() {
                Ok(event) if event.name == FILES_CHANGED_EVENT => {
                    if serde_json::from_value::<FilesChanged>(event.payload).is_ok_and(|changed| {
                        runner_core::app_paths::same_checkout_path(&changed.root, &canonical)
                            && (changed.paths.iter().any(|p| p == ".watch-ready")
                                || (changed.paths.is_empty() && changed.folders == [""]))
                    }) {
                        saw_changed = true;
                    }
                }
                Ok(event) if event.name == FILES_STATUS_EVENT => {
                    if serde_json::from_value::<FilesStatusChanged>(event.payload).is_ok_and(
                        |update| {
                            runner_core::app_paths::same_checkout_path(&update.root, &canonical)
                                && update
                                    .status
                                    .entries
                                    .iter()
                                    .any(|e| e.path == ".watch-ready")
                        },
                    ) {
                        saw_status = true;
                    }
                }
                Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
        if !(saw_changed && saw_status) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    assert!(
        saw_changed && saw_status,
        "timed out waiting for sentinel readiness (saw_changed={saw_changed}, saw_status={saw_status})"
    );
    wait(&mut visual, "sentinel dirties Sessions dot", &|v| {
        v.debug_bounds("PANEL_CHANGES_DIRTY_DOT").is_some()
    });
    std::fs::remove_file(&sentinel).unwrap();
    wait(
        &mut visual,
        "sentinel removed and main checkout returns clean",
        &|v| v.debug_bounds("PANEL_CHANGES_DIRTY_DOT").is_none(),
    );

    // 2. Clean -> dirty -> clean with a single target write while staying on Sessions.
    std::fs::write(repo.join("lib.rs"), "pub fn dirty() {}\n").unwrap();
    wait(
        &mut visual,
        "Sessions dirty dot appears after single write to lib.rs",
        &|v| v.debug_bounds("PANEL_CHANGES_DIRTY_DOT").is_some(),
    );

    assert!(Command::new("git")
        .current_dir(&repo)
        .args(["checkout", "--", "lib.rs"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .unwrap()
        .success());
    wait(
        &mut visual,
        "Sessions dirty dot clears after cleaning main checkout",
        &|v| v.debug_bounds("PANEL_CHANGES_DIRTY_DOT").is_none(),
    );

    // 3. Switching to Files transfers watch ownership and deactivates DirtyTracker (releasing its
    // watch and FilesHub subscription); switching back to Sessions re-activates it.
    let tracker_active = |visual: &gpui::VisualTestContext| {
        host.read_with(visual, |root, cx| {
            root.mission_workspace
                .read(cx)
                .rail_files
                .dirty_tracker_active(cx)
        })
        .unwrap()
    };
    assert!(tracker_active(&visual));
    let files_btn = visual
        .debug_bounds("PANEL_ACTIVITY Files")
        .expect("Files activity button");
    visual.simulate_click(files_btn.center(), gpui::Modifiers::default());
    wait(
        &mut visual,
        "Files takes over watch and deactivates DirtyTracker",
        &|v| v.debug_bounds("FILES_ROW lib.rs").is_some() && !tracker_active(v),
    );
    let sessions_btn = visual
        .debug_bounds("PANEL_ACTIVITY Sessions")
        .expect("Sessions activity button");
    visual.simulate_click(sessions_btn.center(), gpui::Modifiers::default());
    wait(
        &mut visual,
        "returning to Sessions re-activates DirtyTracker watch",
        &|v| {
            tracker_active(v)
                && watched_roots(v)
                    .iter()
                    .any(|p| runner_core::app_paths::same_checkout_path(p, &canonical))
        },
    );

    // 4. Disabling Changes in settings releases the DirtyTracker watch cleanly.
    store.update(&mut visual, |store, cx| {
        store.update_settings(
            |s| {
                s.changes_view = false;
                true
            },
            false,
            cx,
        );
    });
    wait(
        &mut visual,
        "disabling Changes releases Sessions dirty watch",
        &|v| !tracker_active(v) && watched_roots(v).is_empty(),
    );

    // 5. Re-enabling Changes re-acquires the watch; collapsing the rail releases it.
    store.update(&mut visual, |store, cx| {
        store.update_settings(
            |s| {
                s.changes_view = true;
                true
            },
            false,
            cx,
        );
    });
    wait(&mut visual, "re-enabling Changes re-acquires watch", &|v| {
        tracker_active(v)
            && watched_roots(v)
                .iter()
                .any(|p| runner_core::app_paths::same_checkout_path(p, &canonical))
    });
    host.update(&mut visual, |root, _, cx| {
        root.mission_workspace.update(cx, |ws, cx| {
            ws.set_rail_open(false, cx);
            cx.notify();
        });
    })
    .unwrap();
    wait(
        &mut visual,
        "collapsing rail releases Sessions watch",
        &|v| !tracker_active(v) && watched_roots(v).is_empty(),
    );

    // 6. Re-opening rail re-acquires the watch; navigating away from the mission hides workspace
    // and releases the watch.
    host.update(&mut visual, |root, _, cx| {
        root.mission_workspace.update(cx, |ws, cx| {
            ws.set_rail_open(true, cx);
            cx.notify();
        });
    })
    .unwrap();
    wait(&mut visual, "re-opening rail re-acquires watch", &|v| {
        tracker_active(v)
            && watched_roots(v)
                .iter()
                .any(|p| runner_core::app_paths::same_checkout_path(p, &canonical))
    });
    host.update(&mut visual, |root, _, cx| {
        root.set_route(crate::AppRoute::Roles, cx);
        cx.notify();
    })
    .unwrap();
    wait(
        &mut visual,
        "navigating away from mission releases Sessions watch",
        &|v| !tracker_active(v) && watched_roots(v).is_empty(),
    );

    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn mission_rail_info_shows_branch_row_only_for_linked_worktree() {
    use std::process::Command;

    let (temp, mut cx, host, _fake) = suspended_mission_test();
    let repo = temp.path().join("mission-info-repo");
    let wt = repo.join(".worktrees").join("feat-rail");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "core.autocrlf", "false"][..],
    ] {
        assert!(Command::new("git")
            .current_dir(&repo)
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .unwrap()
            .success());
    }
    std::fs::write(repo.join("lib.rs"), "pub fn m() {}\n").unwrap();
    for args in [
        &["add", "-A"][..],
        &[
            "-c",
            "user.name=Runner",
            "-c",
            "user.email=runner@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "init",
        ][..],
        &[
            "worktree",
            "add",
            "-b",
            "feat/rail",
            wt.to_str().unwrap(),
            "main",
        ][..],
    ] {
        assert!(Command::new("git")
            .current_dir(&repo)
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .unwrap()
            .success());
    }
    let repo_canon = repo.canonicalize().unwrap();
    let wt_canon = wt.canonicalize().unwrap();

    host.update(&mut cx, |root, window, cx| {
        root.app_store.update(cx, |store, cx| {
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute(
                    "UPDATE missions SET cwd = ?1, files_root = ?1 WHERE id = 'test-mission'",
                    [repo_canon.to_string_lossy().as_ref()],
                )
                .unwrap();
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        root.open_mission("test-mission".into(), window, cx);
    })
    .unwrap();

    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1600.), px(900.)));

    let wait = |visual: &mut gpui::VisualTestContext,
                what: &str,
                ready: &dyn Fn(&mut gpui::VisualTestContext) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready(visual) {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            std::thread::yield_now();
            visual.update(|window, _| window.refresh());
            visual.run_until_parked();
        }
    };

    wait(&mut visual, "test-mission loaded", &|v| {
        host.read_with(v, |root, cx| {
            root.mission_workspace
                .read(cx)
                .mission_view()
                .is_some_and(|m| m.id == "test-mission")
        })
        .unwrap()
    });
    host.update(&mut visual, |root, window, cx| {
        root.root_focus.focus(window, cx);
    })
    .unwrap();
    visual.run_until_parked();
    visual.dispatch_action(crate::ToggleSessionInfo);
    wait(&mut visual, "main mission checkout resolved", &|v| {
        host.read_with(v, |root, cx| {
            root.app_store
                .read(cx)
                .worktree_checkout_for(repo_canon.to_string_lossy().as_ref(), None)
                .is_some()
        })
        .unwrap()
            && v.debug_bounds("MISSION_META_ID").is_some()
    });
    assert!(visual.debug_bounds("MISSION_META_BRANCH").is_none());

    host.update(&mut visual, |root, window, cx| {
        root.mission_workspace.update(cx, |workspace, cx| {
            workspace.toggle_mission_info_popover(window, cx);
        });
        root.app_store.update(cx, |store, cx| {
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute(
                    "INSERT INTO missions (id, crew_id, title, status, cwd, files_root, started_at)
                     VALUES ('wt-mission', (SELECT crew_id FROM missions WHERE id = 'test-mission'), 'WT Mission', 'completed', ?1, ?1, '2026-11-17T00:00:00Z')",
                    [wt_canon.to_string_lossy().as_ref()],
                )
                .unwrap();
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        root.open_mission("wt-mission".into(), window, cx);
    })
    .unwrap();
    wait(&mut visual, "wt-mission loaded", &|v| {
        host.read_with(v, |root, cx| {
            root.mission_workspace
                .read(cx)
                .mission_view()
                .is_some_and(|m| m.id == "wt-mission")
        })
        .unwrap()
    });
    host.update(&mut visual, |root, window, cx| {
        root.mission_workspace.update(cx, |workspace, cx| {
            workspace.toggle_mission_info_popover(window, cx);
            cx.notify();
        });
    })
    .unwrap();
    wait(&mut visual, "linked worktree mission branch row", &|v| {
        v.debug_bounds("MISSION_META_BRANCH").is_some()
            && v.debug_bounds("INFO_WORKTREE_MARKER Worktree · from main")
                .is_some()
    });

    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

fn seed_local_mission_drawer_shells(
    host: gpui::WindowHandle<NativeRoot>,
    cx: &mut gpui::TestAppContext,
    core: &runner_daemon::AppCore,
    shells: &[&str],
    open: bool,
) -> String {
    let mut layout = MissionLayout::default();
    for &id in shells {
        layout.drawer.add(id.to_owned());
    }
    layout.drawer.set_open(open);
    let node_id = {
        let conn = core.db.get().unwrap();
        for &id in shells {
            conn.execute(
                "INSERT INTO sessions (id, role_id, mission_id, slot_id, status, agent_runtime, agent_command)
                 VALUES (?1, 'test-shell-role', NULL, NULL, 'running', 'shell', '/bin/sh')",
                [id],
            )
            .unwrap();
        }
        let node =
            runner_daemon::repo::node::ensure_mission_node(&conn, "test-mission", None).unwrap();
        runner_daemon::repo::node::set_layout(&conn, &node.id, &layout.serialize().unwrap())
            .unwrap();
        node.id
    };
    let events: Arc<dyn runner_daemon::session::manager::SessionEvents> =
        Arc::new(core.session_events());
    host.update(cx, |root, _, cx| {
        root.app_store.update(cx, |store, store_cx| {
            for &id in shells {
                core.sessions
                    .prepare_unlisted_terminal(id, (80, 24), &core.db, &events)
                    .unwrap();
                store.bridge.attach(id).unwrap();
            }
            store.refresh_sessions(store_cx);
            store.refresh_nodes(store_cx).unwrap();
        });
    })
    .unwrap();
    node_id
}

fn wait_local_mission(
    visual: &mut gpui::VisualTestContext,
    what: &str,
    mut ready: impl FnMut(&mut gpui::VisualTestContext) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        visual.run_until_parked();
        if ready(visual) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        std::thread::yield_now();
    }
}

#[test]
fn local_mission_drawer_cmd_w_closes_shells_persists_layout_and_restores_focus() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for on_session_tab in [false, true] {
        let (_temp, mut cx, host, fake) = suspended_mission_test();
        let core = fake.base.0.clone();
        let node_id = seed_local_mission_drawer_shells(
            host,
            &mut cx,
            &core,
            &["drawer-1", "drawer-2", "drawer-3"],
            true,
        );

        host.update(&mut cx, |root, window, cx| {
            root.open_mission("test-mission".into(), window, cx);
        })
        .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(gpui::size(px(1600.), px(900.)));
        visual.update(|window, _| window.activate_window());

        wait_local_mission(&mut visual, "mission and drawer shells loaded", |v| {
            host.read_with(v, |root, cx| {
                let ws = root.mission_workspace.read(cx);
                !ws.loading
                    && ws.layout.drawer.open()
                    && ws.layout.drawer.shells() == ["drawer-1", "drawer-2", "drawer-3"]
                    && ws.attached.contains_key("drawer-1")
                    && ws.attached.contains_key("drawer-2")
                    && ws.attached.contains_key("drawer-3")
                    && ws.attached.contains_key("mission-agent")
            })
            .unwrap()
                && v.debug_bounds("MISSION_TERMINAL_DRAWER").is_some()
        });

        // Focus non-final middle shell drawer-2 so closing it must select predecessor drawer-1.
        let (focus_shell_1, focus_shell_2, focus_shell_3, focus_member, focus_root) = host
            .update(&mut visual, |root, window, cx| {
                root.mission_workspace.update(cx, |ws, cx| {
                    if on_session_tab {
                        ws.select_mission_session("mission-agent", window, cx);
                    } else {
                        ws.select_mission_feed(window, cx);
                    }
                    ws.activate_terminal_drawer_shell("drawer-2", window, cx);
                    (
                        ws.attached["drawer-1"].terminal_focus.clone(),
                        ws.attached["drawer-2"].terminal_focus.clone(),
                        ws.attached["drawer-3"].terminal_focus.clone(),
                        ws.attached["mission-agent"].terminal_focus.clone(),
                        ws.root_focus.clone(),
                    )
                })
            })
            .unwrap();
        wait_local_mission(&mut visual, "drawer-2 focused", |v| {
            host.update(v, |root, window, cx| {
                let ws = root.mission_workspace.read(cx);
                focus_shell_2.is_focused(window)
                    && ws.drawer_focused(window, cx)
                    && ws.drawer_was_focused
            })
            .unwrap()
        });

        // First Cmd+W closes non-final active drawer-2 -> selects predecessor drawer-1.
        host.update(&mut visual, crate::surfaces::windowing::close_tab)
            .unwrap();
        wait_local_mission(
            &mut visual,
            "drawer-2 closed and focus moved to drawer-1",
            |v| {
                let state_ok = host
                    .read_with(v, |root, cx| {
                        let ws = root.mission_workspace.read(cx);
                        ws.layout.drawer.open()
                            && ws.layout.drawer.shells() == ["drawer-1", "drawer-3"]
                            && ws.layout.drawer.active_shell() == Some("drawer-1")
                            && !ws.attached.contains_key("drawer-2")
                            && ws.drawer_was_focused
                    })
                    .unwrap();
                let focus_ok = v.update(|window, _| focus_shell_1.is_focused(window));
                let db_ok = {
                    let conn = core.db.get().unwrap();
                    let row = runner_daemon::repo::node::get(&conn, &node_id)
                        .unwrap()
                        .unwrap();
                    let persisted = MissionLayout::from_node_row(&row).unwrap();
                    let gone = runner_daemon::repo::session::get_row(&conn, "drawer-2")
                        .unwrap()
                        .is_none();
                    gone && persisted.drawer.open()
                        && persisted.drawer.shells() == ["drawer-1", "drawer-3"]
                        && persisted.drawer.active_shell() == Some("drawer-1")
                };
                state_ok && focus_ok && db_ok
            },
        );

        // Second Cmd+W closes index-0 drawer-1 -> moves focus to remaining drawer-3.
        host.update(&mut visual, crate::surfaces::windowing::close_tab)
            .unwrap();
        wait_local_mission(
            &mut visual,
            "drawer-1 closed and focus moved to drawer-3",
            |v| {
                let state_ok = host
                    .read_with(v, |root, cx| {
                        let ws = root.mission_workspace.read(cx);
                        ws.layout.drawer.open()
                            && ws.layout.drawer.shells() == ["drawer-3"]
                            && ws.layout.drawer.active_shell() == Some("drawer-3")
                            && !ws.attached.contains_key("drawer-1")
                            && ws.drawer_was_focused
                    })
                    .unwrap();
                let focus_ok = v.update(|window, _| focus_shell_3.is_focused(window));
                let db_ok = {
                    let conn = core.db.get().unwrap();
                    let row = runner_daemon::repo::node::get(&conn, &node_id)
                        .unwrap()
                        .unwrap();
                    let persisted = MissionLayout::from_node_row(&row).unwrap();
                    let gone = runner_daemon::repo::session::get_row(&conn, "drawer-1")
                        .unwrap()
                        .is_none();
                    gone && persisted.drawer.open()
                        && persisted.drawer.shells() == ["drawer-3"]
                        && persisted.drawer.active_shell() == Some("drawer-3")
                };
                state_ok && focus_ok && db_ok
            },
        );

        // Third Cmd+W closes last drawer-3, hides drawer, clears drawer_was_focused, and
        // returns focus to the active mission surface.
        host.update(&mut visual, crate::surfaces::windowing::close_tab)
            .unwrap();
        wait_local_mission(
            &mut visual,
            "last drawer shell closed, drawer hidden, focus restored to mission",
            |v| {
                let state_ok = host
                    .read_with(v, |root, cx| {
                        let ws = root.mission_workspace.read(cx);
                        !ws.layout.drawer.open()
                            && ws.layout.drawer.shells().is_empty()
                            && !ws.attached.contains_key("drawer-3")
                            && !ws.drawer_was_focused
                    })
                    .unwrap();
                let focus_ok = v.update(|window, _| {
                    if on_session_tab {
                        focus_member.is_focused(window)
                    } else {
                        focus_root.is_focused(window)
                    }
                });
                let db_ok = {
                    let conn = core.db.get().unwrap();
                    let row = runner_daemon::repo::node::get(&conn, &node_id)
                        .unwrap()
                        .unwrap();
                    let persisted = MissionLayout::from_node_row(&row).unwrap();
                    let gone = runner_daemon::repo::session::get_row(&conn, "drawer-3")
                        .unwrap()
                        .is_none();
                    gone && !persisted.drawer.open() && persisted.drawer.shells().is_empty()
                };
                state_ok && focus_ok && db_ok
            },
        );

        host.update(&mut visual, |_, window, _| window.remove_window())
            .unwrap();
    }
}

#[test]
fn local_mission_drawer_cmd_w_respects_intervening_focus_change_and_concurrent_layout_edits() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, host, fake) = suspended_mission_test();
    let core = fake.base.0.clone();
    let node_id =
        seed_local_mission_drawer_shells(host, &mut cx, &core, &["drawer-1", "drawer-2"], true);

    host.update(&mut cx, |root, window, cx| {
        root.open_mission("test-mission".into(), window, cx);
    })
    .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1600.), px(900.)));
    visual.update(|window, _| window.activate_window());

    wait_local_mission(&mut visual, "mission loaded for gated close test", |v| {
        host.read_with(v, |root, cx| {
            let ws = root.mission_workspace.read(cx);
            !ws.loading
                && ws.layout.drawer.open()
                && ws.layout.drawer.shells() == ["drawer-1", "drawer-2"]
                && ws.attached.contains_key("drawer-1")
                && ws.attached.contains_key("drawer-2")
        })
        .unwrap()
            && v.debug_bounds("MISSION_TERMINAL_DRAWER").is_some()
    });

    let (focus_shell_1, focus_shell_2, focus_composer) = host
        .update(&mut visual, |root, window, cx| {
            root.mission_workspace.update(cx, |ws, cx| {
                ws.select_mission_feed(window, cx);
                ws.activate_terminal_drawer_shell("drawer-2", window, cx);
                (
                    ws.attached["drawer-1"].terminal_focus.clone(),
                    ws.attached["drawer-2"].terminal_focus.clone(),
                    ws.composer_input.read(cx).focus_handle(),
                )
            })
        })
        .unwrap();
    wait_local_mission(&mut visual, "drawer-2 focused before gated close", |v| {
        v.update(|window, _| focus_shell_2.is_focused(window))
    });

    // Gate applying drawer-2's close reply, move focus to Feed composer, and simulate a
    // concurrent shell addition in SQLite before releasing the UI callback.
    let (gate, handle) = crate::test_support::StagedReplyGate::pair();
    host.update(&mut visual, |root, window, cx| {
        root.mission_workspace
            .update(cx, |ws, _| ws.drawer_close_gates.push_back(gate));
        crate::surfaces::windowing::close_tab(root, window, cx);
    })
    .unwrap();
    wait_local_mission(&mut visual, "drawer-2 close reply entered gate", |_| {
        handle.is_entered()
    });

    visual.update(|window, cx| focus_composer.focus(window, cx));
    wait_local_mission(&mut visual, "composer focused while close in flight", |v| {
        v.update(|window, _| focus_composer.is_focused(window))
    });

    {
        let conn = core.db.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, role_id, mission_id, slot_id, status, agent_runtime, agent_command)
             VALUES ('drawer-concurrent', 'test-shell-role', NULL, NULL, 'running', 'shell', '/bin/sh')",
            [],
        )
        .unwrap();
        let row = runner_daemon::repo::node::get(&conn, &node_id)
            .unwrap()
            .unwrap();
        let mut layout = MissionLayout::from_node_row(&row).unwrap();
        layout.drawer.add("drawer-concurrent".into());
        runner_daemon::repo::node::set_layout(&conn, &node_id, &layout.serialize().unwrap())
            .unwrap();
    }

    let applied = handle.release();
    wait_local_mission(
        &mut visual,
        "drawer-2 closed while preserving concurrent shell and composer focus",
        |v| {
            let ui_ok = host
                .read_with(v, |root, cx| {
                    let ws = root.mission_workspace.read(cx);
                    ws.layout.drawer.open()
                        && ws.layout.drawer.shells() == ["drawer-1", "drawer-concurrent"]
                        && !ws.closing_drawer_shells.contains("drawer-2")
                })
                .unwrap();
            let db_ok = {
                let conn = core.db.get().unwrap();
                let row = runner_daemon::repo::node::get(&conn, &node_id)
                    .unwrap()
                    .unwrap();
                let persisted = MissionLayout::from_node_row(&row).unwrap();
                persisted.drawer.shells() == ["drawer-1", "drawer-concurrent"]
            };
            applied.is_applied() && ui_ok && db_ok
        },
    );
    assert!(visual.update(|window, _| focus_composer.is_focused(window)));
    assert!(!visual.update(|window, _| focus_shell_1.is_focused(window)));

    // Clean up drawer-concurrent, then verify last-shell close also preserves intervening
    // composer focus while clearing drawer_was_focused.
    runner_daemon::ops::session::session_close(&core, "drawer-concurrent").unwrap();
    wait_local_mission(&mut visual, "only drawer-1 remains", |v| {
        host.read_with(v, |root, cx| {
            root.mission_workspace.read(cx).layout.drawer.shells() == ["drawer-1"]
        })
        .unwrap()
    });
    host.update(&mut visual, |root, window, cx| {
        root.mission_workspace.update(cx, |ws, cx| {
            ws.activate_terminal_drawer_shell("drawer-1", window, cx);
        });
    })
    .unwrap();
    wait_local_mission(
        &mut visual,
        "drawer-1 focused before last gated close",
        |v| {
            host.update(v, |root, window, cx| {
                let ws = root.mission_workspace.read(cx);
                focus_shell_1.is_focused(window) && ws.drawer_was_focused
            })
            .unwrap()
        },
    );

    let (gate, handle) = crate::test_support::StagedReplyGate::pair();
    host.update(&mut visual, |root, window, cx| {
        root.mission_workspace
            .update(cx, |ws, _| ws.drawer_close_gates.push_back(gate));
        crate::surfaces::windowing::close_tab(root, window, cx);
    })
    .unwrap();
    wait_local_mission(&mut visual, "drawer-1 close reply entered gate", |_| {
        handle.is_entered()
    });
    visual.update(|window, cx| focus_composer.focus(window, cx));
    wait_local_mission(
        &mut visual,
        "composer focused before last close reply",
        |v| v.update(|window, _| focus_composer.is_focused(window)),
    );
    let applied = handle.release();
    wait_local_mission(
        &mut visual,
        "last shell closed keeping composer focus and clearing drawer_was_focused",
        |v| {
            applied.is_applied()
                && host
                    .read_with(v, |root, cx| {
                        let ws = root.mission_workspace.read(cx);
                        !ws.layout.drawer.open()
                            && ws.layout.drawer.shells().is_empty()
                            && !ws.drawer_was_focused
                            && !ws.closing_drawer_shells.contains("drawer-1")
                    })
                    .unwrap()
        },
    );
    assert!(visual.update(|window, _| focus_composer.is_focused(window)));

    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[test]
fn local_mission_drawer_empty_focused_cmd_w_hides_and_non_drawer_actions_stay_intact() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();

    // 1. Open and focused drawer with active_shell() == None falls back safely to hiding.
    {
        let (_temp, mut cx, host, fake) = suspended_mission_test();
        let core = fake.base.0.clone();
        let node_id = seed_local_mission_drawer_shells(host, &mut cx, &core, &[], true);
        host.update(&mut cx, |root, window, cx| {
            root.open_mission("test-mission".into(), window, cx);
        })
        .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(gpui::size(px(1600.), px(900.)));
        visual.update(|window, _| window.activate_window());

        wait_local_mission(&mut visual, "mission with empty open drawer loaded", |v| {
            host.read_with(v, |root, cx| {
                let ws = root.mission_workspace.read(cx);
                !ws.loading && ws.layout.drawer.open() && ws.layout.drawer.active_shell().is_none()
            })
            .unwrap()
                && v.debug_bounds("MISSION_TERMINAL_DRAWER").is_some()
        });

        let focus_root = host
            .update(&mut visual, |root, window, cx| {
                root.mission_workspace.update(cx, |ws, cx| {
                    ws.select_mission_feed(window, cx);
                    ws.drawer_focus.focus(window, cx);
                    ws.root_focus.clone()
                })
            })
            .unwrap();
        wait_local_mission(&mut visual, "empty drawer focused", |v| {
            host.update(v, |root, window, cx| {
                let ws = root.mission_workspace.read(cx);
                ws.drawer_focused(window, cx) && ws.drawer_was_focused
            })
            .unwrap()
        });

        host.update(&mut visual, crate::surfaces::windowing::close_tab)
            .unwrap();
        wait_local_mission(&mut visual, "empty drawer hidden on Cmd+W", |v| {
            let state_ok = host
                .read_with(v, |root, cx| {
                    let ws = root.mission_workspace.read(cx);
                    !ws.layout.drawer.open() && !ws.drawer_was_focused
                })
                .unwrap();
            let focus_ok = v.update(|window, _| focus_root.is_focused(window));
            let db_ok = {
                let conn = core.db.get().unwrap();
                let row = runner_daemon::repo::node::get(&conn, &node_id)
                    .unwrap()
                    .unwrap();
                !MissionLayout::from_node_row(&row).unwrap().drawer.open()
            };
            state_ok && focus_ok && db_ok
        });

        host.update(&mut visual, |_, window, _| window.remove_window())
            .unwrap();
    }

    // 2. Non-drawer Cmd+W, drawer hide button, and toggle leave shells intact.
    {
        let (_temp, mut cx, host, fake) = suspended_mission_test();
        let core = fake.base.0.clone();
        seed_local_mission_drawer_shells(host, &mut cx, &core, &["drawer-1", "drawer-2"], true);
        host.update(&mut cx, |root, window, cx| {
            root.open_mission("test-mission".into(), window, cx);
        })
        .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(gpui::size(px(1600.), px(900.)));

        wait_local_mission(&mut visual, "mission with two drawer shells loaded", |v| {
            host.read_with(v, |root, cx| {
                let ws = root.mission_workspace.read(cx);
                !ws.loading
                    && ws.layout.drawer.open()
                    && ws.layout.drawer.shells() == ["drawer-1", "drawer-2"]
                    && ws.attached.contains_key("mission-agent")
            })
            .unwrap()
        });

        // Focus outside drawer (Feed and Session tab): Cmd+W leaves drawer open and shells alive.
        for on_session_tab in [false, true] {
            host.update(&mut visual, |root, window, cx| {
                root.mission_workspace.update(cx, |ws, cx| {
                    if on_session_tab {
                        ws.select_mission_session("mission-agent", window, cx);
                    } else {
                        ws.select_mission_feed(window, cx);
                    }
                });
            })
            .unwrap();
            visual.run_until_parked();
            host.update(&mut visual, crate::surfaces::windowing::close_tab)
                .unwrap();
            visual.run_until_parked();
            host.read_with(&visual, |root, cx| {
                let ws = root.mission_workspace.read(cx);
                assert!(ws.layout.drawer.open());
                assert_eq!(ws.layout.drawer.shells(), ["drawer-1", "drawer-2"]);
            })
            .unwrap();
        }

        // Hide button and Opt+F12 toggle hide without closing shells, and reopen with both intact.
        host.update(&mut visual, |root, window, cx| {
            root.mission_workspace
                .update(cx, |ws, cx| ws.hide_terminal_drawer(window, cx));
        })
        .unwrap();
        visual.run_until_parked();
        host.read_with(&visual, |root, cx| {
            let ws = root.mission_workspace.read(cx);
            assert!(!ws.layout.drawer.open());
            assert_eq!(ws.layout.drawer.shells(), ["drawer-1", "drawer-2"]);
            assert!(!ws.drawer_was_focused);
        })
        .unwrap();
        visual.dispatch_action(crate::ToggleTerminalDrawer);
        visual.run_until_parked();
        host.read_with(&visual, |root, cx| {
            let ws = root.mission_workspace.read(cx);
            assert!(ws.layout.drawer.open());
            assert_eq!(ws.layout.drawer.shells(), ["drawer-1", "drawer-2"]);
        })
        .unwrap();
        let conn = core.db.get().unwrap();
        assert!(runner_daemon::repo::session::get_row(&conn, "drawer-1")
            .unwrap()
            .is_some());
        assert!(runner_daemon::repo::session::get_row(&conn, "drawer-2")
            .unwrap()
            .is_some());

        host.update(&mut visual, |_, window, _| window.remove_window())
            .unwrap();
    }
}

#[test]
fn mission_card_runtime_line_formats_values_and_omits_unknown_and_shell() {
    let member = |runtime: &str, model: Option<&str>, effort: Option<&str>| {
        crate::surfaces::front_page::model::MissionMemberDisplay {
            session: crate::surfaces::front_page::model::MissionMemberSession {
                id: "s".into(),
                status: SessionStatus::Running,
                cwd: None,
            },
            handle: "coder".into(),
            runtime: runtime.into(),
            agent_model: model.map(str::to_owned),
            agent_effort: effort.map(str::to_owned),
            live_title: None,
            agent_session_key: None,
        }
    };
    assert_eq!(
        rail::session_runtime_line(&member("codex", Some("gpt-6-astra"), Some("xhigh"))).as_deref(),
        Some("Codex · gpt-6-astra · Extra high")
    );
    crate::i18n::testing::with_locale(crate::i18n::Locale::SimplifiedChinese, || {
        assert_eq!(
            rail::session_runtime_line(&member("codex", Some("gpt-6-astra"), Some("xhigh")))
                .as_deref(),
            Some("Codex · gpt-6-astra · 极高")
        );
    });
    assert_eq!(
        rail::session_runtime_line(&member("claude-code", Some("opus"), Some("high"))).as_deref(),
        Some("Claude Code · opus · High")
    );
    assert_eq!(
        rail::session_runtime_line(&member("pi", Some("deepseek-chat"), None)).as_deref(),
        Some("pi · deepseek-chat")
    );
    assert_eq!(
        rail::session_runtime_line(&member("codex", None, None)).as_deref(),
        Some("Codex")
    );
    assert_eq!(
        rail::session_runtime_line(&member("codex", None, Some("high"))).as_deref(),
        Some("Codex · High")
    );
    assert_eq!(
        rail::session_runtime_line(&member("codex", Some("  "), Some("default"))).as_deref(),
        Some("Codex")
    );
    assert_eq!(
        rail::session_runtime_line(&member("claude-code", Some("default"), Some("high")))
            .as_deref(),
        Some("Claude Code · High")
    );
    assert_eq!(
        rail::session_runtime_line(&member("codex", Some("gpt-6-astra"), Some("DEFAULT")))
            .as_deref(),
        Some("Codex · gpt-6-astra")
    );
    assert_eq!(
        rail::session_runtime_line(&member("codex", Some("gpt-6-astra"), Some(""))).as_deref(),
        Some("Codex · gpt-6-astra")
    );
    assert_eq!(
        rail::session_runtime_line(&member("future-agent", Some("v1"), Some("high"))).as_deref(),
        Some("future-agent · v1 · High")
    );
    assert_eq!(
        rail::session_runtime_line(&member("shell", Some("ignored"), Some("high"))),
        None
    );
    assert_eq!(rail::session_runtime_line(&member("", None, None)), None);
}

#[cfg(unix)]
#[test]
fn mission_rail_cards_show_runtime_line_omit_shell_and_fit_two_up() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{TestAppContext, VisualTestContext};
    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store)
    });
    let mission = store.read_with(&cx, |store, _| {
        runner_daemon::ops::mission::mission_get(&store.test_core, "test-mission").unwrap()
    });
    let host = cx.add_window(|window, cx| {
        window.set_rem_size(px(16.));
        let workspace = cx.new(|cx| {
            let mut workspace = MissionWorkspace::new(
                "rail-runtime".into(),
                WeakEntity::new_invalid(),
                store,
                window,
                cx,
            );
            workspace.mission = Some(mission);
            let wrapped_key = "01a119c9-4714-7091-8b75-81cec4ff360c";
            workspace.sessions = [
                (
                    "coder",
                    "claude-code",
                    Some("opus"),
                    Some("high"),
                    Some("01kzmr6z"),
                ),
                ("qa", "pi", Some("deepseek-chat"), None, None),
                ("bare", "codex", None, None, None),
                ("shell", "shell", None, None, None),
                (
                    "reviewer",
                    "codex",
                    Some("a-very-long-model-name-that-must-truncate"),
                    Some("xhigh"),
                    Some(wrapped_key),
                ),
            ]
            .into_iter()
            .map(|(id, runtime, model, effort, key)| SessionRow {
                session: runner_core::protocol::model::Session {
                    id: id.into(),
                    mission_id: Some("test-mission".into()),
                    role_id: "test-shell-role".into(),
                    slot_id: None,
                    cwd: None,
                    status: SessionStatus::Running,
                    pid: None,
                    started_at: None,
                    stopped_at: None,
                },
                handle: id.into(),
                role_handle: id.into(),
                lead: false,
                runtime: runtime.into(),
                agent_model: model.map(str::to_owned),
                agent_effort: effort.map(str::to_owned),
                live_title: None,
                agent_session_key: key.map(str::to_owned),
            })
            .collect();
            workspace.sync_mission_copy_entities(cx);
            workspace
        });
        MissionRailLayoutTest {
            workspace,
            width: None,
        }
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(2400.), px(900.)));
    for width in [app_settings::MISSION_RAIL_DEFAULT, 560., 680.] {
        host.update(&mut visual, |host, _, cx| {
            host.width = Some(width);
            cx.notify();
        })
        .unwrap();
        visual.run_until_parked();
        let full_runtime = visual.debug_bounds("RAIL_FULL_RUNTIME").unwrap();
        for id in ["coder", "qa", "bare", "reviewer"] {
            let card = visual
                .debug_bounds(format!("MISSION_CARD {id}").leak())
                .unwrap();
            let avatar = visual
                .debug_bounds(format!("MISSION_CARD_AVATAR {id}").leak())
                .unwrap();
            let handle = visual
                .debug_bounds(format!("MISSION_CARD_HANDLE {id}").leak())
                .unwrap();
            let status = visual
                .debug_bounds(format!("MISSION_CARD_STATUS {id}").leak())
                .unwrap();
            let runtime = visual
                .debug_bounds(format!("MISSION_CARD_RUNTIME {id}").leak())
                .unwrap();
            let mark = visual
                .debug_bounds(format!("MISSION_CARD_RUNTIME_MARK {id}").leak())
                .unwrap();
            let text = visual
                .debug_bounds(format!("MISSION_CARD_RUNTIME_TEXT {id}").leak())
                .unwrap();
            let key = visual
                .debug_bounds(format!("MISSION_CARD_KEY {id}").leak())
                .unwrap();
            assert!(status.left() >= avatar.right(), "{id} at {width}");
            assert!(status.top() >= handle.bottom(), "{id} at {width}");
            assert!(runtime.top() >= status.bottom(), "{id} at {width}");
            assert!(runtime.bottom() <= key.top(), "{id} at {width}");
            assert!(runtime.right() <= card.right(), "{id} at {width}");
            assert_eq!(mark.size, size(px(12.), px(12.)), "{id} at {width}");
            assert_eq!(
                text.size.height, full_runtime.size.height,
                "{id} at {width}"
            );
            assert!(text.right() <= card.right(), "{id} at {width}");
            if id == "reviewer" {
                assert!(
                    card.size.height > px(108.),
                    "wrapped session_key card should grow beyond 108 px: {id} at {width}: {card:?}"
                );
            } else {
                assert_eq!(
                    card.size.height,
                    px(108.),
                    "single-line key card (populated or NULL) should match the spec's 108 px height: {id} at {width}: {card:?}"
                );
            }
        }
        for (id, expected_key) in [
            ("coder", "01kzmr6z"),
            ("reviewer", "01a119c9-4714-7091-8b75-81cec4ff360c"),
        ] {
            let card = visual
                .debug_bounds(format!("MISSION_CARD {id}").leak())
                .unwrap();
            let key = visual
                .debug_bounds(format!("MISSION_CARD_KEY {id}").leak())
                .unwrap();
            let copy = visual
                .debug_bounds(format!("MISSION_CARD_KEY_COPY {id}").leak())
                .unwrap();
            assert_eq!(copy.size, size(px(20.), px(20.)), "{id} at {width}");
            assert!(key.right() <= copy.left(), "{id} at {width}");
            assert!(copy.right() <= card.right(), "{id} at {width}");
            visual.simulate_click(copy.center(), gpui::Modifiers::default());
            visual.run_until_parked();
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some(expected_key.into()),
                "copy hit target failed for {id} at {width}"
            );
        }
        let reviewer_text = visual
            .debug_bounds("MISSION_CARD_RUNTIME_TEXT reviewer")
            .unwrap();
        assert!(
            reviewer_text.size.width < full_runtime.size.width,
            "reviewer long runtime line should truncate at {width}: {reviewer_text:?}"
        );
        assert!(visual.debug_bounds("MISSION_CARD shell").is_some());
        assert!(visual.debug_bounds("MISSION_CARD_STATUS shell").is_none());
        assert!(visual.debug_bounds("MISSION_CARD_RUNTIME shell").is_none());
        assert!(visual
            .debug_bounds("MISSION_CARD_RUNTIME_MARK shell")
            .is_none());
    }
    host.update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}
