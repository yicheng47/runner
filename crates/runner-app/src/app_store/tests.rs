use super::*;

#[test]
fn remote_status_shows_signed_in_surfaces_and_wakes_every_window() {
    use runner_core::remote::{RemoteState, RemoteStatus, StopReason};
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = super::test_lifecycle_store(&mut cx, temp.path());
    cx.run_until_parked();
    store.update(&mut cx, |store, _| {
        for state in [
            RemoteState::Connecting { attempt: 0 },
            RemoteState::Connected {
                since: chrono::Utc::now(),
            },
            RemoteState::Reconnecting {
                attempt: 1,
                next_retry_at: chrono::Utc::now(),
            },
            RemoteState::Paused,
            RemoteState::Stopped {
                reason: StopReason::Removed,
            },
            RemoteState::Off,
        ] {
            let previous = store.remote_signed_in();
            let revision = store.revisions.shell_settings;
            let status = RemoteStatus {
                state,
                host_id: Some("old-host".into()),
                account: Some(Default::default()),
                ..Default::default()
            };
            let signed_in = crate::surfaces::settings::remote::account::signed_in(&status);
            store.apply_remote_update(super::RemoteUpdate::Status(status), false);
            assert_eq!(store.remote_signed_in(), signed_in);
            assert_eq!(
                store.revisions.shell_settings,
                revision + u64::from(previous != signed_in)
            );
        }
    });
}

#[test]
fn restored_pin_lock_is_silent_on_startup_and_reconnect_but_later_locks_notify() {
    use runner_core::remote::RemoteTrust;
    let mut cx = gpui::TestAppContext::single();
    let temp = tempfile::tempdir().unwrap();
    let store = super::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        let unlocked = RemoteTrust {
            fingerprint: Some("630d cd29 66c4 3366 9112".into()),
            pin_set: true,
            ..Default::default()
        };
        let locked = RemoteTrust {
            pin_locked: true,
            ..unlocked.clone()
        };
        let baseline = store.revisions.remote_notice;
        for cycle in 0..2 {
            // Startup/reconnect reads the fast default while restore is held,
            // then receives the restored state through the event stream.
            store.apply_remote_update(super::RemoteUpdate::Trust(RemoteTrust::default()), false);
            store.apply_remote_update(super::RemoteUpdate::Trust(locked.clone()), true);
            assert!(store.remote_trust.as_ref().unwrap().pin_locked);
            assert_eq!(store.revisions.remote_notice, baseline + cycle);
            store.apply_remote_update(super::RemoteUpdate::Trust(locked.clone()), true);
            assert_eq!(store.revisions.remote_notice, baseline + cycle);
            store.apply_remote_update(super::RemoteUpdate::Trust(unlocked.clone()), true);
            store.apply_remote_update(super::RemoteUpdate::Trust(locked.clone()), true);
            assert_eq!(store.revisions.remote_notice, baseline + cycle + 1);
            assert!(matches!(
                store.remote_warning,
                Some(super::RemoteWarning::PinLocked)
            ));
        }
        // A fully restored snapshot at reconnect is silent as well.
        store.apply_remote_update(super::RemoteUpdate::Trust(unlocked), false);
        assert_eq!(store.revisions.remote_notice, baseline + 2);
        store.apply_remote_update(super::RemoteUpdate::Trust(locked), true);
        assert_eq!(store.revisions.remote_notice, baseline + 3);
    });
}

#[test]
fn working_count_excludes_busy_shells_and_uses_effective_mission_runtime() {
    use gpui::TestAppContext;
    use runner_core::protocol::status::{Activity, Lifecycle};
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store);
        assert!(store.error.is_none(), "{:?}", store.error);
        assert_eq!(store.working_agent_count(), 2);
        store
            .session_statuses
            .get_mut("direct-agent")
            .unwrap()
            .observation
            .activity = Activity::Idle;
        assert_eq!(store.working_agent_count(), 1);
        store
            .session_statuses
            .get_mut("mission-agent")
            .unwrap()
            .lifecycle = Lifecycle::Stopped;
        assert_eq!(store.working_agent_count(), 0);
    });
}

fn event(name: &'static str) -> AppEvent {
    AppEvent {
        name: name.to_owned(),
        payload: serde_json::Value::Null,
    }
}

#[test]
fn daemon_notice_survives_refresh_errors_and_clears_on_reconnect() {
    use gpui::{AppContext as _, TestAppContext};
    use runner_daemon::{db, session, shell_path};
    use std::sync::RwLock;
    let root = tempfile::tempdir().unwrap();
    let env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
    let discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
    let core = crate::test_support::core(
        Arc::new(db::open_fixture_pool(&root.path().join("runner.db")).unwrap()),
        root.path().into(),
        session::SessionManager::new(
            env.clone(),
            discovery.clone(),
            Arc::new(session::pty_runtime::PtyRuntime::new()),
        ),
        env,
        discovery,
    );
    let events = core.events.clone();
    let mut cx = TestAppContext::single();
    let store = cx.new(|cx| {
        AppStore::new(
            core,
            None,
            None,
            root.path().join("settings.json"),
            AppSettings::default(),
            None,
            cx,
        )
    });
    let before = store.read_with(&cx, |store, _| store.revisions.error);
    events.emit(
        "daemon/disconnected",
        &serde_json::json!({"restart_limit":true}),
    );
    cx.run_until_parked();
    store.update(&mut cx, |store, _| {
        store.record_error("connection closed".into())
    });
    store.read_with(&cx, |store, _| {
        assert!(store.error.is_none());
        assert_eq!(
            store.daemon_notice,
            Some(runner_app::lifecycle::DaemonNotice::Repeated)
        );
        assert!(store.revisions.error > before);
    });
    events.emit("daemon/reconnected", &serde_json::Value::Null);
    cx.run_until_parked();
    store.read_with(&cx, |store, _| {
        assert!(!store.daemon_disconnected);
        assert!(store.error.is_none() && store.daemon_notice.is_none());
    });
    events.emit(
        "daemon/disconnected",
        &serde_json::json!({"restart_limit":true,"in_use":true}),
    );
    cx.run_until_parked();
    store.read_with(&cx, |store, _| {
        assert!(store.daemon_disconnected);
        assert_eq!(
            store.daemon_notice,
            Some(runner_app::lifecycle::DaemonNotice::InUse)
        );
    });
    events.emit("daemon/reconnected", &serde_json::Value::Null);
    cx.run_until_parked();
    store.read_with(&cx, |store, _| {
        assert!(!store.daemon_disconnected);
        assert!(store.error.is_none() && store.daemon_notice.is_none());
    });
    store.update(&mut cx, |store, _| {
        store.live_session_count = 6;
        store.stopped_session_count = 6;
    });
    events.emit("daemon/disconnected", &serde_json::Value::Null);
    cx.run_until_parked();
    store.read_with(&cx, |store, _| {
        assert!(store.daemon_disconnected && store.error.is_none());
    });
    events.emit("daemon/reconnected", &serde_json::Value::Null);
    cx.run_until_parked();
    store.read_with(&cx, |store, _| {
        assert!(!store.daemon_disconnected);
        assert_eq!(
            store
                .error
                .as_ref()
                .map(|error| error.resolve().to_string())
                .as_deref(),
            Some("Background service restarted · 6 sessions stopped")
        );
    });
}

#[test]
fn refresh_kinds_preserve_each_data_dependency() {
    assert_eq!(
        StoreRefreshKind::for_event(&event("window_focus_map")),
        Some(StoreRefreshKind::Windows)
    );
    assert_eq!(
        StoreRefreshKind::for_event(&event("session/status")),
        Some(StoreRefreshKind::Activity)
    );
    assert_eq!(
        StoreRefreshKind::for_event(&event("chat/layout-changed")),
        Some(StoreRefreshKind::Nodes)
    );
    assert_eq!(
        StoreRefreshKind::for_event(&event("mission/changed")),
        Some(StoreRefreshKind::All)
    );
    assert_eq!(
        StoreRefreshKind::for_event(&event("session/fork-started")),
        Some(StoreRefreshKind::All)
    );
    assert_eq!(
        StoreRefreshKind::for_event(&event("onboarding/changed")),
        Some(StoreRefreshKind::Onboarding)
    );
    assert_eq!(StoreRefreshKind::for_event(&event("unrelated")), None);
}

#[test]
fn coalesced_refreshes_cannot_drop_a_data_domain() {
    assert_eq!(
        StoreRefreshKind::Activity.merge(StoreRefreshKind::Missions),
        StoreRefreshKind::All
    );
    assert_eq!(
        StoreRefreshKind::Nodes.merge(StoreRefreshKind::Nodes),
        StoreRefreshKind::Nodes
    );
}

