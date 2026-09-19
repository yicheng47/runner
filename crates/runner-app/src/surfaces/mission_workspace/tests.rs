use std::sync::Arc;

use chrono::Utc;
use gpui::prelude::*;
use gpui::px;
use runner_app::ui::SessionControlKind;
use runner_backend::model::{Event, EventKind, MissionStatus, SessionStatus};

use super::*;
use crate::surfaces::*;
use crate::*;

fn signal(signal_type: &str, payload: serde_json::Value) -> Event {
    Event {
        id: signal_type.into(),
        ts: Utc::now(),
        crew_id: "crew".into(),
        mission_id: "mission".into(),
        kind: EventKind::Signal,
        from: "system".into(),
        to: None,
        signal_type: Some(runner_backend::model::SignalType::new(signal_type)),
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
        Some(&runner_backend::session::manager::SessionActivityState::Idle)
    );
    assert_eq!(
        statuses.get("reviewer"),
        Some(&runner_backend::session::manager::SessionActivityState::Idle)
    );
    assert!(observations.is_empty());
}

#[test]
fn sidebar_and_mission_fills_follow_carbon_and_runner_light() {
    use crate::theme_snapshot::{assert_fill, ThemeGuard};
    use gpui::{TestAppContext, VisualTestContext};
    use runner_backend::session::manager::{OutputEvent, SessionEvents};
    use runner_backend::{db, event_bus, events, mcp, router, session, shell_path, windows};
    use std::sync::{Mutex, RwLock};

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
    let core = AppCore {
        db: pool.clone(),
        app_data_dir: temp.path().to_owned(),
        sessions: session::SessionManager::new(
            runtime_shell_env.clone(),
            runtime_discovery.clone(),
            Arc::new(session::pty_runtime::PtyRuntime::new()),
        ),
        runtime_shell_env,
        runtime_discovery,
        buses: event_bus::BusRegistry::new(),
        routers: router::RouterRegistry::new(),
        mission_grid_hint: Arc::new(Mutex::new(None)),
        mcp: Arc::new(mcp::McpHandle::new()),
        windows: Arc::new(windows::WindowRegistry::new()),
        events: events::EventChannel::new(),
        session_event_observer: Default::default(),
        app_version: "0.0.0-test".into(),
    };

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
    for (id, mission_id) in [("direct", None), ("slot", Some("mission".into()))] {
        core.session_events().output(&OutputEvent {
            session_id: id.into(),
            mission_id,
            seq: 1,
            bytes: b"ready".to_vec(),
        });
    }
    let bridge = cx.update(|cx| store.read(cx).bridge.clone());
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
                Some(runner_backend::ops::mission::mission_get(&core, "mission").unwrap());
            workspace.crew = Some(runner_backend::ops::crew::crew_get(&core, "crew").unwrap());
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
        assert_fill(&mut visual, "MISSION_BG", colors.bg);
        assert_fill(&mut visual, "MISSION_PANEL", colors.panel);
        assert_fill(&mut visual, "MISSION_ACCENT", colors.accent);
        let palette = app_settings::terminal_palette(&AppSettings::default(), variant);
        assert_eq!(bridge.session("direct").unwrap().palette(), palette);
        assert_eq!(bridge.session("slot").unwrap().palette(), palette);
    }

    let archived_slot = SessionRow {
        live_title: None,
        session: runner_backend::model::Session {
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
        runner_backend::repo::session_attention::record_completion(
            &conn,
            "archived-slot",
            false,
            1,
        )
        .unwrap();
    }
    host.update(&mut cx, |root, window, cx| {
        root.mission_workspace.update(cx, |workspace, _| {
            workspace.sessions = vec![archived_slot.clone()];
            workspace.active_tab = MissionTab::Session("archived-slot".into());
            workspace.mission.as_mut().unwrap().archived_at = Some(Utc::now());
            workspace.session_observations.insert(
                "archived".into(),
                runner_backend::session::status::AgentStatus {
                    lifecycle: runner_backend::session::status::Lifecycle::Stopped,
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
    assert!(runner_backend::repo::session_attention::any_unread(
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
    assert!(!runner_backend::repo::session_attention::any_unread(
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
