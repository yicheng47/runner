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
fn same_mission_notification_resumes_a_suspended_local_mission() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, host, fake) = suspended_mission_test();
    host.update(&mut cx, |root, window, cx| {
        root.open_mission("test-mission".into(), window, cx)
    })
    .unwrap();
    cx.run_until_parked();
    host.update(&mut cx, |root, window, cx| {
        root.mission_workspace.update(cx, |workspace, _| {
            workspace.active_tab = MissionTab::Session("mission-shell".into());
        });
        root.visit_computer(other_computer(), window, cx)
    })
    .unwrap();
    cx.run_until_parked();
    fake.viewed.lock().unwrap().clear();
    cx.update(|cx| crate::notifications::open("mission-agent", cx));
    cx.run_until_parked();
    host.read_with(&cx, |root, cx| {
        assert!(root.remote_client.is_local());
        assert_eq!(root.route, AppRoute::Mission("test-mission".into()));
        let workspace = root.mission_workspace.read(cx);
        assert!(workspace.active);
        assert_eq!(
            workspace.active_tab,
            MissionTab::Session("mission-agent".into())
        );
        assert!(workspace.attached.contains_key("mission-agent"));
        assert!(workspace.mission_terminal_interactive("mission-agent", cx));
    })
    .unwrap();
    assert!(fake
        .viewed
        .lock()
        .unwrap()
        .iter()
        .any(|id| id == "mission-agent"));
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
                dark_terminal_theme: app_settings::DarkTerminalTheme::RosePineDawn,
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
    assert_eq!(theme::active_variant(), theme::ThemeVariant::Carbon);
    assert_eq!(
        bridge.session("direct").unwrap().palette(),
        runner_terminal::palette::ROSE_PINE_DAWN
    );
    assert_eq!(
        bridge.session("slot").unwrap().palette(),
        runner_terminal::palette::ROSE_PINE_DAWN
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
                    settings.dark_terminal_theme = app_settings::DarkTerminalTheme::Runner;
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
    let visual = cx.add_empty_window();
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
    use gpui::{ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext};
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
    for (view, viewport_selector, final_selector) in [
        (
            PanelViewId::Sessions,
            "MISSION_ROLES_SCROLL",
            "MISSION_CARD slot-19",
        ),
        (PanelViewId::Info, "MISSION_META_SCROLL", "MISSION_META_END"),
    ] {
        host.update(&mut visual, |host, _, cx| {
            host.workspace
                .update(cx, |workspace, _| workspace.rail_surfaces.open.open(view));
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
        visual.simulate_event(ScrollWheelEvent {
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
    use crate::surfaces::side_panel::{self, PanelSurface};

    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let (store, host) = rail_header_window(&mut cx, &temp);
    let workspace = host
        .read_with(&cx, |host, _| host.workspace.clone())
        .unwrap();
    workspace.update(&mut cx, |workspace, cx| {
        workspace.prepare_mission("first".into());
        workspace.rail_surfaces.open.open(PanelViewId::Info);
        workspace.rail_surfaces.open.open(PanelViewId::Files);
        workspace.set_rail_open(true, cx);
        workspace.prepare_mission("second".into());
        assert_eq!(workspace.rail_surfaces.open, Default::default());
        assert!(
            workspace.rail_is_open(cx),
            "a new mission follows the last choice"
        );
        workspace.rail_surfaces.open.open(PanelViewId::Info);
        workspace.set_rail_open(false, cx);
        workspace.prepare_mission("first".into());
        assert!(workspace.rail_is_open(cx));
        assert_eq!(
            side_panel::shown(
                PanelSurface::Mission,
                workspace.settings(cx),
                &workspace.rail_surfaces.open
            ),
            Some(PanelViewId::Files)
        );
        workspace.prepare_mission("second".into());
        assert!(!workspace.rail_is_open(cx));
        assert_eq!(
            side_panel::shown(
                PanelSurface::Mission,
                workspace.settings(cx),
                &workspace.rail_surfaces.open
            ),
            Some(PanelViewId::Info)
        );
        workspace.prepare_mission("new".into());
        assert!(!workspace.rail_is_open(cx));
        assert_eq!(workspace.rail_surfaces.open, Default::default());
        workspace.prepare_mission("second".into());
        workspace.rail_surfaces.open.open(PanelViewId::Files);
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
        assert_eq!(workspace.rail_surfaces.open.ids().0, &[PanelViewId::Info]);
        workspace.prepare_mission("first".into());
        assert_eq!(workspace.rail_surfaces.open.ids().0, &[PanelViewId::Info]);
        assert!(
            workspace.rail_is_open(cx),
            "Settings preserves each mission's collapse choice"
        );
    });
}

#[test]
fn mission_rail_toggles_notify_even_when_the_default_already_matches() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let (store, host) = rail_header_window(&mut cx, &temp);
    let workspace = host
        .read_with(&cx, |host, _| host.workspace.clone())
        .unwrap();
    workspace.update(&mut cx, |workspace, cx| {
        workspace.prepare_mission("open".into());
        workspace.set_rail_open(true, cx);
        workspace.prepare_mission("collapsed".into());
        workspace.set_rail_open(false, cx);
        workspace.prepare_mission("open".into());
        assert!(!workspace.settings(cx).mission_rail_open);
        let before = store.read(cx).revisions.settings;
        workspace.set_rail_open(false, cx);
        let collapsed = store.read(cx).revisions.settings;
        assert_ne!(
            before, collapsed,
            "the grid hint observer needs the toggle notification"
        );
        workspace.set_rail_open(false, cx);
        assert_eq!(
            store.read(cx).revisions.settings,
            collapsed,
            "a no-op skips the save"
        );

        workspace.prepare_mission("other".into());
        workspace.set_rail_open(true, cx);
        workspace.prepare_mission("collapsed".into());
        assert!(workspace.settings(cx).mission_rail_open);
        assert!(!workspace.rail_is_open(cx));
        let before = store.read(cx).revisions.settings;
        workspace.set_rail_open(true, cx);
        let opened = store.read(cx).revisions.settings;
        assert_ne!(
            before, opened,
            "opening a collapsed rail needs the same notification"
        );
        workspace.set_rail_open(true, cx);
        assert_eq!(
            store.read(cx).revisions.settings,
            opened,
            "a repeated open skips the save"
        );
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
            assert!(workspace.settings(cx).mission_rail_open);
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
            assert!(!workspace.settings(cx).mission_rail_open);
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
fn mission_panel_header_keeps_sessions_first_and_adds_info_and_files_from_plus() {
    use crate::surfaces::side_panel::{PanelContext, PanelSubject};
    use crate::theme_snapshot::{assert_fill_color, assert_text, ThemeGuard};
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
    // Sessions alone, permanent: no × and not in `+`.
    let rail = visual.debug_bounds("MISSION_RAIL").unwrap();
    let sessions = visual.debug_bounds("PANEL_TAB Sessions").unwrap();
    let add = visual.debug_bounds("PANEL_ADD_SURFACE").unwrap();
    assert!(
        sessions.left() - rail.left() <= px(12.),
        "{sessions:?} {rail:?}"
    );
    assert_eq!(sessions.size.height, px(28.));
    assert!(sessions.right() < add.left());
    assert_eq!(add.size, gpui::size(px(24.), px(24.)));
    assert_eq!(sessions.center().y, add.center().y);
    assert!(visual.debug_bounds("PANEL_TAB_CLOSE Sessions").is_none());
    assert!(visual.debug_bounds("PANEL_TAB Info").is_none());
    assert!(visual.debug_bounds("MISSION_ROLES_SCROLL").is_some());
    for variant in [
        theme::ThemeVariant::Carbon,
        theme::ThemeVariant::RunnerLight,
    ] {
        theme::set_active_variant(variant);
        visual.refresh().unwrap();
        visual.run_until_parked();
        assert_fill_color(&mut visual, "PANEL_TAB Sessions", Some(theme::raised()));
        assert_text(&mut visual, "PANEL_TAB_INK Sessions", theme::text());
    }

    // `+` offers Info and Files, never Sessions, with nothing checked.
    click(&mut visual, "PANEL_ADD_SURFACE");
    assert!(visual.debug_bounds("PANEL_MENU_ROW Info").is_some());
    assert!(visual.debug_bounds("PANEL_MENU_ROW Files").is_some());
    assert!(visual.debug_bounds("PANEL_MENU_ROW Sessions").is_none());
    assert!(visual.debug_bounds("PANEL_MENU_CHECK Info").is_none());
    click(&mut visual, "PANEL_MENU_ROW Files");
    assert!(
        visual.debug_bounds("PANEL_MENU_ROW Files").is_none(),
        "closed"
    );
    let context = PanelContext {
        subject: PanelSubject::Mission("test-mission".into()),
        surface: crate::surfaces::side_panel::PanelSurface::Mission,
        cwd: Some("/tmp/panel-mission".into()),
        project: None,
        remote: false,
    };
    assert!(visual
        .debug_bounds(format!("FILES_VIEW {context:?}").leak())
        .is_some());
    assert!(visual.debug_bounds("MISSION_ROLES_SCROLL").is_none());
    let sessions = visual.debug_bounds("PANEL_TAB Sessions").unwrap();
    let files = visual.debug_bounds("PANEL_TAB Files").unwrap();
    assert!(sessions.right() < files.left());
    assert!(visual.debug_bounds("PANEL_TAB_CLOSE Files").is_some());
    for variant in [
        theme::ThemeVariant::Carbon,
        theme::ThemeVariant::RunnerLight,
    ] {
        theme::set_active_variant(variant);
        visual.refresh().unwrap();
        visual.run_until_parked();
        assert_fill_color(&mut visual, "PANEL_TAB Sessions", None);
        assert_fill_color(&mut visual, "PANEL_TAB Files", Some(theme::raised()));
        assert_text(&mut visual, "PANEL_TAB_INK Sessions", theme::muted());
    }

    // Info opens after Files; Sessions switches back; closing returns to
    // the tab before, and the first closable one to Sessions.
    click(&mut visual, "PANEL_ADD_SURFACE");
    assert!(visual.debug_bounds("PANEL_MENU_CHECK Files").is_some());
    click(&mut visual, "PANEL_MENU_ROW Info");
    assert!(visual.debug_bounds("MISSION_META_SCROLL").is_some());
    let files = visual.debug_bounds("PANEL_TAB Files").unwrap();
    let info = visual.debug_bounds("PANEL_TAB Info").unwrap();
    assert!(
        files.right() < info.left(),
        "tabs keep the order they opened"
    );
    click(&mut visual, "PANEL_TAB Sessions");
    assert!(visual.debug_bounds("MISSION_ROLES_SCROLL").is_some());
    click(&mut visual, "PANEL_TAB Info");
    click(&mut visual, "PANEL_TAB_CLOSE Info");
    assert!(visual.debug_bounds("PANEL_TAB Info").is_none());
    assert!(visual
        .debug_bounds(format!("FILES_VIEW {context:?}").leak())
        .is_some());
    click(&mut visual, "PANEL_TAB_CLOSE Files");
    assert!(visual.debug_bounds("MISSION_ROLES_SCROLL").is_some());
    assert!(visual.debug_bounds("PANEL_EMPTY").is_none(), "never empty");

    // Settings → Side panel turning Info and Files off leaves Sessions,
    // with no `+`, and closes an open Files tab.
    click(&mut visual, "PANEL_ADD_SURFACE");
    click(&mut visual, "PANEL_MENU_ROW Files");
    store.update(&mut visual, |store, cx| {
        store.update_settings(
            |settings| {
                settings.info_view = false;
                settings.files_view = false;
                true
            },
            false,
            cx,
        );
    });
    visual.refresh().unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("PANEL_TAB Sessions").is_some());
    assert!(visual.debug_bounds("PANEL_TAB Files").is_none());
    assert!(visual.debug_bounds("PANEL_ADD_SURFACE").is_none());
    store.update(&mut visual, |store, cx| {
        store.update_settings(
            |settings| {
                settings.info_view = true;
                settings.files_view = true;
                true
            },
            false,
            cx,
        );
    });
    visual.refresh().unwrap();
    visual.run_until_parked();
    assert!(
        visual.debug_bounds("PANEL_TAB Files").is_none(),
        "turning Files back on does not reopen its tab"
    );
    assert!(visual.debug_bounds("MISSION_ROLES_SCROLL").is_some());
}

#[test]
fn mission_panel_header_keeps_plus_and_collapse_in_a_narrow_rail_in_both_locales() {
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
                    workspace.rail_surfaces.open.open(PanelViewId::Files);
                    workspace.rail_surfaces.open.open(PanelViewId::Info);
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
                let close = visual.debug_bounds("PANEL_TAB_CLOSE Info").unwrap();
                let add = visual.debug_bounds("PANEL_ADD_SURFACE").unwrap();
                let collapse = visual.debug_bounds("PANEL_COLLAPSE").unwrap();
                assert!(
                    close.right() <= add.left(),
                    "{locale:?} {width}: the shown tab's × before `+`: {close:?} {add:?}"
                );
                assert!(add.right() <= collapse.left(), "{locale:?} {width}");
                assert!(
                    collapse.right() <= rail.right(),
                    "{locale:?} {width}: {collapse:?} {rail:?}"
                );
                assert_eq!(add.size.width, px(24.), "{locale:?} {width}");
                // `+` still opens its menu, and Sessions still switches.
                visual.simulate_click(add.center(), Modifiers::default());
                visual.run_until_parked();
                assert!(visual.debug_bounds("PANEL_MENU_ROW Files").is_some());
                visual.simulate_keystrokes("escape");
                let sessions = visual.debug_bounds("PANEL_TAB Sessions").unwrap();
                assert!(sessions.left() >= rail.left(), "{locale:?} {width}");
                visual.simulate_click(sessions.center(), Modifiers::default());
                visual.run_until_parked();
                assert!(visual.debug_bounds("MISSION_ROLES_SCROLL").is_some());
                let info = visual.debug_bounds("PANEL_TAB Info").unwrap();
                visual.simulate_click(info.center(), Modifiers::default());
                visual.run_until_parked();
                assert!(visual.debug_bounds("MISSION_META_SCROLL").is_some());
            }
        });
    }
}

impl gpui::Render for MissionRailLayoutTest {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rail = self.workspace.update(cx, |workspace, cx| {
            let layout = match self.width {
                Some(width) => (width, None),
                None => workspace.rail_layout(cx),
            };
            workspace.render_mission_rail(layout, 1., true, window, cx)
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
        for (card_selector, handle_selector, controls_selector) in [
            (
                "MISSION_CARD lead",
                "MISSION_CARD_HANDLE lead",
                "MISSION_CARD_CONTROLS lead",
            ),
            (
                "MISSION_CARD worker",
                "MISSION_CARD_HANDLE worker",
                "MISSION_CARD_CONTROLS worker",
            ),
        ] {
            let card = visual.debug_bounds(card_selector).unwrap();
            let handle = visual.debug_bounds(handle_selector).unwrap();
            let controls = visual.debug_bounds(controls_selector).unwrap();
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
                controls.right() <= card.right(),
                "controls overflow at rail width {width}"
            );
            assert_eq!(
                controls.size.width,
                px(80.),
                "controls shrank at rail width {width}"
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
        host.update(&mut visual, |host, _, cx| {
            host.workspace.update(cx, |workspace, cx| {
                let mission_id = "01M4CWJGAR56HCBK0RNRQS8DV0";
                workspace.mission.as_mut().unwrap().id = mission_id.into();
                workspace.mission_id_copy.update(cx, |copy, cx| {
                    copy.set_value(Some(mission_id.into()), cx);
                });
                workspace.rail_surfaces.open.open(PanelViewId::Info);
            });
            cx.notify();
        })
        .unwrap();
        cx.run_until_parked();
        assert!(
            visual.debug_bounds("MISSION_META_ID").unwrap().size.height
                <= visual
                    .debug_bounds("RAIL_TWO_LINE_MISSION_ID")
                    .unwrap()
                    .size
                    .height,
            "mission ID exceeds two lines at rail width {width}"
        );
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
    use crate::surfaces::mission_markdown::PARSE_COUNT;
    use gpui::{ListOffset, VisualTestContext};
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
    workspace.read_with(&visual, |this, _| {
        assert_feed_projection(this);
        assert!(this.feed_list.is_following_tail());
        assert!(this.feed_list.bounds_for_item(1999).is_some());
        assert!(this.feed_block_builds.get() > 0);
        assert!(this.feed_block_builds.get() < 100);
    });
    PARSE_COUNT.set(0);
    workspace.update(&mut visual, |this, cx| {
        this.feed_block_builds.set(0);
        cx.notify();
    });
    visual.run_until_parked();
    assert_eq!(PARSE_COUNT.get(), 0);
    visual.simulate_input("typing in a long feed");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.feed_block_builds.get() < 100);
        assert_eq!(this.composer.draft, "typing in a long feed");
    });
    assert_eq!(PARSE_COUNT.get(), 0);
    workspace.update(&mut visual, |this, cx| {
        let mut event = this.events.last().unwrap().clone();
        event.id = "2000".into();
        this.append_event(event);
        cx.notify();
    });
    visual.run_until_parked();
    assert!(PARSE_COUNT.get() <= 1);
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
            anchor: (0, 0),
            head: (0, 3),
        });
        cx.notify();
    });
    visual.run_until_parked();
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
            item_ix: this.feed_blocks.len() - 1,
            offset_in_item: px(0.),
        });
        cx.notify();
    });
    visual.run_until_parked();
    let viewport = workspace.read_with(&visual, |this, _| this.feed_list.viewport_bounds());
    for _ in 0..8 {
        visual.simulate_event(gpui::ScrollWheelEvent {
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
        visual.simulate_event(gpui::ScrollWheelEvent {
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
        assert!(this
            .feed_markdown
            .borrow()
            .contains_key(&this.events[top].id));
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
    use crate::surfaces::mission_markdown::{PARSE_COUNT, RENDER_COUNT};
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
        assert_eq!(this.feed_rows.len(), 2000);
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
        assert_eq!(this.feed_rows.len(), 2001);
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
    use crate::surfaces::mission_markdown::{PARSE_COUNT, RENDER_COUNT};
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
            anchor: (1996, 0),
            head: (1997, 15),
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
    use crate::surfaces::mission_markdown::{LINE_COUNT, PARSE_COUNT};
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
                anchor: if table { (4000, 0) } else { (1998, 0) },
                head: if table { (4001, 5) } else { (1999, 10) },
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
        visual.dispatch_action(Copy);
        assert_eq!(
            visual
                .read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some(if table {
                "L1999\nR1999"
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
            let last = visual
                .debug_bounds("MISSION_TABLE_ROW 00000000000000000000000000 0 2000")
                .unwrap();
            visual.simulate_event(gpui::ScrollWheelEvent {
                position: point(last.left() + px(20.), last.center().y),
                delta: gpui::ScrollDelta::Pixels(point(px(-60.), px(0.))),
                ..Default::default()
            });
            visual.run_until_parked();
            let next = visual
                .debug_bounds("MISSION_TABLE_ROW 00000000000000000000000000 0 2000")
                .unwrap();
            let previous = visual
                .debug_bounds("MISSION_TABLE_ROW 00000000000000000000000000 0 1999")
                .unwrap();
            assert!(
                next.left() < last.left(),
                "table gutter wheel did not scroll horizontally"
            );
            assert_eq!(next.left(), previous.left());
            workspace.update(&mut visual, |this, cx| {
                this.feed_list.scroll_to(ListOffset::default());
                cx.notify();
            });
            visual.run_until_parked();
            workspace.update(&mut visual, |this, cx| {
                this.scroll_feed_to_bottom();
                cx.notify();
            });
            visual.run_until_parked();
            let restored = visual
                .debug_bounds("MISSION_TABLE_ROW 00000000000000000000000000 0 2000")
                .unwrap();
            assert_eq!(restored.left(), next.left());
            host.update(&mut visual, |host, _, cx| {
                host.width = 800.;
                cx.notify();
            })
            .unwrap();
            visual.run_until_parked();
            let widened = visual
                .debug_bounds("MISSION_TABLE_ROW 00000000000000000000000000 0 2000")
                .unwrap();
            assert_eq!(widened.left(), last.left());
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
        assert!(
            (800..1200).contains(&top.item_ix),
            "track click reached {:?}",
            top
        );
        assert!(!this.feed_list.is_following_tail());
        assert!(!this.feed_list.is_scrollbar_dragging());
        top
    });
    visual.simulate_event(gpui::ScrollWheelEvent {
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
            anchor: (0, 0),
            head: (0, 3),
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
        assert!(this.feed_block_builds.get() < 200);
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
    let visual = cx.add_empty_window();
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
    store.update(&mut cx, |store, cx| {
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
        store.update_settings(
            |settings| {
                settings.mission_rail_open = false;
                true
            },
            false,
            cx,
        );
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
                .and_then(|view| view.read(cx).active_tab())
                == Some("lib.rs".into())
        })
    });
    assert_eq!(launched(), 0);
    assert!(workspace.read_with(&visual, |workspace, cx| workspace.rail_is_open(cx)));
    assert_eq!(
        workspace.read_with(&visual, |workspace, _| workspace.rail_surfaces.open.clone()),
        {
            let mut files = crate::surfaces::side_panel::OpenSurfaces::default();
            files.open(PanelViewId::Files);
            files
        },
        "the closed Files tab opens"
    );

    // A file left open keeps the rail wide behind Sessions; closing Files, or
    // re-entering the mission, gives the width back and keeps the file's tab.
    let widened = |visual: &mut gpui::VisualTestContext| {
        workspace.read_with(visual, |workspace, cx| {
            workspace.rail_layout(cx).1.is_some()
        })
    };
    assert!(widened(&mut visual));
    workspace.update(&mut visual, |workspace, _| {
        workspace.rail_surfaces.open.select(PanelViewId::Sessions)
    });
    assert!(widened(&mut visual), "another surface showing");
    workspace.update(&mut visual, |workspace, _| {
        workspace.rail_surfaces.open.close(PanelViewId::Files, true)
    });
    assert!(!widened(&mut visual), "Files closed");
    workspace.update(&mut visual, |workspace, _| {
        workspace.rail_surfaces.open.open(PanelViewId::Files);
        workspace.rail_surfaces.reset(Default::default());
    });
    assert!(!widened(&mut visual), "the surface list reset");
    assert!(workspace.read_with(&visual, |workspace, cx| {
        workspace
            .rail_files
            .view(&subject)
            .and_then(|view| view.read(cx).active_tab())
            == Some("lib.rs".into())
    }));
    workspace.update(&mut visual, |workspace, _| {
        workspace.rail_surfaces.open.open(PanelViewId::Files)
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