#[test]
fn entity_refresh_events_match_role_and_crew_dependencies() {
    assert_eq!(
        EntityRefreshKind::for_event(&event("role/activity")),
        EntityRefreshKind::Roles
    );
    for name in ["role/changed", "crew/changed", "slot/changed"] {
        assert_eq!(
            EntityRefreshKind::for_event(&event(name)),
            EntityRefreshKind::All
        );
    }
    assert_eq!(
        EntityRefreshKind::for_event(&event("session/status")),
        EntityRefreshKind::None
    );
}

#[test]
fn entity_refresh_merge_covers_every_pair() {
    use EntityRefreshKind::{All, None, Roles};

    for (left, right, expected) in [
        (None, None, None),
        (None, Roles, Roles),
        (None, All, All),
        (Roles, None, Roles),
        (Roles, Roles, Roles),
        (Roles, All, All),
        (All, None, All),
        (All, Roles, All),
        (All, All, All),
    ] {
        assert_eq!(left.merge(right), expected, "{left:?} + {right:?}");
    }
}

#[test]
fn revision_reactions_match_root_side_effects() {
    let before = StoreRevisions::default();
    let mut after_settings = before;
    after_settings.settings = 1;
    assert_eq!(
        after_settings.reactions_since(before),
        StoreReactions {
            notify: true,
            ..Default::default()
        }
    );

    let mut after_terminal_settings = before;
    after_terminal_settings.settings = 1;
    after_terminal_settings.terminal_settings = 1;
    assert_eq!(
        after_terminal_settings.reactions_since(before),
        StoreReactions {
            apply_terminal_settings: true,
            notify: true,
            notify_without_settings: true,
            ..Default::default()
        }
    );

    let mut after_tabs = before;
    after_tabs.nodes = 1;
    after_tabs.tab_rows = 1;
    assert_eq!(
        after_tabs.reactions_since(before),
        StoreReactions {
            reload_tabs: true,
            notify: true,
            notify_without_settings: true,
            ..Default::default()
        }
    );

    let mut after_full_refresh = before;
    after_full_refresh.sessions = 1;
    after_full_refresh.full_refresh = 1;
    assert_eq!(
        after_full_refresh.reactions_since(before),
        StoreReactions {
            prune_window_state: true,
            notify: true,
            notify_without_settings: true,
            ..Default::default()
        }
    );

    let mut after_mission_settings = before;
    after_mission_settings.settings = 1;
    after_mission_settings.mission_settings = 1;
    let reactions = after_mission_settings.reactions_since(before);
    assert!(reactions.mission_settings);
    assert!(!reactions.notify_without_settings);
    assert!(!reactions.notify_shell());

    after_mission_settings.shell_settings = 1;
    assert!(after_mission_settings
        .reactions_since(before)
        .notify_shell());

    after_mission_settings.sessions = 1;
    assert!(after_mission_settings
        .reactions_since(before)
        .notify_shell());
}

#[test]
fn role_data_and_surface_reload_revisions_are_independent() {
    let before = StoreRevisions::default();
    let mut after_data = before;
    after_data.roles = 1;
    assert!(!after_data.reactions_since(before).reload_role_surfaces);

    let mut after_event = before;
    after_event.role_surfaces = 1;
    assert!(after_event.reactions_since(before).reload_role_surfaces);
}

#[test]
fn terminal_settings_snapshot_tracks_the_resolved_mode_and_both_picks() {
    use crate::theme::ThemeVariant;
    let settings = AppSettings::default();
    assert_ne!(
        TerminalSettingsSnapshot::for_variant(&settings, ThemeVariant::Carbon),
        TerminalSettingsSnapshot::for_variant(&settings, ThemeVariant::RunnerLight)
    );
    assert_eq!(
        TerminalSettingsSnapshot::for_variant(&settings, ThemeVariant::RunnerLight),
        TerminalSettingsSnapshot::for_variant(&settings, ThemeVariant::CatppuccinLatte)
    );
    assert_eq!(
        TerminalSettingsSnapshot::for_variant(&settings, ThemeVariant::Carbon),
        TerminalSettingsSnapshot::for_variant(&settings, ThemeVariant::CatppuccinMocha)
    );

    let mut light_pick = settings.clone();
    light_pick.light_app_theme = LightTheme::GruvboxLight;
    let mut dark_pick = settings.clone();
    dark_pick.dark_app_theme = DarkTheme::CatppuccinMocha;
    for variant in [ThemeVariant::Carbon, ThemeVariant::RunnerLight] {
        assert_ne!(
            TerminalSettingsSnapshot::for_variant(&settings, variant),
            TerminalSettingsSnapshot::for_variant(&light_pick, variant),
            "{variant:?}"
        );
        assert_ne!(
            TerminalSettingsSnapshot::for_variant(&settings, variant),
            TerminalSettingsSnapshot::for_variant(&dark_pick, variant),
            "{variant:?}"
        );
    }
}

#[test]
fn font_family_names_invalidate_terminal_and_shell_settings() {
    let mut settings = AppSettings {
        terminal_font_family: TerminalFontFamily::new("SF Mono"),
        ..AppSettings::default()
    };
    let terminal = TerminalSettingsSnapshot::from(&settings);
    let shell = ShellSettingsSnapshot::from(&settings);
    settings.terminal_font_family = TerminalFontFamily::new("Fira Code");
    assert_ne!(terminal, TerminalSettingsSnapshot::from(&settings));
    assert_ne!(shell, ShellSettingsSnapshot::from(&settings));
}

#[test]
fn terminal_settings_snapshot_ignores_unrelated_preferences() {
    let before = AppSettings {
        dark_app_theme: DarkTheme::CatppuccinMocha,
        ..AppSettings::default()
    };
    let mut after = before.clone();
    after.sidebar_width += 1.;
    assert_eq!(
        TerminalSettingsSnapshot::from(&before),
        TerminalSettingsSnapshot::from(&after)
    );

    after.terminal_font_size += 1;
    assert_ne!(
        TerminalSettingsSnapshot::from(&before),
        TerminalSettingsSnapshot::from(&after)
    );
}

#[test]
fn mission_settings_snapshot_tracks_only_workspace_preferences() {
    let before = AppSettings::default();
    let mut after = before.clone();
    after.sidebar_width += 1.;
    assert_eq!(
        MissionSettingsSnapshot::from(&before),
        MissionSettingsSnapshot::from(&after)
    );

    after.mission_rail_width += 1.;
    assert_ne!(
        MissionSettingsSnapshot::from(&before),
        MissionSettingsSnapshot::from(&after)
    );
}

#[test]
fn shell_settings_snapshot_ignores_mission_preferences() {
    let before = AppSettings {
        dark_app_theme: DarkTheme::CatppuccinMocha,
        ..AppSettings::default()
    };
    let mut after = before.clone();
    after.mission_rail_width += 1.;
    after
        .last_mission_terminal_ids
        .insert("mission".into(), "session".into());
    assert_eq!(
        ShellSettingsSnapshot::from(&before),
        ShellSettingsSnapshot::from(&after)
    );

    after.sidebar_active_only = true;
    assert_ne!(
        ShellSettingsSnapshot::from(&before),
        ShellSettingsSnapshot::from(&after)
    );
    after.sidebar_active_only = false;
    after.sidebar_width += 1.;
    assert_ne!(
        ShellSettingsSnapshot::from(&before),
        ShellSettingsSnapshot::from(&after)
    );
}

struct GuardedTransport {
    inner: runner_daemon::daemon::InProcessTransport,
    forbid_foreground: Arc<std::sync::atomic::AtomicBool>,
    calls: Arc<std::sync::Mutex<Vec<String>>>,
}

impl runner_core::protocol::Transport for GuardedTransport {
    fn call(
        &self,
        request: runner_core::protocol::Request,
    ) -> std::result::Result<runner_core::protocol::Response, runner_core::protocol::ClientError>
    {
        let debug_name = format!("{request:?}");
        let name = debug_name
            .split([' ', '{', '('])
            .next()
            .unwrap_or("unknown")
            .to_owned();
        assert!(
            !self
                .forbid_foreground
                .load(std::sync::atomic::Ordering::Acquire),
            "Transport::call({name}) invoked synchronously on GPUI foreground turn"
        );
        self.calls.lock().unwrap().push(name);
        self.inner.call(request)
    }

    fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
        self.inner.subscribe()
    }

    fn attach(
        &self,
        session_id: &str,
    ) -> std::result::Result<
        runner_core::protocol::terminal::TerminalAttachment,
        runner_core::protocol::ClientError,
    > {
        self.inner.attach(session_id)
    }
}

