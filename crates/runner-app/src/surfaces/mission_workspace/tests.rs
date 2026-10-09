use std::sync::Arc;

use chrono::Utc;
use gpui::prelude::*;
use gpui::px;
use runner_app::ui::SessionControlKind;
use runner_daemon::model::{Event, EventKind, MissionStatus, SessionStatus};

use super::*;
use crate::surfaces::*;
use crate::*;

struct MissionComposerInputTest {
    workspace: Entity<MissionWorkspace>,
}

impl gpui::Render for MissionComposerInputTest {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(600.))
            .child(self.workspace.update(cx, |workspace, cx| {
                workspace.render_mission_composer(window, cx)
            }))
    }
}

#[test]
fn composer_tracks_keyboard_and_mouse_caret_moves_and_completes_without_resetting_input() {
    use gpui::{EntityInputHandler, MouseButton, TestAppContext, VisualTestContext, WeakEntity};
    use runner_daemon::{db, session, shell_path};
    use std::sync::RwLock;

    let temp = tempfile::tempdir().unwrap();
    let env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
    let discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
    let core = crate::test_support::core(
        Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap()),
        temp.path().to_owned(),
        session::SessionManager::new(
            env.clone(),
            discovery.clone(),
            Arc::new(session::pty_runtime::PtyRuntime::new()),
        ),
        env,
        discovery,
    );
    let mut cx = TestAppContext::single();
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
                WeakEntity::new_invalid(),
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
        MissionComposerInputTest { workspace }
    });
    let workspace = host
        .read_with(&cx, |host, _| host.workspace.clone())
        .unwrap();
    let input = workspace.read_with(&cx, |workspace, _| workspace.composer_input.clone());
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_input("please check @rev");
    let query = |visual: &VisualTestContext| {
        workspace.read_with(visual, |workspace, _| {
            crate::surfaces::mission_composer::mention_query(&workspace.composer).map(str::to_owned)
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
    assert_eq!(
        workspace
            .read_with(&visual, |workspace, _| workspace.composer.target.clone())
            .as_deref(),
        Some("reviewer")
    );
    visual.simulate_keystrokes(if cfg!(windows) { "ctrl-z" } else { "cmd-z" });
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
}

fn signal(signal_type: &str, payload: serde_json::Value) -> Event {
    Event {
        id: signal_type.into(),
        ts: Utc::now(),
        crew_id: "crew".into(),
        mission_id: "mission".into(),
        kind: EventKind::Signal,
        from: "system".into(),
        to: None,
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
    let pool = Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap());
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
        #[cfg(not(windows))]
        let updater = cx.new(|cx| crate::Updater::new(false, cx));
        #[cfg(windows)]
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
            workspace.crew = Some(runner_daemon::ops::crew::crew_get(&core, "crew").unwrap());
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
                drop(workspace.render_mission_terminal_pane(archived_slot.clone(), window, cx));
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
fn cached_mission_header_tabs_and_feed_span_the_content_column() {
    use gpui::{TestAppContext, VisualTestContext};
    use runner_daemon::{db, session, shell_path};
    use std::sync::RwLock;

    let temp = tempfile::tempdir().unwrap();
    let pool = Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap());
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
        #[cfg(not(windows))]
        let updater = cx.new(|cx| crate::Updater::new(false, cx));
        #[cfg(windows)]
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
            workspace.crew = Some(runner_daemon::ops::crew::crew_get(&core, "crew").unwrap());
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
}

struct MissionRailLayoutTest {
    workspace: Entity<MissionWorkspace>,
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
                    lead: index == 0,
                    runtime: "shell".into(),
                    live_title: None,
                    agent_session_key: None,
                })
                .collect();
            workspace
        });
        MissionRailLayoutTest { workspace }
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(960.), px(300.)));
    visual.run_until_parked();
    for (view, viewport_selector, final_selector) in [
        (
            MissionRailView::Roles,
            "MISSION_ROLES_SCROLL",
            "MISSION_CARD slot-19",
        ),
        (
            MissionRailView::Meta,
            "MISSION_META_SCROLL",
            "MISSION_META_END",
        ),
    ] {
        host.update(&mut visual, |host, _, cx| {
            host.workspace
                .update(cx, |workspace, _| workspace.rail_view = view);
            cx.notify();
        })
        .unwrap();
        visual.run_until_parked();
        let rail = visual.debug_bounds("MISSION_RAIL").unwrap();
        let viewport = visual.debug_bounds(viewport_selector).unwrap();
        assert_eq!(viewport.bottom(), rail.bottom() - px(8.));
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

impl gpui::Render for MissionRailLayoutTest {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rail = self.workspace.update(cx, |workspace, cx| {
            workspace.render_mission_rail(1., true, false, window, cx)
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
                        .debug_selector(|| "RAIL_FULL_LEAD".into())
                        .child(runner_app::ui::lead_badge()),
                )
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
        let pool = Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap());
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
                    lead,
                    runtime: "codex".into(),
                    live_title: None,
                    agent_session_key: (id != "short").then(|| session_key.into()),
                })
                .collect();
                workspace.sync_mission_copy_entities(cx);
                assert_eq!(
                    workspace.session_title_tooltip(&workspace.sessions[0], cx),
                    "@abcdefghijklmnopqrstuvwxyz012345"
                );
                workspace
            });
            MissionRailLayoutTest { workspace }
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(960.), px(600.)));
        cx.run_until_parked();
        let full = visual.debug_bounds("RAIL_FULL_HANDLE").unwrap();
        let prefix = visual.debug_bounds("RAIL_VISIBLE_PREFIX").unwrap();
        let badge = visual.debug_bounds("MISSION_CARD_LEAD lead").unwrap();
        let full_badge = visual.debug_bounds("RAIL_FULL_LEAD").unwrap();
        assert_eq!(
            badge.size, full_badge.size,
            "badge shrank at rail width {width}"
        );
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
                px(52.),
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
                assert!(
                    handle.right() <= badge.left(),
                    "handle overlaps LEAD at rail width {width}"
                );
                assert!(
                    badge.right() <= controls.left(),
                    "LEAD overlaps controls at rail width {width}"
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
            let badge = visual.debug_bounds("MISSION_CARD_LEAD lead").unwrap();
            assert_eq!(
                handle.size.width,
                visual.debug_bounds(reference).unwrap().size.width,
                "short lead handle changed at rail width {width}, status {status:?}"
            );
            let expected_gap = if status == MissionStatus::Running && width < 220. {
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
                badge.left() - handle.right(),
                px(expected_gap),
                "short lead badge gap changed at rail width {width}, status {status:?}"
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
                workspace.rail_view = MissionRailView::Meta;
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
#[cfg(not(windows))]
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
    assert_eq!(STOP_ALL_BODY, "Every slot's PTY is killed and whatever turn it is on is cut off. The mission stays open; each slot can be resumed with its conversation, or restarted with its brief.");
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