#[test]
fn steady_state_store_refresh_offloads_client_calls_and_coalesces_latest_state_1377() {
    use gpui::TestAppContext;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    cx.run_until_parked();

    let forbid_foreground = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    store.update(&mut cx, |store, _| {
        store.client = DaemonClient::new(Arc::new(GuardedTransport {
            inner: runner_daemon::daemon::InProcessTransport(core.clone()),
            forbid_foreground: Arc::clone(&forbid_foreground),
            calls: Arc::clone(&calls),
        }));
    });

    let tab_id = {
        let conn = core.db.get().unwrap();
        conn.execute_batch(
                "INSERT INTO roles (id, handle, display_name, runtime, command, created_at, updated_at)
                 VALUES ('role-1377', 'agent', 'Agent', 'codex', 'codex', '2026-11-18T00:00:00Z', '2026-11-18T00:00:00Z');
                 INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('crew-1377', 'Crew', '2026-11-18T00:00:00Z', '2026-11-18T00:00:00Z');
                 INSERT INTO missions (id, crew_id, title, status, started_at)
                 VALUES ('mission-1377', 'crew-1377', 'Async Mission', 'running', '2026-11-18T00:00:00Z');
                 INSERT INTO slots (id, crew_id, role_id, slot_handle, position, lead, added_at)
                 VALUES ('slot-1377', 'crew-1377', 'role-1377', 'agent', 0, 1, '2026-11-18T00:00:00Z');
                 INSERT INTO sessions (id, role_id, mission_id, slot_id, status, agent_runtime, agent_command)
                 VALUES ('mission-slot-1377', 'role-1377', 'mission-1377', 'slot-1377', 'running', 'codex', 'codex');
                 INSERT INTO sessions (id, status, agent_runtime, agent_command)
                 VALUES ('direct-1377', 'stopped', 'codex', 'codex');",
            )
            .unwrap();
        let layout = crate::pane_layout::PaneLayout::single(Some("direct-1377"), &[]);
        runner_daemon::repo::node::create_tab(
            &conn,
            None,
            "Tab 1377",
            0,
            &layout.serialize().unwrap(),
        )
        .unwrap()
        .id
    };

    forbid_foreground.store(true, Ordering::Release);
    store.update(&mut cx, |store, cx| {
        store.refresh(StoreRefreshKind::Windows, cx);
        assert!(store.refresh_in_flight);
        assert!(store.pending_refresh.is_none());
        // Concurrent refreshes while Windows is in flight coalesce into a single follow-up All.
        store.refresh(StoreRefreshKind::Activity, cx);
        store.refresh(StoreRefreshKind::Nodes, cx);
        store.refresh(StoreRefreshKind::Missions, cx);
        assert_eq!(
            store.pending_refresh,
            Some((StoreRefreshKind::All, EntityRefreshKind::None))
        );
    });
    forbid_foreground.store(false, Ordering::Release);
    assert!(
        calls.lock().unwrap().is_empty(),
        "steady-state AppStore::refresh must make zero foreground RPCs"
    );

    cx.run_until_parked();
    store.read_with(&cx, |store, _| {
        assert!(!store.refresh_in_flight);
        assert!(store.pending_refresh.is_none());
        assert!(store
            .missions
            .iter()
            .any(|m| m.mission.id == "mission-1377"));
        assert!(store.mission_agent_ids.contains("mission-slot-1377"));
        assert!(store.nodes.iter().any(|n| n.id == tab_id));
        assert!(store.sessions.iter().any(|s| s.session_id == "direct-1377"));
    });

    // A foreground replace_node after an in-flight Nodes refresh starts must not be clobbered.
    store.update(&mut cx, |store, cx| {
        store.refresh(StoreRefreshKind::Nodes, cx);
        let mut renamed = store.nodes.iter().find(|n| n.id == tab_id).unwrap().clone();
        renamed.name = Some("Newer Foreground Name".into());
        store.replace_node(renamed, cx);
    });
    cx.run_until_parked();
    store.read_with(&cx, |store, _| {
        let node = store.nodes.iter().find(|n| n.id == tab_id).unwrap();
        assert_eq!(node.name.as_deref(), Some("Newer Foreground Name"));
    });
}

#[test]
fn window_activation_subjects_and_viewed_marks_run_off_foreground_and_release_pool_1377() {
    use gpui::{TestAppContext, VisualTestContext};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    let (app_weak, db_weak) = {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let app_weak = Rc::downgrade(&cx.app);
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let core = store.read_with(&cx, |store, _| store.test_core.clone());
        let db_weak = Arc::downgrade(&core.db);

        let (tab_a, tab_b) = {
            let conn = core.db.get().unwrap();
            conn.execute_batch(
                "INSERT INTO crews (id, name, created_at, updated_at)
                     VALUES ('crew-w', 'Crew', '2026-11-18T00:00:00Z', '2026-11-18T00:00:00Z');
                     INSERT INTO missions (id, crew_id, title, status, started_at)
                     VALUES ('mission-w', 'crew-w', 'Mission W', 'running', '2026-11-18T00:00:00Z');
                     INSERT INTO sessions (id, status, agent_runtime, agent_command)
                     VALUES ('chat-a', 'stopped', 'codex', 'codex'),
                            ('chat-b', 'stopped', 'codex', 'codex');
                     INSERT INTO sessions (id, mission_id, status, agent_runtime, agent_command)
                     VALUES ('slot-1', 'mission-w', 'stopped', 'codex', 'codex'),
                            ('slot-2', 'mission-w', 'stopped', 'codex', 'codex');",
            )
            .unwrap();
            let layout_a = crate::pane_layout::PaneLayout::single(Some("chat-a"), &[]);
            let layout_b = crate::pane_layout::PaneLayout::single(Some("chat-b"), &[]);
            let row_a = runner_daemon::repo::node::create_tab(
                &conn,
                None,
                "A",
                0,
                &layout_a.serialize().unwrap(),
            )
            .unwrap();
            let row_b = runner_daemon::repo::node::create_tab(
                &conn,
                None,
                "B",
                1,
                &layout_b.serialize().unwrap(),
            )
            .unwrap();
            runner_daemon::repo::session_attention::record_completion(&conn, "chat-a", false, 100)
                .unwrap();
            runner_daemon::repo::session_attention::record_completion(&conn, "chat-b", false, 200)
                .unwrap();
            runner_daemon::repo::session_attention::record_completion(&conn, "slot-1", false, 300)
                .unwrap();
            runner_daemon::repo::session_attention::record_completion(&conn, "slot-2", false, 400)
                .unwrap();
            runner_daemon::repo::node::record_completion(
                &conn,
                &row_a.id,
                false,
                chrono::Utc::now(),
            )
            .unwrap();
            runner_daemon::repo::node::record_completion(
                &conn,
                &row_b.id,
                false,
                chrono::Utc::now(),
            )
            .unwrap();
            (row_a.id, row_b.id)
        };

        store.update(&mut cx, |store, cx| {
            store.refresh(StoreRefreshKind::All, cx);
        });
        cx.run_until_parked();

        cx.update(|cx| {
            cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
            crate::notifications::install(cx);
            cx.set_global(crate::GlobalAppStore(store.clone()));
            cx.set_global(crate::WindowLayoutCheckpoint::default());
            #[cfg(target_os = "macos")]
            let updater = cx.new(|cx| crate::Updater::new(false, cx));
            #[cfg(any(windows, target_os = "linux"))]
            let updater = cx.new(|cx| crate::Updater::new(false, temp.path().join("updates"), cx));
            cx.set_global(crate::GlobalUpdater(updater));
        });

        core.windows.register("win-1377");
        let host = cx.add_window(|window, cx| {
            let mut root = crate::surfaces::root::NativeRoot::new(
                "win-1377".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.apply_tab_rows(cx);
            assert!(root.tabs.activate(&tab_a));
            root.route = crate::AppRoute::Chat;
            root
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();

        let forbid_foreground = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(Mutex::new(Vec::new()));
        store.update(&mut visual, |store, _| {
            store.client = DaemonClient::new(Arc::new(GuardedTransport {
                inner: runner_daemon::daemon::InProcessTransport(core.clone()),
                forbid_foreground: Arc::clone(&forbid_foreground),
                calls: Arc::clone(&calls),
            }));
        });

        // Rapid focus + tab-switch burst executes zero RPCs on the foreground UI turn,
        // coalescing onto the final active tab (`tab_b` / `chat-b`).
        forbid_foreground.store(true, Ordering::Release);
        host.update(&mut visual, |root, window, cx| {
            root.report_current_subjects(cx);
            root.sync_window_activation(window, cx);
            assert!(root.tabs.activate(&tab_b));
            root.report_current_subjects(cx);
            root.mark_active_tab_viewed(window, cx);
        })
        .unwrap();
        forbid_foreground.store(false, Ordering::Release);
        assert!(
            calls.lock().unwrap().is_empty(),
            "window activation and tab-viewed updates must make zero foreground RPCs"
        );

        visual.run_until_parked();
        assert!(!runner_daemon::repo::session_attention::any_unread(
            &core.db.get().unwrap(),
            &["chat-b".into()],
        )
        .unwrap());
        store.read_with(&visual, |store, _| {
            let node_b = store.nodes.iter().find(|n| n.id == tab_b).unwrap();
            assert!(node_b.last_viewed_at.is_some());
        });

        // Mission workspace mark_active_session_viewed also offloads and coalesces slot switches.
        let mission_row = runner_daemon::ops::mission::mission_get(&core, "mission-w").unwrap();
        calls.lock().unwrap().clear();
        forbid_foreground.store(true, Ordering::Release);
        host.update(&mut visual, |root, window, cx| {
            root.route = crate::AppRoute::Mission("mission-w".into());
            root.mission_workspace.update(cx, |workspace, cx| {
                workspace.set_test_active_mission_session(mission_row.clone(), "slot-1");
                workspace.mark_active_session_viewed(window, cx);
                workspace.set_test_active_mission_session(mission_row, "slot-2");
                workspace.mark_active_session_viewed(window, cx);
            });
        })
        .unwrap();
        forbid_foreground.store(false, Ordering::Release);
        assert!(
            calls.lock().unwrap().is_empty(),
            "mark_active_session_viewed must make zero foreground RPCs"
        );

        visual.run_until_parked();
        assert!(!runner_daemon::repo::session_attention::any_unread(
            &core.db.get().unwrap(),
            &["slot-2".into()],
        )
        .unwrap());

        // Queue fresh async store + window tasks and tear down immediately without parking:
        // entity-owned Task handles must cancel cleanly with zero leaked AppCore/DbPool refs.
        host.update(&mut visual, |root, window, cx| {
            root.app_store.update(cx, |store, store_cx| {
                store.refresh(StoreRefreshKind::All, store_cx)
            });
            root.sync_window_activation(window, cx);
        })
        .unwrap();
        drop(core);
        (app_weak, db_weak)
    };
    assert_eq!(app_weak.strong_count(), 0);
    assert_eq!(db_weak.strong_count(), 0);
}

type RequestHook = (
    Box<dyn Fn(&runner_core::protocol::Request) -> bool>,
    Box<dyn FnOnce()>,
);

std::thread_local! {
    static BEFORE_CALL_HOOKS: std::cell::RefCell<Vec<RequestHook>> = const { std::cell::RefCell::new(Vec::new()) };
    static AFTER_CALL_HOOKS: std::cell::RefCell<Vec<RequestHook>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn push_before_call_hook(
    pred: impl Fn(&runner_core::protocol::Request) -> bool + 'static,
    hook: impl FnOnce() + 'static,
) {
    BEFORE_CALL_HOOKS.with(|hooks| {
        hooks.borrow_mut().push((Box::new(pred), Box::new(hook)));
    });
}

fn push_after_call_hook(
    pred: impl Fn(&runner_core::protocol::Request) -> bool + 'static,
    hook: impl FnOnce() + 'static,
) {
    AFTER_CALL_HOOKS.with(|hooks| {
        hooks.borrow_mut().push((Box::new(pred), Box::new(hook)));
    });
}

fn run_matching_hook(
    slot: &'static std::thread::LocalKey<std::cell::RefCell<Vec<RequestHook>>>,
    request: &runner_core::protocol::Request,
) {
    let matched = slot.with(|hooks| {
        let mut hooks = hooks.borrow_mut();
        hooks
            .iter()
            .position(|(pred, _)| pred(request))
            .map(|idx| hooks.remove(idx).1)
    });
    if let Some(hook) = matched {
        hook();
    }
}

struct HookGuard;
impl Drop for HookGuard {
    fn drop(&mut self) {
        BEFORE_CALL_HOOKS.with(|hooks| hooks.borrow_mut().clear());
        AFTER_CALL_HOOKS.with(|hooks| hooks.borrow_mut().clear());
    }
}

struct HookedTransport {
    inner: runner_daemon::daemon::InProcessTransport,
    forbid_foreground: Arc<AtomicBool>,
    calls: Arc<Mutex<Vec<runner_core::protocol::Request>>>,
}

impl runner_core::protocol::Transport for HookedTransport {
    fn call(
        &self,
        request: runner_core::protocol::Request,
    ) -> std::result::Result<runner_core::protocol::Response, runner_core::protocol::ClientError>
    {
        assert!(
            !self.forbid_foreground.load(Ordering::Acquire),
            "Transport::call({request:?}) invoked synchronously on GPUI foreground turn"
        );
        let serialized = serde_json::to_value(&request).unwrap();
        let for_log: runner_core::protocol::Request =
            serde_json::from_value(serialized.clone()).unwrap();
        let for_after: runner_core::protocol::Request = serde_json::from_value(serialized).unwrap();
        self.calls.lock().unwrap().push(for_log);
        run_matching_hook(&BEFORE_CALL_HOOKS, &request);
        let response = self.inner.call(request);
        run_matching_hook(&AFTER_CALL_HOOKS, &for_after);
        response
    }

    fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
        self.inner.subscribe()
    }

    fn attach(
        &self,
        session_id: &str,
    ) -> std::result::Result<
        runner_core::protocol::terminal::TerminalAttachment,
        runner_core::protocol::ClientError,
    > {
        self.inner.attach(session_id)
    }
}

#[test]
fn window_queue_route_change_invalidates_pending_and_inflight_viewed_tab_1377() {
    use gpui::{TestAppContext, VisualTestContext};
    use runner_core::protocol::{window::Subject, Request};

    let _hooks = HookGuard;
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let core = store.read_with(&cx, |store, _| store.test_core.clone());

    let (tab_a, tab_b) = {
        let conn = core.db.get().unwrap();
        conn.execute_batch(
            "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('crew-r', 'Crew', '2026-11-18T00:00:00Z', '2026-11-18T00:00:00Z');
                 INSERT INTO missions (id, crew_id, title, status, started_at)
                 VALUES ('mission-r', 'crew-r', 'Mission R', 'running', '2026-11-18T00:00:00Z');
                 INSERT INTO sessions (id, status, agent_runtime, agent_command)
                 VALUES ('chat-ra', 'stopped', 'codex', 'codex'),
                        ('chat-rb', 'stopped', 'codex', 'codex');",
        )
        .unwrap();
        let layout_a = crate::pane_layout::PaneLayout::single(Some("chat-ra"), &[]);
        let layout_b = crate::pane_layout::PaneLayout::single(Some("chat-rb"), &[]);
        let a = runner_daemon::repo::node::create_tab(
            &conn,
            None,
            "RA",
            0,
            &layout_a.serialize().unwrap(),
        )
        .unwrap()
        .id;
        let b = runner_daemon::repo::node::create_tab(
            &conn,
            None,
            "RB",
            1,
            &layout_b.serialize().unwrap(),
        )
        .unwrap()
        .id;
        (a, b)
    };

    store.update(&mut cx, |store, cx| {
        store.refresh(StoreRefreshKind::All, cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        crate::notifications::install(cx);
    });

    core.windows.register("win-route");
    let host = cx.add_window(|window, cx| {
        let mut root = crate::surfaces::root::NativeRoot::new(
            "win-route".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.apply_tab_rows(cx);
        assert!(root.tabs.activate(&tab_a));
        root.route = crate::AppRoute::Chat;
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();

    let forbid_foreground = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(Mutex::new(Vec::new()));
    store.update(&mut visual, |store, _| {
        store.client = DaemonClient::new(Arc::new(HookedTransport {
            inner: runner_daemon::daemon::InProcessTransport(core.clone()),
            forbid_foreground: Arc::clone(&forbid_foreground),
            calls: Arc::clone(&calls),
        }));
    });

    // 1. While batch 0 is blocked in report_subjects, enqueue viewed mark for tab_b and then
    // switch route to Mission: pending viewed_tab must be cleared so node_mark_viewed never
    // clobbers the mission route's subjects.
    {
        let mut hook_visual = visual.clone();
        let tab_b_clone = tab_b.clone();
        push_before_call_hook(
            |req| matches!(req, Request::report_subjects { label, .. } if label == "win-route"),
            move || {
                host.update(&mut hook_visual, |root, window, cx| {
                    assert!(root.tabs.activate(&tab_b_clone));
                    root.mark_active_tab_viewed(window, cx);
                    root.set_route(crate::AppRoute::Mission("mission-r".into()), cx);
                })
                .unwrap();
            },
        );
    }
    host.update(&mut visual, |root, _, cx| root.report_current_subjects(cx))
        .unwrap();
    visual.run_until_parked();

    let snap = core
        .windows
        .snapshot()
        .into_iter()
        .find(|entry| entry.label == "win-route")
        .unwrap();
    assert_eq!(snap.subjects, vec![Subject::Mission("mission-r".into())]);
    assert!(core.windows.focused_direct_sessions("win-route").is_empty());
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|req| matches!(req, Request::node_mark_viewed { id, .. } if id == &tab_b)),
        "stale pending viewed_tab must be cleared when route leaves Chat"
    );

    // 2. When node_mark_viewed itself is already executing in flight and the window switches
    // to Settings, a follow-up report_subjects restores empty subjects and viewed_session_id=None.
    let mut hook_visual = visual.clone();
    host.update(&mut visual, |root, window, cx| {
            root.set_route(crate::AppRoute::Chat, cx);
            assert!(root.tabs.activate(&tab_b));
            push_before_call_hook(
                move |req| matches!(req, Request::node_mark_viewed { window_label, .. } if window_label == "win-route"),
                move || {
                    host.update(&mut hook_visual, |root, _, cx| {
                        root.set_route(crate::AppRoute::Settings, cx);
                    })
                    .unwrap();
                },
            );
            root.mark_active_tab_viewed(window, cx);
        })
        .unwrap();
    visual.run_until_parked();

    let snap = core
        .windows
        .snapshot()
        .into_iter()
        .find(|entry| entry.label == "win-route")
        .unwrap();
    assert!(snap.subjects.is_empty());
    assert!(core.windows.focused_direct_sessions("win-route").is_empty());
}

#[test]
fn two_window_activation_preserves_causal_focus_order_and_snapshot_monotonicity_1377() {
    use gpui::TestAppContext;
    use runner_core::protocol::Request;

    let _hooks = HookGuard;
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let core = store.read_with(&cx, |store, _| store.test_core.clone());

    let tab_shared = {
        let conn = core.db.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, status, agent_runtime, agent_command)
                 VALUES ('chat-shared', 'stopped', 'codex', 'codex')",
            [],
        )
        .unwrap();
        let layout = crate::pane_layout::PaneLayout::single(Some("chat-shared"), &[]);
        runner_daemon::repo::node::create_tab(
            &conn,
            None,
            "Shared",
            0,
            &layout.serialize().unwrap(),
        )
        .unwrap()
        .id
    };

    store.update(&mut cx, |store, cx| {
        store.refresh(StoreRefreshKind::All, cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        crate::notifications::install(cx);
    });

    core.windows.register("win-a");
    core.windows.register("win-b");
    let win_a = cx.add_window(|window, cx| {
        let mut root = crate::surfaces::root::NativeRoot::new(
            "win-a".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.apply_tab_rows(cx);
        assert!(root.tabs.activate(&tab_shared));
        root.route = crate::AppRoute::Chat;
        root
    });
    let win_b = cx.add_window(|window, cx| {
        let mut root = crate::surfaces::root::NativeRoot::new(
            "win-b".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.apply_tab_rows(cx);
        assert!(root.tabs.activate(&tab_shared));
        root.route = crate::AppRoute::Chat;
        root
    });
    cx.run_until_parked();

    let forbid_foreground = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(Mutex::new(Vec::new()));
    store.update(&mut cx, |store, _| {
        store.client = DaemonClient::new(Arc::new(HookedTransport {
            inner: runner_daemon::daemon::InProcessTransport(core.clone()),
            forbid_foreground: Arc::clone(&forbid_foreground),
            calls: Arc::clone(&calls),
        }));
    });

    // Activate win-a first; while win-a's batch is in-flight at report_subjects (before
    // mark_focused / node_mark_viewed), start an independent StoreRefreshKind::Windows fetch
    // AND activate win-b on the foreground thread.
    {
        let mut hook_cx = cx.clone();
        let store_for_hook = store.clone();
        push_before_call_hook(
            |req| matches!(req, Request::report_subjects { label, .. } if label == "win-a"),
            move || {
                store_for_hook.update(&mut hook_cx, |store, store_cx| {
                    store.refresh(StoreRefreshKind::Windows, store_cx);
                });
                win_b
                    .update(&mut hook_cx, |root, window, cx| {
                        window.activate_window();
                        root.sync_window_activation(window, cx);
                    })
                    .unwrap();
            },
        );
    }
    win_a
        .update(&mut cx, |root, window, cx| {
            window.activate_window();
            root.sync_window_activation(window, cx);
        })
        .unwrap();
    cx.run_until_parked();

    let shared_subject = runner_core::protocol::window::Subject::DirectChat("chat-shared".into());
    assert_eq!(
        runner_core::protocol::window::is_secondary_for(
            &core.windows.snapshot(),
            "win-a",
            &shared_subject,
        )
        .primary_label
        .as_deref(),
        Some("win-b"),
        "win-b activated second and must hold primary ownership in runnerd"
    );
    store.read_with(&cx, |store, _| {
            assert_eq!(
                runner_core::protocol::window::is_secondary_for(
                    &store.window_entries,
                    "win-a",
                    &shared_subject,
                )
                .primary_label
                .as_deref(),
                Some("win-b"),
                "stale StoreRefreshKind::Windows snapshot must not overwrite newer WindowCoordinator snapshot"
            );
        });
    win_b
        .read_with(&cx, |root, _| {
            assert!(
                !root.chat_secondaries.contains_key("chat-shared"),
                "active win-b must be primary"
            );
        })
        .unwrap();
    win_a
        .read_with(&cx, |root, _| {
            assert_eq!(
                root.chat_secondaries.get("chat-shared").map(String::as_str),
                Some("win-b"),
                "older win-a must yield to win-b as secondary"
            );
        })
        .unwrap();
}

#[test]
fn closing_window_mid_inflight_rpc_keeps_closed_label_absent_and_transfers_ownership_1377() {
    use gpui::TestAppContext;
    use runner_core::protocol::Request;

    let _hooks = HookGuard;
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let core = store.read_with(&cx, |store, _| store.test_core.clone());

    let tab_shared = {
        let conn = core.db.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, status, agent_runtime, agent_command)
                 VALUES ('chat-close-1377', 'running', 'codex', 'codex')",
            [],
        )
        .unwrap();
        let events: Arc<dyn runner_daemon::session::manager::SessionEvents> =
            Arc::new(core.session_events());
        core.sessions
            .prepare_unlisted_terminal("chat-close-1377", (80, 24), &core.db, &events)
            .unwrap();
        let layout = crate::pane_layout::PaneLayout::single(Some("chat-close-1377"), &[]);
        runner_daemon::repo::node::create_tab(
            &conn,
            None,
            "Close Test",
            0,
            &layout.serialize().unwrap(),
        )
        .unwrap()
        .id
    };

    store.update(&mut cx, |store, cx| {
        store
            .bridge
            .attach("chat-close-1377")
            .unwrap()
            .test_stop_frames();
        store.refresh(StoreRefreshKind::All, cx);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        crate::notifications::install(cx);
    });

    core.windows.register("win-survivor");
    core.windows.register("win-closing");
    let survivor = cx.add_window(|window, cx| {
        let mut root = crate::surfaces::root::NativeRoot::new(
            "win-survivor".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.apply_tab_rows(cx);
        assert!(root.tabs.activate(&tab_shared));
        root.route = crate::AppRoute::Chat;
        window.activate_window();
        root.sync_window_activation(window, cx);
        root.ensure_active_tab_attached(window, cx).unwrap();
        root
    });
    let closing = cx.add_window(|window, cx| {
        let mut root = crate::surfaces::root::NativeRoot::new(
            "win-closing".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.apply_tab_rows(cx);
        assert!(root.tabs.activate(&tab_shared));
        root.route = crate::AppRoute::Chat;
        root.ensure_active_tab_attached(window, cx).unwrap();
        root
    });
    cx.run_until_parked();

    let forbid_foreground = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(Mutex::new(Vec::new()));
    store.update(&mut cx, |store, _| {
        store.client = DaemonClient::new(Arc::new(HookedTransport {
            inner: runner_daemon::daemon::InProcessTransport(core.clone()),
            forbid_foreground: Arc::clone(&forbid_foreground),
            calls: Arc::clone(&calls),
        }));
    });

    // Activate win-closing, and while its report_subjects call is mid-execution on the
    // background worker, close win-closing on the foreground thread (with forbid_foreground=true!).
    {
        let mut hook_cx = cx.clone();
        let forbid_fg = Arc::clone(&forbid_foreground);
        push_before_call_hook(
            |req| matches!(req, Request::report_subjects { label, .. } if label == "win-closing"),
            move || {
                forbid_fg.store(true, Ordering::Release);
                closing
                    .update(&mut hook_cx, |root, window, cx| {
                        root.prepare_window_close(window, cx);
                        window.remove_window();
                    })
                    .unwrap();
                forbid_fg.store(false, Ordering::Release);
            },
        );
    }
    closing
        .update(&mut cx, |root, window, cx| {
            window.activate_window();
            root.sync_window_activation(window, cx);
        })
        .unwrap();
    cx.run_until_parked();

    assert!(
        !core
            .windows
            .snapshot()
            .iter()
            .any(|entry| entry.label == "win-closing"),
        "closed window must remain unregistered after its in-flight RPC batch finishes"
    );
    assert!(
        !calls.lock().unwrap().iter().any(|req| matches!(
            req,
            Request::mark_focused { label } | Request::node_mark_viewed { window_label: label, .. }
                if label == "win-closing"
        )),
        "cancelled batch must not issue mark_focused or node_mark_viewed after close"
    );
    survivor
        .read_with(&cx, |root, _| {
            assert!(
                !root.chat_secondaries.contains_key("chat-close-1377"),
                "surviving window must hold primary ownership"
            );
            assert!(
                root.attached.contains_key("chat-close-1377"),
                "surviving window must re-attach terminal after closed peer unregisters"
            );
        })
        .unwrap();

    // 2. Delay old batch's window_snapshot completion (which captured win-delayed-snap as
    // primary), close win-delayed-snap before that snapshot applies, and inspect BEFORE the
    // follow-up unregister batch executes: win-delayed-snap must stay absent from AppStore
    // and survivor must stay primary & attached throughout.
    core.windows.register("win-delayed-snap");
    let delayed = cx.add_window(|window, cx| {
        let mut root = crate::surfaces::root::NativeRoot::new(
            "win-delayed-snap".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.apply_tab_rows(cx);
        assert!(root.tabs.activate(&tab_shared));
        root.route = crate::AppRoute::Chat;
        root.ensure_active_tab_attached(window, cx).unwrap();
        root
    });
    cx.run_until_parked();

    let checked_before_unregister = Arc::new(AtomicBool::new(false));
    {
        let mut hook_cx = cx.clone();
        let store_for_hook = store.clone();
        let forbid_fg = Arc::clone(&forbid_foreground);
        let checked = Arc::clone(&checked_before_unregister);
        push_after_call_hook(
            |req| matches!(req, Request::window_snapshot {}),
            move || {
                forbid_fg.store(true, Ordering::Release);
                delayed
                    .update(&mut hook_cx, |root, window, cx| {
                        root.prepare_window_close(window, cx);
                        window.remove_window();
                    })
                    .unwrap();
                forbid_fg.store(false, Ordering::Release);

                let check_cx = hook_cx.clone();
                push_before_call_hook(
                    |req| {
                        matches!(
                            req,
                            Request::unregister { label } if label == "win-delayed-snap"
                        )
                    },
                    move || {
                        store_for_hook.read_with(&check_cx, |store, _| {
                                assert!(
                                    !store
                                        .window_entries
                                        .iter()
                                        .any(|entry| entry.label == "win-delayed-snap"),
                                    "delayed snapshot from before close must not reintroduce closed label while unregister is pending"
                                );
                            });
                        survivor
                                .read_with(&check_cx, |root, _| {
                                    assert!(
                                        !root.chat_secondaries.contains_key("chat-close-1377"),
                                        "survivor must remain primary while unregister batch is held"
                                    );
                                    assert!(
                                        root.attached.contains_key("chat-close-1377"),
                                        "survivor must keep terminal attached while unregister batch is held"
                                    );
                                })
                                .unwrap();
                        checked.store(true, Ordering::Release);
                    },
                );
            },
        );
    }
    delayed
        .update(&mut cx, |root, window, cx| {
            window.activate_window();
            root.sync_window_activation(window, cx);
        })
        .unwrap();
    cx.run_until_parked();

    assert!(checked_before_unregister.load(Ordering::Acquire));
    assert!(!core
        .windows
        .snapshot()
        .iter()
        .any(|entry| entry.label == "win-delayed-snap"));
    core.sessions.forget_session_state("chat-close-1377");
}

#[cfg(unix)]
#[test]
fn runtime_refresh_serializes_skill_and_command_reconciliation_with_user_toggles_1377() {
    use gpui::TestAppContext;
    use runner_core::protocol::{agent_skill, Request};
    use runner_daemon::{db, session, shell_path};
    use std::os::unix::fs::PermissionsExt as _;
    use std::sync::RwLock;

    let _hooks = HookGuard;
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let app_data = temp.path().join("app-data");
    let local_bin = home.join(".local/bin");
    let system_bin = temp.path().join("system-bin");
    std::fs::create_dir_all(&local_bin).unwrap();
    std::fs::create_dir_all(&system_bin).unwrap();

    let fake_codex = local_bin.join("codex");
    std::fs::write(&fake_codex, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&fake_codex, std::fs::Permissions::from_mode(0o755)).unwrap();

    let sidecar = agent_skill::sidecar_path(&app_data);
    std::fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
    std::fs::write(&sidecar, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o755)).unwrap();

    let login_env = shell_path::LoginShellEnv {
        path: Some(local_bin.display().to_string()),
        ..Default::default()
    };
    let env = Arc::new(RwLock::new(login_env.clone()));
    let mut disc = shell_path::DiscoveryState::startup(None, None);
    disc.checking = false;
    disc.result = Some(shell_path::DiscoveryResult {
        shell: None,
        outcome: runner_core::protocol::DiscoveryOutcome::Ok,
        duration_ms: 0,
        env: login_env,
    });
    let discovery = Arc::new(RwLock::new(disc));
    let core = crate::test_support::core(
        Arc::new(db::open_fixture_pool(&app_data.join("runner.db")).unwrap()),
        app_data.clone(),
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
            core.clone(),
            Some(home.clone()),
            Some(CommandInstallSupport::isolated(system_bin)),
            temp.path().join("settings.json"),
            AppSettings::default(),
            None,
            cx,
        )
    });
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        cx.set_global(GlobalAppStore(store.clone()));
        cx.set_global(crate::WindowLayoutCheckpoint::default());
        cx.set_global(runner_app::lifecycle::QuitState::default());
        #[cfg(target_os = "macos")]
        let updater = cx.new(|cx| crate::Updater::new(false, cx));
        #[cfg(target_os = "linux")]
        let updater = cx.new(|cx| crate::Updater::new(false, temp.path().join("updates"), cx));
        cx.set_global(crate::GlobalUpdater(updater));
        crate::notifications::install(cx);
    });
    let win = cx.add_window(|window, cx| {
        crate::surfaces::root::NativeRoot::new(
            "win-runtime-1377".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    cx.run_until_parked();

    let skill_dir = home
        .join(".agents/skills")
        .join(agent_skill::skill_name(cfg!(debug_assertions)));
    let cmd_link = local_bin.join(if cfg!(debug_assertions) {
        "runner-dev"
    } else {
        "runner"
    });
    assert!(skill_dir.join("SKILL.md").exists());
    assert!(cmd_link.exists());

    let forbid_foreground = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(Mutex::new(Vec::new()));
    store.update(&mut cx, |store, _| {
        store.client = DaemonClient::new(Arc::new(HookedTransport {
            inner: runner_daemon::daemon::InProcessTransport(core.clone()),
            forbid_foreground: Arc::clone(&forbid_foreground),
            calls: Arc::clone(&calls),
        }));
    });

    // 1. Start Runtimes refresh with runner_skill_enabled=true; while runtime_catalog is in
    // flight, user disables the skill. Refresh must NOT reinstall the removed skill files.
    {
        let mut hook_cx = cx.clone();
        let forbid_fg = Arc::clone(&forbid_foreground);
        push_after_call_hook(
            |req| matches!(req, Request::runtime_catalog {}),
            move || {
                forbid_fg.store(true, Ordering::Release);
                win.update(&mut hook_cx, |root, _, cx| {
                    root.set_runner_skill_enabled(false, cx);
                })
                .unwrap();
                forbid_fg.store(false, Ordering::Release);
            },
        );
    }
    store.update(&mut cx, |store, cx| {
        store.refresh(StoreRefreshKind::Runtimes, cx);
    });
    cx.run_until_parked();

    assert!(
        !skill_dir.exists(),
        "stale Runtimes refresh must not reinstall skill after user disabled it"
    );
    store.read_with(&cx, |store, _| {
        assert!(!store.settings.runner_skill_enabled);
    });

    // 2. Reset initialized_command_install=false so Runtimes refresh attempts default install;
    // while discovery_snapshot is in flight, user explicitly clicks Uninstall. Default install
    // must be suppressed and the command symlink must end absent on disk.
    store.update(&mut cx, |store, _| {
        store.settings.initialized_command_install = false;
        store
            .command_install_initialized
            .store(false, Ordering::Release);
    });
    assert!(cmd_link.exists());
    {
        let mut hook_cx = cx.clone();
        let forbid_fg = Arc::clone(&forbid_foreground);
        push_after_call_hook(
            |req| matches!(req, Request::discovery_snapshot {}),
            move || {
                forbid_fg.store(true, Ordering::Release);
                win.update(&mut hook_cx, |root, _, cx| {
                    root.uninstall_runner_command(cx);
                })
                .unwrap();
                forbid_fg.store(false, Ordering::Release);
            },
        );
    }
    store.update(&mut cx, |store, cx| {
        store.refresh(StoreRefreshKind::Runtimes, cx);
    });
    cx.run_until_parked();

    assert!(
        !cmd_link.exists(),
        "explicit command uninstall must win over concurrent Runtimes default install"
    );
    store.read_with(&cx, |store, _| {
        assert!(store.settings.initialized_command_install);
        assert_ne!(
            store
                .runner_command_status()
                .map(|status| status.state.clone()),
            Some(runner_core::command_install::RunnerCommandState::Installed)
        );
    });

    // 3. Toggling enabled_agents / disabled_agents through update_settings while background
    // reconcile locks are held must return immediately on the foreground thread with zero
    // synchronous RPCs or lock waits, then reconcile in the background once locks release.
    let (skill_lock, cmd_lock) = store.read_with(&cx, |store, _| {
        (
            Arc::clone(&store.skill_reconcile_lock),
            Arc::clone(&store.command_reconcile_lock),
        )
    });
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _skill_guard = skill_lock.lock().unwrap();
        let _cmd_guard = cmd_lock.lock().unwrap();
        locked_tx.send(()).unwrap();
        release_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
    });
    locked_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();

    calls.lock().unwrap().clear();
    forbid_foreground.store(true, Ordering::Release);
    store.update(&mut cx, |store, cx| {
        assert!(store.update_settings(
            |settings| {
                settings.disabled_agents.insert("codex".into());
                settings.runner_skill_enabled = true;
                settings.initialized_command_install = false;
                true
            },
            true,
            cx,
        ));
        assert!(store.update_settings(
            |settings| {
                settings.disabled_agents.remove("codex");
                settings.enabled_agents.insert("codex".into());
                true
            },
            true,
            cx,
        ));
    });
    forbid_foreground.store(false, Ordering::Release);
    assert!(
        calls.lock().unwrap().is_empty(),
        "update_settings agent toggles must not issue foreground RPCs"
    );

    release_tx.send(()).unwrap();
    holder.join().unwrap();
    cx.run_until_parked();

    assert!(
        skill_dir.join("SKILL.md").exists(),
        "background Runtimes refresh from update_settings must reconcile skill defaults"
    );
    assert!(
        cmd_link.exists(),
        "background Runtimes refresh from update_settings must reconcile command default"
    );
}

#[test]
fn delayed_node_viewed_preserves_concurrent_layout_and_unrelated_node_refresh_1377() {
    use gpui::{TestAppContext, VisualTestContext};
    use runner_core::protocol::Request;

    let _hooks = HookGuard;
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let core = store.read_with(&cx, |store, _| store.test_core.clone());

    let (tab_a, tab_b) = {
        let conn = core.db.get().unwrap();
        conn.execute_batch(
            "INSERT INTO sessions (id, status, agent_runtime, agent_command)
                 VALUES ('chat-na1', 'stopped', 'codex', 'codex'),
                        ('chat-na2', 'stopped', 'codex', 'codex'),
                        ('chat-nb', 'stopped', 'codex', 'codex'),
                        ('chat-nc', 'stopped', 'codex', 'codex');",
        )
        .unwrap();
        let layout_a = crate::pane_layout::PaneLayout::single(Some("chat-na1"), &[]);
        let layout_b = crate::pane_layout::PaneLayout::single(Some("chat-nb"), &[]);
        let a = runner_daemon::repo::node::create_tab(
            &conn,
            None,
            "Tab A",
            0,
            &layout_a.serialize().unwrap(),
        )
        .unwrap()
        .id;
        let b = runner_daemon::repo::node::create_tab(
            &conn,
            None,
            "Tab B",
            1,
            &layout_b.serialize().unwrap(),
        )
        .unwrap()
        .id;
        runner_daemon::repo::node::record_completion(&conn, &a, false, chrono::Utc::now()).unwrap();
        (a, b)
    };

    store.update(&mut cx, |store, cx| {
        store.refresh(StoreRefreshKind::All, cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        crate::notifications::install(cx);
    });

    core.windows.register("win-nodes");
    let host = cx.add_window(|window, cx| {
        let mut root = crate::surfaces::root::NativeRoot::new(
            "win-nodes".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.apply_tab_rows(cx);
        assert!(root.tabs.activate(&tab_a));
        root.route = crate::AppRoute::Chat;
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();

    let forbid_foreground = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(Mutex::new(Vec::new()));
    store.update(&mut visual, |store, _| {
        store.client = DaemonClient::new(Arc::new(HookedTransport {
            inner: runner_daemon::daemon::InProcessTransport(core.clone()),
            forbid_foreground: Arc::clone(&forbid_foreground),
            calls: Arc::clone(&calls),
        }));
    });

    // Case 1: Start Nodes refresh, and while node_list is in flight (after reading DB with
    // Tab C + renamed Tab B + split Tab A, before applying on foreground), apply a viewed
    // mark on Tab A. Neither Tab C/Tab B updates nor Tab A's newer viewed timestamp may be lost,
    // with NO extra daemon event emitted.
    let mut split_a = crate::pane_layout::PaneLayout::single(Some("chat-na1"), &[]);
    let second_pane = split_a
        .split("p1", crate::pane_layout::SplitOrientation::Row)
        .unwrap();
    split_a.assign_session(&second_pane, "chat-na2").unwrap();
    let split_a_json = split_a.serialize().unwrap();
    let tab_c = {
        let conn = core.db.get().unwrap();
        runner_daemon::repo::node::set_layout(&conn, &tab_a, &split_a_json).unwrap();
        runner_daemon::repo::node::rename(&conn, &tab_b, "Tab B Updated").unwrap();
        let layout_c = crate::pane_layout::PaneLayout::single(Some("chat-nc"), &[]);
        runner_daemon::repo::node::create_tab(
            &conn,
            None,
            "Tab C",
            2,
            &layout_c.serialize().unwrap(),
        )
        .unwrap()
        .id
    };

    let newer_viewed = "2099-01-01T00:00:00Z".to_owned();
    {
        let mut hook_visual = visual.clone();
        let store_for_hook = store.clone();
        let tab_a_for_hook = tab_a.clone();
        let viewed_for_hook = newer_viewed.clone();
        push_after_call_hook(
            |req| matches!(req, Request::node_list {}),
            move || {
                store_for_hook.update(&mut hook_visual, |store, store_cx| {
                    store.apply_node_viewed(&tab_a_for_hook, Some(viewed_for_hook), store_cx);
                });
            },
        );
    }
    store.update(&mut visual, |store, cx| {
        store.refresh(StoreRefreshKind::Nodes, cx);
    });
    visual.run_until_parked();

    store.read_with(&visual, |store, _| {
        let row_a = store.nodes.iter().find(|n| n.id == tab_a).unwrap();
        assert_eq!(row_a.layout.as_deref(), Some(split_a_json.as_str()));
        assert_eq!(row_a.last_viewed_at.as_deref(), Some(newer_viewed.as_str()));
        let row_b = store.nodes.iter().find(|n| n.id == tab_b).unwrap();
        assert_eq!(row_b.name.as_deref(), Some("Tab B Updated"));
        assert!(
            store.nodes.iter().any(|n| n.id == tab_c),
            "unrelated Tab C creation must survive concurrent viewed update without extra event"
        );
    });

    // Case 2: Delayed node_mark_viewed RPC returning an older NodeRow (with single-pane layout)
    // after store already holds a newer layout must NOT overwrite the newer layout in store.nodes.
    let newer_layout_json = {
        let mut layout = split_a.clone();
        let third = layout
            .split(&second_pane, crate::pane_layout::SplitOrientation::Column)
            .unwrap();
        layout.assign_session(&third, "chat-nc").unwrap();
        layout.serialize().unwrap()
    };
    {
        let mut hook_visual = visual.clone();
        let store_for_hook = store.clone();
        let core_for_hook = core.clone();
        let tab_a_for_hook = tab_a.clone();
        let layout_for_hook = newer_layout_json.clone();
        push_after_call_hook(
            move |req| matches!(req, Request::node_mark_viewed { id, .. } if id == &tab_a_for_hook),
            move || {
                let conn = core_for_hook.db.get().unwrap();
                runner_daemon::repo::node::set_layout(&conn, &tab_a, &layout_for_hook).unwrap();
                let fresh_rows = runner_daemon::repo::node::list(&conn).unwrap();
                store_for_hook.update(&mut hook_visual, |store, store_cx| {
                    store.replace_nodes(fresh_rows, store_cx);
                });
            },
        );
    }
    host.update(&mut visual, |root, window, cx| {
        root.mark_active_tab_viewed(window, cx);
    })
    .unwrap();
    visual.run_until_parked();

    store.read_with(&visual, |store, _| {
        let row_a = store
            .nodes
            .iter()
            .find(|n| n.name.as_deref() == Some("Tab A"))
            .unwrap();
        assert_eq!(
            row_a.layout.as_deref(),
            Some(newer_layout_json.as_str()),
            "stale NodeRow returned by delayed node_mark_viewed must not overwrite newer layout"
        );
    });
}

#[test]
fn shell_exit_cleans_up_standalone_fallback_and_split_without_not_found_banner_1377() {
    use gpui::TestAppContext;
    use runner_core::pane_layout::{PaneLayout, SplitOrientation};

    let _hooks = HookGuard;
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let core = store.read_with(&cx, |store, _| store.test_core.clone());

    let events: Arc<dyn runner_daemon::session::manager::SessionEvents> =
        Arc::new(core.session_events());
    let (fallback_tab_id, standalone_tab_id) = {
        let conn = core.db.get().unwrap();
        conn.execute_batch(
            "INSERT INTO sessions (id, status, agent_runtime, agent_command)
                 VALUES ('fallback-chat', 'running', 'codex', 'codex'),
                        ('split-shell', 'running', 'shell', '/bin/sh'),
                        ('standalone-shell', 'running', 'shell', '/bin/sh');",
        )
        .unwrap();
        drop(conn);
        for id in ["fallback-chat", "split-shell", "standalone-shell"] {
            core.sessions
                .prepare_unlisted_terminal(id, (80, 24), &core.db, &events)
                .unwrap();
        }

        let mut fallback = PaneLayout::single(Some("fallback-chat"), &["fallback-chat".into()]);
        fallback.id = "01ARZ3NDEKTSV4RRFFQ69G5FB1".into();
        let split_pane = fallback.split("p1", SplitOrientation::Row).unwrap();
        fallback.assign_session(&split_pane, "split-shell").unwrap();
        let fallback_row =
            runner_daemon::ops::node::node_tab_upsert(&core, fallback.upsert_input().unwrap())
                .unwrap();

        let mut standalone = PaneLayout::single(Some("standalone-shell"), &[]);
        standalone.id = "01ARZ3NDEKTSV4RRFFQ69G5FB2".into();
        let standalone_row =
            runner_daemon::ops::node::node_tab_upsert(&core, standalone.upsert_input().unwrap())
                .unwrap();
        (fallback_row.id, standalone_row.id)
    };

    store.update(&mut cx, |store, cx| {
        store.refresh_sessions_inner();
        store.refresh_nodes(cx).unwrap();
        for id in ["fallback-chat", "split-shell", "standalone-shell"] {
            store.bridge.attach(id).unwrap().test_stop_frames();
        }
    });
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        crate::notifications::install(cx);
    });

    let win = cx.add_window(|window, cx| {
        let mut root = crate::surfaces::root::NativeRoot::new(
            "win-shell-exit-1377".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.apply_tab_rows(cx);
        assert!(root.tabs.activate(&standalone_tab_id));
        root.route = crate::AppRoute::Chat;
        root.ensure_active_tab_attached(window, cx).unwrap();
        root
    });
    cx.run_until_parked();

    let forbid_foreground = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(Mutex::new(Vec::new()));
    store.update(&mut cx, |store, _| {
        store.client = DaemonClient::new(Arc::new(HookedTransport {
            inner: runner_daemon::daemon::InProcessTransport(core.clone()),
            forbid_foreground: Arc::clone(&forbid_foreground),
            calls: Arc::clone(&calls),
        }));
    });

    // 1. Standalone shell exits with a fallback split tab: no "direct chat not found" banner,
    // zero foreground RPCs on session/exit, and both panes of the fallback tab attach immediately.
    {
        let conn = core.db.get().unwrap();
        conn.execute("DELETE FROM sessions WHERE id = 'standalone-shell'", [])
            .unwrap();
        conn.execute("DELETE FROM nodes WHERE id = ?1", [&standalone_tab_id])
            .unwrap();
    }
    forbid_foreground.store(true, Ordering::Release);
    win.update(&mut cx, |root, window, cx| {
        root.handle_chat_lifecycle_event(
            runner_core::protocol::ClientEvent {
                name: "session/exit".into(),
                payload: serde_json::json!({ "session_id": "standalone-shell", "exit_code": 0 }),
            },
            window,
            cx,
        );
        // Even if sync_subject_ownership runs before async Nodes refresh applies, no error fires.
        root.sync_subject_ownership(window, cx);
        assert!(root.chat_error.is_none(), "{:?}", root.chat_error);
    })
    .unwrap();
    forbid_foreground.store(false, Ordering::Release);
    cx.run_until_parked();

    win.read_with(&cx, |root, _| {
        assert!(root.chat_error.is_none(), "{:?}", root.chat_error);
        assert_eq!(root.tabs.active_tab_id(), Some(fallback_tab_id.as_str()));
        assert!(
            root.attached.contains_key("fallback-chat")
                && root.attached.contains_key("split-shell"),
            "fallback tab panes must attach immediately when standalone shell tab exits"
        );
    })
    .unwrap();

    // 2. Split-pane shell exits: sibling chat pane stays attached and chat_error stays None.
    {
        let mut single_chat = PaneLayout::single(Some("fallback-chat"), &["fallback-chat".into()]);
        single_chat.id = fallback_tab_id.clone();
        runner_daemon::ops::node::node_tab_upsert(&core, single_chat.upsert_input().unwrap())
            .unwrap();
        let conn = core.db.get().unwrap();
        conn.execute("DELETE FROM sessions WHERE id = 'split-shell'", [])
            .unwrap();
    }
    forbid_foreground.store(true, Ordering::Release);
    win.update(&mut cx, |root, window, cx| {
        root.handle_chat_lifecycle_event(
            runner_core::protocol::ClientEvent {
                name: "session/exit".into(),
                payload: serde_json::json!({ "session_id": "split-shell", "exit_code": 0 }),
            },
            window,
            cx,
        );
        root.sync_subject_ownership(window, cx);
        assert!(root.chat_error.is_none(), "{:?}", root.chat_error);
    })
    .unwrap();
    forbid_foreground.store(false, Ordering::Release);
    cx.run_until_parked();

    win.read_with(&cx, |root, _| {
        assert!(root.chat_error.is_none(), "{:?}", root.chat_error);
        assert!(!root.attached.contains_key("split-shell"));
        assert!(root.attached.contains_key("fallback-chat"));
    })
    .unwrap();

    // 3. Standalone shell exits as the only remaining tab -> clean Launcher transition, no error.
    let last_shell_tab = {
        let conn = core.db.get().unwrap();
        conn.execute("DELETE FROM sessions WHERE id = 'fallback-chat'", [])
            .unwrap();
        conn.execute("DELETE FROM nodes WHERE id = ?1", [&fallback_tab_id])
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, status, agent_runtime, agent_command)
                 VALUES ('last-shell', 'running', 'shell', '/bin/sh')",
            [],
        )
        .unwrap();
        drop(conn);
        core.sessions
            .prepare_unlisted_terminal("last-shell", (80, 24), &core.db, &events)
            .unwrap();
        let mut layout = PaneLayout::single(Some("last-shell"), &[]);
        layout.id = "01ARZ3NDEKTSV4RRFFQ69G5FB3".into();
        runner_daemon::ops::node::node_tab_upsert(&core, layout.upsert_input().unwrap())
            .unwrap()
            .id
    };
    store.update(&mut cx, |store, _| {
        store
            .bridge
            .attach("last-shell")
            .unwrap()
            .test_stop_frames();
    });
    store.update(&mut cx, |store, cx| {
        store.refresh(StoreRefreshKind::All, cx);
    });
    cx.run_until_parked();
    win.update(&mut cx, |root, window, cx| {
        assert!(root.tabs.activate(&last_shell_tab));
        root.route = crate::AppRoute::Chat;
        root.ensure_active_tab_attached(window, cx).unwrap();
    })
    .unwrap();
    cx.run_until_parked();

    {
        let conn = core.db.get().unwrap();
        conn.execute("DELETE FROM sessions WHERE id = 'last-shell'", [])
            .unwrap();
        conn.execute("DELETE FROM nodes WHERE id = ?1", [&last_shell_tab])
            .unwrap();
    }
    win.update(&mut cx, |root, window, cx| {
        root.handle_chat_lifecycle_event(
            runner_core::protocol::ClientEvent {
                name: "session/exit".into(),
                payload: serde_json::json!({ "session_id": "last-shell", "exit_code": 0 }),
            },
            window,
            cx,
        );
        root.sync_subject_ownership(window, cx);
    })
    .unwrap();
    cx.run_until_parked();

    win.read_with(&cx, |root, _| {
        assert!(root.chat_error.is_none(), "{:?}", root.chat_error);
        assert!(root.tabs.active().is_none());
        assert!(crate::surfaces::panes::start_surface_visible(
            root.tabs.active()
        ));
        assert!(root.attached.is_empty());
    })
    .unwrap();

    for id in [
        "fallback-chat",
        "split-shell",
        "standalone-shell",
        "last-shell",
    ] {
        core.sessions.forget_session_state(id);
    }
}
