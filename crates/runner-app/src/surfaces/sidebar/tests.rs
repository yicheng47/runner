use super::archive::archive_session_plan;
use super::archive::pane_close_after_archive;
use super::archive::take_pending_pane_closes;
use super::elements::chat_tab_row_active;
use super::elements::command_held_alone;
use super::elements::default_session_label_parts;
use super::elements::other_modifiers_held;
use super::elements::sidebar_fork_menu_target;
use super::elements::sidebar_icon_color;
use super::elements::tab_label_live;
use super::menus::mission_menu_entries;
use super::menus::project_create_menu_entries;
use super::menus::project_menu_entries;
use super::menus::sidebar_create_menu_entries;
use super::menus::sidebar_tab_icon;
use super::menus::tab_menu_entries;
use super::rows_render::project_header_icon;
use super::view::sidebar_scroll_container;
use super::view::sidebar_scroll_frame;
use super::*;
use crate::surfaces::sidebar_logic::DropKind;
use gpui::{
    prelude::*, size, Context, Render, ScrollHandle, TestAppContext, VisualTestContext, Window,
};
use runner_core::protocol::node::NodeRow;
use runner_core::protocol::ClientEvent as AppEvent;

struct SidebarRenameTest {
    sidebar: Entity<Sidebar>,
    input: Entity<TextField>,
}

impl Render for SidebarRenameTest {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.input.clone())
            .child(
                div()
                    .id("outside-rename")
                    .h(px(40.))
                    .w_full()
                    .on_click(|_, window, cx| window.blur(cx))
                    .debug_selector(|| "OUTSIDE_RENAME".into()),
            )
    }
}

/// One completed mission, "Original mission", with its sidebar node.
fn seeded_store(
    cx: &mut TestAppContext,
    temp: &std::path::Path,
) -> (Arc<runner_daemon::db::DbPool>, Entity<AppStore>) {
    use runner_daemon::{db, session, shell_path};
    use std::sync::RwLock;

    let pool = Arc::new(db::open_pool(&temp.join("runner.db")).unwrap());
    let conn = pool.get().unwrap();
    conn.execute_batch(
        "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('crew', 'Crew', '2026-09-06T00:00:00Z', '2026-09-06T00:00:00Z');
             INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('mission', 'crew', 'Original mission', 'completed', '2026-09-06T00:00:00Z');",
    )
    .unwrap();
    runner_daemon::repo::node::ensure_mission_node(&conn, "mission", None).unwrap();
    drop(conn);
    let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
    let runtime_discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
    let core = crate::test_support::core(
        pool.clone(),
        temp.to_owned(),
        session::SessionManager::new(
            runtime_shell_env.clone(),
            runtime_discovery.clone(),
            Arc::new(session::pty_runtime::PtyRuntime::new()),
        ),
        runtime_shell_env,
        runtime_discovery,
    );
    let store = cx.new(|cx| {
        AppStore::new(
            core,
            None,
            None,
            temp.join("settings.json"),
            AppSettings::default(),
            None,
            cx,
        )
    });
    (pool, store)
}

#[test]
fn paused_mission_hides_unread_without_acknowledging_sessions() {
    use runner_app::ui::agent_status::StatusRollup;
    use runner_core::protocol::model::MissionStatus;
    use runner_core::protocol::status::{Activity, AgentStatus, Lifecycle};

    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (_, store) = seeded_store(&mut cx, temp.path());
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    store.update(&mut cx, |store, _| {
        let summary = &mut store.missions[0];
        summary.mission.status = MissionStatus::Running;
        summary.any_session_live = false;
        summary.session_statuses = ["coder", "reviewer"]
            .into_iter()
            .map(|id| {
                (
                    id.into(),
                    AgentStatus {
                        lifecycle: Lifecycle::Stopped,
                        unread_since: Some(1),
                        ..Default::default()
                    },
                )
            })
            .collect();
    });
    let tabs = cx.update(|cx| TabSet::from_rows(&store.read(cx).nodes));
    sidebar.update(&mut cx, |sidebar, cx| {
        let summary = &store.read(cx).missions[0];
        let rollup = sidebar.mission_status_rollup(summary);
        assert_eq!(rollup.priority_value(), 0);
        assert_eq!(rollup.target(), None);
        assert_eq!(rollup.tooltip(), "");
        assert!(summary
            .session_statuses
            .iter()
            .all(|(_, status)| status.unread_since == Some(1)));
        let rows = sidebar.resolved_sidebar_rows_for_tabs(&tabs, cx);
        assert_eq!(rows[0].attention(), AttentionState::None);

        let mut collapsed = rollup;
        collapsed.entries.push((
            "other-chat".into(),
            AgentStatus {
                unread_since: Some(2),
                ..Default::default()
            },
        ));
        assert_eq!(collapsed.priority_value(), 2);
        assert_eq!(collapsed.target(), Some("other-chat"));
        assert_eq!(collapsed.tooltip(), "1 unread response");
    });

    store.update(&mut cx, |store, _| {
        let summary = &mut store.missions[0];
        summary.session_statuses[0].1.unread_since = None;
        summary.any_session_live = true;
        summary.session_statuses[1].1.lifecycle = Lifecycle::Running;
        summary.session_statuses[1].1.observation.activity = Activity::Ready;
    });
    sidebar.update(&mut cx, |sidebar, cx| {
        let rollup = sidebar.mission_status_rollup(&store.read(cx).missions[0]);
        assert_eq!(rollup.priority_value(), 2);
        assert_eq!(rollup.target(), Some("reviewer"));
        assert_eq!(rollup.tooltip(), "1 unread response");
        let rows = sidebar.resolved_sidebar_rows_for_tabs(&tabs, cx);
        assert_eq!(rows[0].attention(), AttentionState::Unread);
        assert_eq!(StatusRollup::priority(&rollup.entries[0].1), 0);
    });
}

#[test]
fn paused_mission_preserves_error_and_approval_attention() {
    use runner_core::protocol::model::MissionStatus;
    use runner_core::protocol::status::{AgentStatus, HumanInteraction, Lifecycle, WaitReason};

    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (_, store) = seeded_store(&mut cx, temp.path());
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    sidebar.update(&mut cx, |sidebar, cx| {
        let mut summary = store.read(cx).missions[0].clone();
        summary.mission.status = MissionStatus::Running;
        summary.any_session_live = false;
        let mut status = AgentStatus {
            lifecycle: Lifecycle::Running,
            unread_since: Some(1),
            ..Default::default()
        };
        status.observation.interactions.push(HumanInteraction {
            id: "approval".into(),
            reason: WaitReason::Approval,
            owners: vec![],
            since: 1,
        });
        summary.session_statuses = vec![("reviewer".into(), status)];
        let approval = sidebar.mission_status_rollup(&summary);
        assert_eq!(approval.priority_value(), 4);
        assert_eq!(approval.target(), Some("reviewer"));
        assert_eq!(approval.tooltip(), "1 approval needed");

        summary.session_statuses[0].1.lifecycle = Lifecycle::Error;
        summary.session_statuses[0].1.error_since = Some(1);
        let error = sidebar.mission_status_rollup(&summary);
        assert_eq!(error.priority_value(), 5);
        assert_eq!(error.target(), Some("reviewer"));
        assert_eq!(error.tooltip(), "1 error");
        assert_eq!(summary.session_statuses[0].1.unread_since, Some(1));

        summary.session_statuses[0].1.error_since = None;
        summary.session_statuses[0].1.failed_since = Some(1);
        let failure = sidebar.mission_status_rollup(&summary);
        assert_eq!(failure.priority_value(), 5);
        assert_eq!(failure.tooltip(), "1 response failed");
    });
}

#[test]
fn sidebar_shortcuts_skip_empty_and_drawer_only_tabs() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (pool, store) = seeded_store(&mut cx, temp.path());
    let conn = pool.get().unwrap();
    let rows = ["a", "b", "empty", "drawer-only", "c"]
        .into_iter()
        .enumerate()
        .map(|(position, name)| {
            let mut layout = PaneLayout::single(None, &[]);
            if name == "drawer-only" {
                conn.execute("INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES ('drawer', 'stopped', 'shell', '/bin/sh')", []).unwrap();
                layout.add_drawer_shell("drawer".into());
            } else if name != "empty" {
                conn.execute("INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES (?1, 'stopped', 'codex', 'codex')", [name]).unwrap();
                layout.assign_session("p1", name).unwrap();
            }
            runner_daemon::repo::node::create_tab(
                &conn, None, name, position as i64, &layout.serialize().unwrap(),
            ).unwrap()
        })
        .collect::<Vec<_>>();
    drop(conn);
    store.update(&mut cx, |store, cx| {
        store.refresh_sessions(cx);
        store.refresh_nodes(cx).unwrap();
    });
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    let tabs = cx.update(|cx| TabSet::from_rows(&store.read(cx).nodes));
    sidebar.update(&mut cx, |sidebar, cx| {
        let visible = sidebar
            .resolved_sidebar_rows_for_tabs(&tabs, cx)
            .into_iter()
            .filter(|row| matches!(row, SidebarRow::Tab { .. }))
            .map(|row| row.node().id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            visible,
            [rows[0].id.clone(), rows[1].id.clone(), rows[4].id.clone()]
        );
        sidebar.refresh_shortcut_rows(&tabs, cx);
        for invisible in [&rows[2], &rows[3]] {
            assert!(!sidebar.tab_index_by_node.contains_key(&invisible.id));
        }
    });
}

#[test]
fn mission_rename_commits_when_the_field_loses_focus() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (pool, store) = seeded_store(&mut cx, temp.path());
    let host = cx.add_window(|window, cx| {
        let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store, None, cx));
        sidebar.update(cx, |sidebar, cx| {
            sidebar.begin_sidebar_rename(
                SidebarRenameTarget::Mission {
                    mission_id: "mission".into(),
                    original: "Original mission".into(),
                },
                "Original mission".into(),
                "Mission name".into(),
                window,
                cx,
            );
        });
        let input = sidebar.read(cx).rename.as_ref().unwrap().input.clone();
        SidebarRenameTest { sidebar, input }
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(host.into(), &cx);
    window.update(|window, _| window.activate_window());
    window.run_until_parked();
    host.update(&mut window, |host, window, cx| {
        host.input.update(cx, |field, cx| {
            field.set_text("Renamed mission", cx);
            assert!(field.focus_handle().is_focused(window));
        });
    })
    .unwrap();
    let outside = window.debug_bounds("OUTSIDE_RENAME").unwrap();
    window.simulate_click(outside.center(), gpui::Modifiers::default());
    window.run_until_parked();
    host.update(&mut window, |host, _, cx| {
        assert!(host.sidebar.read(cx).rename.is_none());
    })
    .unwrap();
    let title: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT title FROM missions WHERE id = 'mission'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(title, "Renamed mission");
}

#[test]
fn sidebar_archive_returns_to_start_only_after_success() {
    for (route, archive_id, expected_route) in [
        (
            AppRoute::Mission("mission".into()),
            "mission",
            AppRoute::Chat,
        ),
        (
            AppRoute::Mission("other".into()),
            "mission",
            AppRoute::Mission("other".into()),
        ),
        (AppRoute::Roles, "mission", AppRoute::Roles),
        (
            AppRoute::Mission("missing".into()),
            "missing",
            AppRoute::Mission("missing".into()),
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let core = store.read_with(&cx, |store, _| store.test_core.clone());
        let conn = core.db.get().unwrap();
        conn.execute_batch(
            "INSERT INTO missions (id, title, status, started_at) VALUES ('mission', 'Empty mission', 'running', '2026-10-15T00:00:00Z');
             INSERT INTO sessions (id, status, agent_runtime, agent_command) VALUES ('saved-chat', 'stopped', 'codex', 'codex');",
        ).unwrap();
        let layout = PaneLayout::single(Some("saved-chat"), &["saved-chat".into()]);
        let tab = runner_daemon::repo::node::create_tab(
            &conn,
            None,
            "Saved chat",
            0,
            &layout.serialize().unwrap(),
        )
        .unwrap();
        drop(conn);
        store.update(&mut cx, |store, cx| store.refresh_nodes(cx).unwrap());
        let host = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "archive-test".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.set_route(route.clone(), cx);
            root
        });
        host.update(&mut cx, |root, window, cx| {
            assert_eq!(root.tabs.active_tab_id(), Some(tab.id.as_str()));
            root.sidebar.update(cx, |sidebar, cx| {
                sidebar.handle_sidebar_menu_action(
                    SidebarMenuAction::ArchiveMission(archive_id.into()),
                    window,
                    cx,
                );
            });
            assert_eq!(root.route, route);
        })
        .unwrap();
        cx.run_until_parked();
        host.update(&mut cx, |root, window, cx| {
            assert_eq!(root.route, expected_route);
            let expected_tab = (expected_route != AppRoute::Chat).then_some(tab.id.as_str());
            assert_eq!(root.tabs.active_tab_id(), expected_tab);
            root.reload_tabs(cx).unwrap();
            assert_eq!(root.tabs.active_tab_id(), expected_tab);
            assert_eq!(root.tabs.tabs().len(), 1);
            assert!(root.sidebar.read(cx).archiving_missions.is_empty());
            window.remove_window();
        })
        .unwrap();
        let mission = runner_daemon::ops::mission::mission_get(&core, "mission").unwrap();
        assert_eq!(mission.archived_at.is_some(), archive_id == "mission");
    }
}

struct SidebarRowTest {
    sidebar: Entity<Sidebar>,
    _observe: gpui::Subscription,
}

impl Render for SidebarRowTest {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = self.sidebar.update(cx, |sidebar, cx| {
            let row = sidebar
                .resolved_sidebar_rows_for_tabs(&TabSet::default(), cx)
                .into_iter()
                .next()
                .unwrap();
            sidebar.render_sidebar_row(row, None, None, DropKind::Leaf, None, Vec::new(), cx)
        });
        div().w(px(260.)).child(row)
    }
}

#[test]
fn hovering_a_row_reveals_its_actions_in_a_frame_that_paints() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (_pool, store) = seeded_store(&mut cx, temp.path());
    let host = cx.add_window(|_, cx| {
        let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store, None, cx));
        SidebarRowTest {
            _observe: cx.observe(&sidebar, |_, _, cx| cx.notify()),
            sidebar,
        }
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(host.into(), &cx);
    window.run_until_parked();
    let row = window
        .debug_bounds("SIDEBAR_ROW sidebar-mission-mission")
        .unwrap();
    let hovered_row = |window: &mut VisualTestContext| {
        host.update(window, |host, _, cx| {
            host.sidebar.read(cx).hovered_row.clone()
        })
        .unwrap()
    };

    window.simulate_mouse_move(row.center(), None, gpui::Modifiers::default());
    window.run_until_parked();
    assert!(hovered_row(&mut window).is_some());

    window.simulate_mouse_move(
        row.bottom_right() + gpui::point(px(0.), px(100.)),
        None,
        gpui::Modifiers::default(),
    );
    window.run_until_parked();
    assert_eq!(hovered_row(&mut window), None);
}

fn direct_session(id: &str, runtime: &str, status: SessionStatus) -> DirectSessionEntry {
    DirectSessionEntry {
        claude_account: None,
        session_id: id.into(),
        project_id: None,
        role_id: None,
        handle: None,
        agent_runtime: runtime.into(),
        agent_command: runtime.into(),
        agent_model: None,
        agent_effort: None,
        display_name: runtime.into(),
        status,
        title: None,
        live_title: None,
        cwd: None,
        started_at: None,
        stopped_at: None,
        resumable: false,
        native_fork: matches!(runtime, "codex" | "claude-code" | "pi"),
        forkable: false,
        agent_session_key: None,
        pinned: false,
        archived_at: None,
    }
}

#[test]
fn session_titles_respect_manual_names_persistence_and_resets() {
    let mut entry = direct_session("chat", "codex", SessionStatus::Running);
    let default = default_session_label(&entry);
    assert_eq!(session_label_live(&entry, None), default);
    entry.live_title = Some("Cars".into());
    assert_eq!(session_label(&entry), "Cars");
    assert_eq!(
        session_label_live(&entry, Some("Electric cars")),
        "Electric cars"
    );
    assert_eq!(session_label_live(&entry, Some("")), "Cars");
    entry.title = Some("My chat".into());
    assert_eq!(session_label_live(&entry, Some("Electric cars")), "My chat");
    assert_eq!(session_label_live(&entry, Some("")), "My chat");
    entry.title = None;
    assert_eq!(
        session_label_live(&entry, Some("Electric cars")),
        "Electric cars"
    );
    entry.status = SessionStatus::Stopped;
    assert_eq!(session_label(&entry), "Cars");
}

#[test]
fn tab_titles_use_live_words_before_the_session_snapshot_refreshes() {
    let mut entry = direct_session("chat", "codex", SessionStatus::Running);
    entry.cwd = Some("/tmp/project".into());
    let layout = PaneLayout::single(Some("chat"), &[]);
    for (live, expected) in [
        (None, "codex"),
        (Some("project"), "codex"),
        (Some("Discuss cars | project"), "Discuss cars"),
    ] {
        assert_eq!(
            tab_label_live(&layout, &[SessionDisplay::local(&entry)], |_| live
                .map(str::to_owned)),
            expected
        );
    }
    entry.live_title = Some("Old topic".into());
    assert_eq!(
        tab_label_live(&layout, &[SessionDisplay::local(&entry)], |_| Some(
            "New topic".into()
        )),
        "New topic"
    );
    entry.title = Some("My chat".into());
    assert_eq!(
        tab_label_live(&layout, &[SessionDisplay::local(&entry)], |_| Some(
            "New topic".into()
        )),
        "My chat"
    );
}

#[test]
fn remote_role_labels_stay_consistent_in_pinned_project_active_rows_and_headers() {
    use crate::surfaces::remote_client::tests::{add_window, choose_host, fixture};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [
        crate::i18n::Locale::English,
        crate::i18n::Locale::SimplifiedChinese,
    ] {
        runner_app::i18n::testing::with_locale(locale, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, _) = fixture(&mut cx, temp.path());
            let mut data: runner_core::companion::Overview = serde_json::from_value(serde_json::json!({
                "projects":[{"id":"project","name":"Host project"}],
                "tabs":[
                    {"id":"pinned","project":"project","name":"","pinned":true,"panes":{"session":"one"},"shells":[]},
                    {"id":"project-tab","project":"project","name":"","pinned":false,"panes":{"session":"two"},"shells":[]},
                    {"id":"recent","project":null,"name":"","pinned":false,"panes":{"session":"three"},"shells":[]}
                ],
                "missions":[],
                "sessions":{
                    "one":{"kind":"chat","title":"@planner","runtime":"pi","status":"working","unread":false},
                    "two":{"kind":"chat","title":"@coder","runtime":"pi","status":"working","unread":false},
                    "three":{"kind":"chat","title":"Fix the parser","runtime":"pi","status":"working","unread":false}
                }
            })).unwrap();
            fake.set_overview(data.clone());
            let window = add_window(&mut cx, &store, temp.path(), "remote-role-labels");
            window.update(&mut cx, choose_host).unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            for (session_title, tab_name, expected) in [
                ("@planner", "", ["@planner", "@coder", "Fix the parser"]),
                (
                    "User session name",
                    "User tab name",
                    ["User session name", "User tab name", "Fix the parser"],
                ),
                ("", "", ["pi", "@coder", "Fix the parser"]),
            ] {
                data.sessions.get_mut("one").unwrap().title = session_title.into();
                data.tabs[1].name = tab_name.into();
                fake.emit(
                    "remote/overview",
                    serde_json::json!({"computer":"host","overview":data}),
                );
                visual.run_until_parked();
                for title in expected {
                    assert!(
                        visual
                            .debug_bounds(Box::leak(
                                format!("SIDEBAR_ITEM_TITLE:{title}").into_boxed_str()
                            ))
                            .is_some(),
                        "{title}"
                    );
                }
                assert!(visual
                    .debug_bounds(Box::leak(
                        format!("WORKSPACE_HEADER_TITLE:{}", expected[0]).into_boxed_str()
                    ))
                    .is_some());
                window
                    .read_with(&visual, |root, cx| {
                        let remote = root.remote_window.as_ref().unwrap();
                        let sidebar = remote.sidebar.read(cx);
                        let mut titles = sidebar
                            .resolved_active_rows(cx)
                            .iter()
                            .map(|row| row.title_and_icon(|_| None).0)
                            .collect::<Vec<_>>();
                        titles.sort();
                        let mut wanted = expected.map(str::to_owned);
                        wanted.sort();
                        assert_eq!(titles, wanted);
                        assert_eq!(
                            root.tab_label(root.front_page_layout().unwrap(), cx),
                            expected[0]
                        );
                    })
                    .unwrap();
                window
                    .update(&mut visual, |root, _, cx| {
                        root.remote_window
                            .as_ref()
                            .unwrap()
                            .sidebar
                            .update(cx, |sidebar, cx| sidebar.toggle_active_only(cx));
                    })
                    .unwrap();
                visual.run_until_parked();
                for id in ["pinned", "project-tab", "recent"] {
                    assert!(visual
                        .debug_bounds(Box::leak(format!("ACTIVE_TITLE {id}").into_boxed_str()))
                        .is_some());
                }
                window
                    .update(&mut visual, |root, _, cx| {
                        root.remote_window
                            .as_ref()
                            .unwrap()
                            .sidebar
                            .update(cx, |sidebar, cx| sidebar.toggle_active_only(cx));
                    })
                    .unwrap();
                visual.run_until_parked();
            }
        });
    }
}

#[test]
fn split_tab_titles_follow_layout_order_and_manual_names_instead_of_focus() {
    let first = direct_session("first", "codex", SessionStatus::Running);
    let second = direct_session("second", "claude-code", SessionStatus::Running);
    let sessions = [second, first]
        .iter()
        .map(SessionDisplay::local)
        .collect::<Vec<_>>();
    let live = |id: &str| Some(if id == "first" { "Cars" } else { "Planes" }.into());
    let mut layout = two_pane_layout("first", "second");
    for id in ["first", "second"] {
        assert!(layout.focus_session(id));
        assert_eq!(tab_label_live(&layout, &sessions, live), "Cars");
    }
    layout.name = Some("My workspace".into());
    assert_eq!(tab_label_live(&layout, &sessions, live), "My workspace");
    layout.name = None;
    layout.remove_session("first");
    assert_eq!(tab_label_live(&layout, &sessions, live), "Planes");
    layout.remove_session("second");
    assert_eq!(tab_label_live(&layout, &sessions, live), "Empty tab");
}

#[test]
fn agent_names_follow_manual_provider_and_default_order() {
    let mut entry = direct_session("chat", "codex", SessionStatus::Running);
    entry.cwd = Some("/Users/jason/repos/yicheng47".into());
    let default = default_session_label(&entry);
    for raw in ["Codex", "yicheng47", "⠋ Working | yicheng47"] {
        assert_eq!(session_label_live(&entry, Some(raw)), default);
    }
    entry.live_title = Some("yicheng47".into());
    assert_eq!(session_label(&entry), default);
    assert_eq!(
        session_label_live(&entry, Some("Ready | yicheng47")),
        default
    );
    assert_eq!(
        session_label_live(&entry, Some("Electric cars | yicheng47")),
        "Electric cars"
    );
    entry.live_title = Some("Electric cars | yicheng47".into());
    for raw in ["", "⠋ Working", "yicheng47"] {
        assert_eq!(session_label_live(&entry, Some(raw)), "Electric cars");
    }
    entry.title = Some("My cars".into());
    assert_eq!(session_label_live(&entry, Some("New topic")), "My cars");
    entry.title = None;
    entry.status = SessionStatus::Stopped;
    assert_eq!(session_label(&entry), "Electric cars");

    let shell = direct_session("shell", "shell", SessionStatus::Running);
    assert_eq!(session_label_live(&shell, Some("⠋ /tmp")), "⠋ /tmp");
    assert_eq!(
        session_label_live(&shell, Some("")),
        default_session_label(&shell)
    );
}

struct SidebarScrollLayoutTest {
    scroll: ScrollHandle,
    block_wrapper_scroll: ScrollHandle,
    constrained_section_scroll: ScrollHandle,
    short_scroll: ScrollHandle,
}

impl Render for SidebarScrollLayoutTest {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .child(sidebar_scroll_column(
                self.scroll.clone(),
                "test-sidebar-node-scroll",
                true,
                false,
                6,
                12,
            ))
            .child(sidebar_scroll_column(
                self.block_wrapper_scroll.clone(),
                "test-block-wrapper-sidebar-node-scroll",
                false,
                false,
                6,
                12,
            ))
            .child(sidebar_scroll_column(
                self.constrained_section_scroll.clone(),
                "test-constrained-section-sidebar-node-scroll",
                true,
                true,
                6,
                12,
            ))
            .child(sidebar_scroll_column(
                self.short_scroll.clone(),
                "test-short-sidebar-node-scroll",
                true,
                false,
                1,
                2,
            ))
    }
}

fn sidebar_scroll_column(
    scroll_handle: ScrollHandle,
    id: &'static str,
    flex_frame: bool,
    constrain_sections: bool,
    project_count: usize,
    chat_count: usize,
) -> AnyElement {
    let projects = div()
        .flex()
        .flex_col()
        .children((0..project_count).map(|_| div().h(px(40.)).flex_none()));
    let mut chat_scope = div()
        .flex_1()
        .flex()
        .flex_col()
        .children((0..chat_count).map(|_| div().h(px(28.)).flex_none()));
    if constrain_sections {
        chat_scope = chat_scope.min_h(rems(28. / 16.));
    }
    let chats_selector = format!("TEST_CHATS_{id}");
    let mut chats = div()
        .debug_selector(move || chats_selector.clone())
        .mt(px(20.))
        .flex_1()
        .flex()
        .flex_col()
        .child(div().h(px(24.)).flex_none())
        .child(chat_scope);
    if constrain_sections {
        chats = chats.min_h(px(0.));
    }
    let scroll = sidebar_scroll_container(id, &scroll_handle)
        .child(projects)
        .child(chats);
    let wrapper = if flex_frame {
        sidebar_scroll_frame().child(scroll)
    } else {
        div().relative().min_h(px(0.)).flex_1().child(scroll)
    };

    div()
        .w(px(240.))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            div()
                .min_h(px(0.))
                .flex_1()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(wrapper),
        )
        .into_any_element()
}

fn appended_event(signal: &str) -> AppEvent {
    AppEvent {
        name: "event/appended".into(),
        payload: serde_json::json!({ "event": { "type": signal } }),
    }
}

fn menu_labels(entries: &[(UiMenuItem, SidebarMenuAction)]) -> Vec<&str> {
    entries
        .iter()
        .map(|(item, _)| item.label.as_ref())
        .collect()
}

#[test]
fn workspace_starts_with_a_non_selectable_new_chat_action() {
    assert_eq!(
        WORKSPACE_ENTRIES,
        [
            WorkspaceEntry::NewTab,
            WorkspaceEntry::Role,
            WorkspaceEntry::Crew,
        ]
    );
    assert!(!WorkspaceEntry::NewTab.selectable());
    assert!(!WorkspaceEntry::NewTab.selected(&AppRoute::Roles));
    assert!(!WorkspaceEntry::NewTab.selected(&AppRoute::Crews));
    assert!(!WorkspaceEntry::NewTab.selected(&AppRoute::Settings));
    assert!(WorkspaceEntry::Role.selectable());
    assert!(WorkspaceEntry::Crew.selectable());
}

#[test]
fn shortcut_pills_use_the_platform_primary_modifier_alone() {
    let mut modifiers = gpui::Modifiers {
        control: cfg!(any(windows, target_os = "linux")),
        platform: cfg!(target_os = "macos"),
        ..Default::default()
    };
    assert!(command_held_alone(modifiers));
    assert!(!other_modifiers_held(modifiers));
    modifiers.shift = true;
    assert!(!command_held_alone(modifiers));
    assert!(other_modifiers_held(modifiers));
    assert!(!command_held_alone(gpui::Modifiers {
        control: cfg!(target_os = "macos"),
        platform: cfg!(any(windows, target_os = "linux")),
        ..Default::default()
    }));
}

#[test]
fn project_create_menu_uses_short_labels_and_project_targets() {
    let root_entries = sidebar_create_menu_entries(&keymap::KeymapOverrides::new());
    assert_eq!(
        menu_labels(&root_entries),
        ["New chat", "New mission", "New terminal"]
    );
    assert_eq!(
        root_entries
            .iter()
            .map(|(_, action)| action.clone())
            .collect::<Vec<_>>(),
        [
            SidebarMenuAction::NewChat(ProjectScope::Root),
            SidebarMenuAction::NewMission(ProjectScope::Root),
            SidebarMenuAction::NewTerminal(ProjectScope::Root),
        ]
    );

    let entries = project_create_menu_entries("project-1", &keymap::KeymapOverrides::new());
    assert_eq!(
        menu_labels(&entries),
        ["New chat", "New mission", "New terminal"]
    );
    assert_eq!(
        entries
            .iter()
            .map(|(_, action)| action.clone())
            .collect::<Vec<_>>(),
        [
            SidebarMenuAction::NewChat(ProjectScope::Project("project-1".into())),
            SidebarMenuAction::NewMission(ProjectScope::Project("project-1".into())),
            SidebarMenuAction::NewTerminal(ProjectScope::Project("project-1".into())),
        ]
    );

    let project_entries = project_menu_entries("project-1".into(), "Runner".into());
    assert_eq!(
        menu_labels(&project_entries),
        ["Rename project", "Delete project"]
    );
}

#[test]
fn tab_and_mission_menus_have_the_trimmed_item_lists() {
    let tab_entries = tab_menu_entries(
        "tab-1",
        false,
        "My tab".into(),
        None,
        false,
        None,
        false,
        vec!["session-1".into(), "session-2".into()],
        vec!["session-1".into(), "session-2".into()],
        None,
        &keymap::KeymapOverrides::new(),
    );
    assert_eq!(menu_labels(&tab_entries), ["Pin", "Rename tab", "Archive"]);
    assert!(tab_entries[2].0.destructive);

    let single_pane_entries = tab_menu_entries(
        "tab-1",
        false,
        "build".into(),
        Some("shell".into()),
        false,
        None,
        false,
        Vec::new(),
        vec!["shell".into()],
        Some("shell".into()),
        &keymap::KeymapOverrides::new(),
    );
    assert_eq!(
        single_pane_entries[1].1,
        SidebarMenuAction::Rename(SidebarRenameTarget::Tab {
            node_id: "tab-1".into(),
            original: "build".into(),
            session_id: Some("shell".into()),
        })
    );
    assert_eq!(
        tab_entries
            .iter()
            .map(|(_, action)| action.clone())
            .collect::<Vec<_>>(),
        [
            SidebarMenuAction::TogglePin {
                node_id: "tab-1".into(),
                pinned: false,
            },
            SidebarMenuAction::Rename(SidebarRenameTarget::Tab {
                node_id: "tab-1".into(),
                original: "My tab".into(),
                session_id: None,
            }),
            SidebarMenuAction::ArchiveTab {
                tab_id: "tab-1".into(),
                session_ids: vec!["session-1".into(), "session-2".into()],
            },
        ]
    );

    let multi_pane_entries = tab_menu_entries(
        "tab-1",
        false,
        "My tab".into(),
        None,
        true,
        None,
        false,
        vec!["session-1".into(), "session-2".into()],
        vec!["session-1".into(), "session-2".into()],
        None,
        &keymap::KeymapOverrides::new(),
    );
    assert_eq!(
        menu_labels(&multi_pane_entries),
        ["Pin", "Rename tab", "Archive"]
    );
    assert!(multi_pane_entries[2].0.destructive);
    assert!(multi_pane_entries[2].0.shortcut.is_none());

    let mixed_entries = tab_menu_entries(
        "tab-1",
        false,
        "Mixed tab".into(),
        None,
        true,
        None,
        false,
        vec!["chat-1".into()],
        vec!["chat-1".into(), "shell-1".into()],
        None,
        &keymap::KeymapOverrides::new(),
    );
    assert_eq!(
        mixed_entries[2].1,
        SidebarMenuAction::ArchiveTab {
            tab_id: "tab-1".into(),
            session_ids: vec!["chat-1".into(), "shell-1".into()],
        }
    );

    let terminal_only_entries = tab_menu_entries(
        "tab-1",
        false,
        "Terminal".into(),
        None,
        false,
        None,
        false,
        Vec::new(),
        vec!["shell-1".into()],
        Some("shell-1".into()),
        &keymap::KeymapOverrides::new(),
    );
    assert_eq!(
        menu_labels(&terminal_only_entries),
        ["Pin", "Rename tab", "Close terminal"]
    );
    assert!(terminal_only_entries[2].0.destructive);
    assert_eq!(
        terminal_only_entries[2].0.shortcut.as_deref(),
        Some(keymap::fixed_shortcut("cmd-w").as_str())
    );
    assert_eq!(
        terminal_only_entries[2].1,
        SidebarMenuAction::CloseTerminalTab {
            tab_id: "tab-1".into(),
            session_id: "shell-1".into(),
        }
    );
    for (runtime, path) in [
        (Some("shell"), "square-terminal.svg"),
        (Some("claude-code"), "claude.svg"),
        (Some("codex"), "openai.svg"),
        (Some("trae"), "trae.svg"),
        (Some("copilot"), "copilot.svg"),
        (Some("pi"), "pi.svg"),
        (Some("antigravity"), "antigravity-icon.png"),
        (Some("unknown"), "message-square.svg"),
        (None, "message-square.svg"),
    ] {
        assert_eq!(sidebar_tab_icon(1, runtime).asset_path(), path);
        for panes in [2, 3, 4] {
            assert_eq!(
                sidebar_tab_icon(panes, runtime).asset_path(),
                "columns-2.svg"
            );
        }
    }

    let mission_entries =
        mission_menu_entries("mission-node-1", false, "mission-1", "My mission".into());
    assert_eq!(menu_labels(&mission_entries), ["Pin", "Rename", "Archive"]);
    assert!(mission_entries[2].0.destructive);
    assert_eq!(
        mission_entries
            .iter()
            .map(|(_, action)| action.clone())
            .collect::<Vec<_>>(),
        [
            SidebarMenuAction::TogglePin {
                node_id: "mission-node-1".into(),
                pinned: false,
            },
            SidebarMenuAction::Rename(SidebarRenameTarget::Mission {
                mission_id: "mission-1".into(),
                original: "My mission".into(),
            }),
            SidebarMenuAction::ArchiveMission("mission-1".into()),
        ]
    );
}

#[test]
fn mixed_tab_archive_plan_and_confirmation_copy_are_explicit() {
    let sessions = [
        direct_session("shell-1", "shell", SessionStatus::Running),
        direct_session("shell-2", "shell", SessionStatus::Stopped),
        direct_session("active-chat", "codex", SessionStatus::Running),
        direct_session("idle-chat", "claude", SessionStatus::Stopped),
    ];
    let session_ids = vec!["shell-1".into(), "active-chat".into(), "idle-chat".into()];

    assert_eq!(
        archive_session_plan(&session_ids, &sessions, Some("active-chat")),
        [
            (
                "idle-chat".into(),
                ArchiveSessionOperation::ArchiveChat { running: false },
            ),
            (
                "active-chat".into(),
                ArchiveSessionOperation::ArchiveChat { running: true },
            ),
            ("shell-1".into(), ArchiveSessionOperation::CloseTerminal,),
        ]
    );
    assert_eq!(
        archive_all_confirmation_body(
            &["active-chat".into(), "shell-1".into()],
            &sessions,
        ).map(|body| body.resolve().to_string()),
        Some("This archives 1 chat and permanently closes 1 terminal. Archived chats can be restored from Settings → Archived; closed terminals cannot.".into()),
    );
    assert_eq!(
        archive_all_confirmation_body(
            &[
                "active-chat".into(),
                "idle-chat".into(),
                "shell-1".into(),
                "shell-2".into(),
            ],
            &sessions,
        ).map(|body| body.resolve().to_string()),
        Some("This archives 2 chats and permanently closes 2 terminals. Archived chats can be restored from Settings → Archived; closed terminals cannot.".into()),
    );
    assert_eq!(
        archive_all_confirmation_body(&["shell-1".into()], &sessions)
            .map(|body| body.resolve().to_string()),
        Some("This permanently closes 1 terminal. Closed terminals cannot be restored.".into()),
    );
    assert_eq!(
        archive_all_confirmation_body(&["active-chat".into(), "idle-chat".into()], &sessions,),
        None,
    );
}

#[test]
fn archiving_every_chat_in_a_tab_also_closes_its_drawer_shells() {
    let mut single = PaneLayout::single(Some("chat"), &["chat".into()]);
    single.add_drawer_shell("shell-1".into());
    single.add_drawer_shell("shell-2".into());
    assert_eq!(
        archive_targets_for_chats(vec!["chat".into()], &[single]),
        ["chat", "shell-1", "shell-2"]
    );

    let mut split = two_pane_layout("chat-1", "chat-2");
    split.add_drawer_shell("shell".into());
    assert_eq!(
        archive_targets_for_chats(vec!["chat-1".into()], &[split.clone()]),
        ["chat-1"]
    );
    assert_eq!(
        archive_targets_for_chats(vec!["chat-1".into(), "chat-2".into()], &[split],),
        ["chat-1", "chat-2", "shell"]
    );
}

fn two_pane_layout(first: &str, second: &str) -> PaneLayout {
    let mut layout = PaneLayout::single(Some(first), &[first.to_owned()]);
    let pane = layout.split("p1", SplitOrientation::Row).unwrap();
    layout.assign_session(&pane, second).unwrap();
    layout
}

fn pending_pane_close(tab_id: &str, pane_id: &str) -> PendingPaneClose {
    PendingPaneClose {
        tab_id: tab_id.into(),
        pane_id: pane_id.into(),
    }
}

#[test]
fn pane_close_after_archive_finds_the_emptied_leaf_by_tab_id() {
    let mut front = PaneLayout::single(Some("chat-0"), &["chat-0".into()]);
    front.id = "front".into();
    let mut split = two_pane_layout("chat-1", "chat-2");
    split.id = "split".into();
    let mut tabs = vec![front, split];
    assert!(pane_close_after_archive(&tabs, &pending_pane_close("split", "p1")).is_none());

    tabs[1].remove_session("chat-1");
    assert_eq!(
        pane_close_after_archive(&tabs, &pending_pane_close("split", "p1"))
            .map(|layout| layout.id.as_str()),
        Some("split")
    );
    assert!(pane_close_after_archive(&tabs, &pending_pane_close("split", "p3")).is_none());
    assert!(pane_close_after_archive(&tabs, &pending_pane_close("split", "p9")).is_none());
    assert!(pane_close_after_archive(&tabs, &pending_pane_close("gone", "p1")).is_none());

    let mut single = PaneLayout::single(None, &[]);
    single.id = "single".into();
    assert!(pane_close_after_archive(&[single], &pending_pane_close("single", "p1")).is_none());
}

#[test]
fn pending_pane_closes_are_consumed_only_by_the_archive_that_attempted_them() {
    let mut pending = HashMap::from([
        ("a".to_owned(), pending_pane_close("tab", "p1")),
        ("b".to_owned(), pending_pane_close("tab", "p2")),
    ]);

    let closes = take_pending_pane_closes(&mut pending, &["b".into()], &["b".into()]);
    assert_eq!(closes, [pending_pane_close("tab", "p2")]);
    assert!(pending.contains_key("a"));

    assert!(take_pending_pane_closes(&mut pending, &["c".into()], &["c".into()]).is_empty());
    assert!(pending.contains_key("a"));

    assert!(take_pending_pane_closes(&mut pending, &["a".into()], &[]).is_empty());
    assert!(pending.is_empty());
}

#[test]
fn overlapping_pane_closes_collapse_a_three_way_split_one_leaf_at_a_time() {
    let mut layout = two_pane_layout("a", "b");
    let third = layout.split("p3", SplitOrientation::Row).unwrap();
    layout.assign_session(&third, "c").unwrap();
    layout.id = "tab".into();
    let mut tabs = vec![layout];
    let mut pending = HashMap::from([
        ("a".to_owned(), pending_pane_close("tab", "p1")),
        ("b".to_owned(), pending_pane_close("tab", "p3")),
    ]);

    tabs[0].remove_session("b");
    for close in take_pending_pane_closes(&mut pending, &["b".into()], &["b".into()]) {
        assert_eq!(
            pane_close_after_archive(&tabs, &close).map(|layout| layout.id.as_str()),
            Some("tab")
        );
        assert!(tabs[0].close_pane(&close.pane_id));
    }
    assert_eq!(tabs[0].session_ids(), ["a", "c"]);
    assert_eq!(tabs[0].root.leaves().len(), 2);

    tabs[0].remove_session("a");
    for close in take_pending_pane_closes(&mut pending, &["a".into()], &["a".into()]) {
        assert!(pane_close_after_archive(&tabs, &close).is_some());
        assert!(tabs[0].close_pane(&close.pane_id));
    }
    assert_eq!(tabs[0].session_ids(), ["c"]);
    assert!(matches!(tabs[0].root, PaneNode::Leaf(_)));
    assert!(pending.is_empty());
}

#[test]
fn sidebar_fork_menu_target_exposes_enabled_and_disabled_single_chats() {
    let layout = PaneLayout::single(Some("chat"), &["chat".into()]);
    let mut codex = direct_session("chat", "codex", SessionStatus::Running);
    codex.forkable = true;
    codex.agent_session_key = Some("key".into());
    let members = vec![codex];
    let target = sidebar_fork_menu_target(&layout, &members).expect("fork target");
    assert_eq!(target.session_id, "chat");
    assert!(!target.disabled);
    assert_eq!(target.reason, None);

    let entries = tab_menu_entries(
        "tab-1",
        false,
        "Codex".into(),
        None,
        false,
        Some(target.clone()),
        false,
        vec!["chat".into()],
        vec!["chat".into()],
        None,
        &keymap::KeymapOverrides::new(),
    );
    assert_eq!(
        menu_labels(&entries),
        ["Pin", "Rename tab", "Fork chat", "Archive"]
    );
    assert!(!entries[2].0.disabled);
    assert_eq!(entries[2].1, SidebarMenuAction::ForkChat("chat".into()));
    assert!(entries[..3].iter().all(|(item, _)| item.shortcut.is_none()));
    assert_eq!(
        entries[3].0.shortcut.as_deref(),
        Some(keymap::fixed_shortcut("cmd-w").as_str())
    );

    let busy_entries = tab_menu_entries(
        "tab-1",
        false,
        "Codex".into(),
        None,
        false,
        Some(target),
        true,
        vec!["chat".into()],
        vec!["chat".into()],
        None,
        &keymap::KeymapOverrides::from([("close-pane".into(), None)]),
    );
    assert!(busy_entries[2].0.disabled);
    assert!(busy_entries[3].0.shortcut.is_none());

    let mut waiting = direct_session("chat", "codex", SessionStatus::Running);
    waiting.native_fork = true;
    let waiting_members = vec![waiting];
    let waiting_target = sidebar_fork_menu_target(&layout, &waiting_members).unwrap();
    assert!(waiting_target.disabled);
    assert_eq!(waiting_target.reason, Some("No session key captured yet"));
    let waiting_entries = tab_menu_entries(
        "tab-1",
        false,
        "Waiting".into(),
        None,
        false,
        Some(waiting_target),
        false,
        vec!["chat".into()],
        vec!["chat".into()],
        None,
        &keymap::KeymapOverrides::new(),
    );
    assert!(waiting_entries[2].0.disabled);
    assert_eq!(waiting_entries[2].0.description, None);
    assert_eq!(
        waiting_entries[2].0.tooltip.clone(),
        Some("No session key captured yet".into())
    );

    let trae_members = vec![direct_session("chat", "trae", SessionStatus::Running)];
    let trae_target = sidebar_fork_menu_target(&layout, &trae_members).unwrap();
    assert!(trae_target.disabled);
    assert_eq!(trae_target.reason, None);
    let trae_entries = tab_menu_entries(
        "tab-1",
        false,
        "Trae".into(),
        None,
        false,
        Some(trae_target),
        false,
        vec!["chat".into()],
        vec!["chat".into()],
        None,
        &keymap::KeymapOverrides::new(),
    );
    assert!(trae_entries[2].0.disabled);
    assert_eq!(trae_entries[2].0.tooltip, None);

    for runtime in ["copilot", "antigravity"] {
        let members = vec![direct_session("chat", runtime, SessionStatus::Running)];
        let target = sidebar_fork_menu_target(&layout, &members).unwrap();
        assert!(target.disabled, "{runtime}");
        assert_eq!(target.reason, None, "{runtime}");
    }

    let shell_members = vec![direct_session("chat", "shell", SessionStatus::Running)];
    assert!(sidebar_fork_menu_target(&layout, &shell_members).is_none());

    let mut grouped = PaneLayout::single(Some("chat"), &["chat".into()]);
    grouped.split("p1", SplitOrientation::Row).unwrap();
    assert!(sidebar_fork_menu_target(&grouped, &members).is_none());
}

#[test]
fn mission_refresh_filters_appended_signals() {
    for signal in [
        "mission_start",
        "mission_stopped",
        "mission_reopened",
        "ask_human",
        "human_question",
        "human_response",
        "session_status",
        "runner_status",
    ] {
        assert_eq!(
            StoreRefreshKind::for_event(&appended_event(signal)),
            Some(StoreRefreshKind::Missions)
        );
    }
    assert_eq!(
        StoreRefreshKind::for_event(&appended_event("inbox_read")),
        None
    );
    assert_eq!(
        StoreRefreshKind::for_event(&AppEvent {
            name: "event/appended".into(),
            payload: serde_json::json!({ "event": { "kind": "message" } }),
        }),
        None
    );
}

#[test]
fn chat_tab_selection_only_appears_on_the_chat_route() {
    assert!(chat_tab_row_active(&AppRoute::Chat, Some("tab-1"), "tab-1"));
    assert!(!chat_tab_row_active(
        &AppRoute::Chat,
        Some("tab-2"),
        "tab-1"
    ));
    assert!(!chat_tab_row_active(
        &AppRoute::Mission("mission-1".into()),
        Some("tab-1"),
        "tab-1"
    ));
    assert!(!chat_tab_row_active(
        &AppRoute::Settings,
        Some("tab-1"),
        "tab-1"
    ));
}

#[test]
fn shell_sessions_default_to_the_shell_command_name() {
    assert_eq!(
        default_session_label_parts("shell", "/bin/zsh", None, "Shell"),
        "zsh"
    );
    assert_eq!(
        default_session_label_parts("shell", "fish", None, "Shell"),
        "fish"
    );
    assert_eq!(
        default_session_label_parts("codex", "codex", Some("coder"), "Codex"),
        "@coder"
    );
}

#[test]
fn sidebar_icons_use_text_opacity_for_liveness_and_provider_tints_only_while_live() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for variant in [
        theme::ThemeVariant::Carbon,
        theme::ThemeVariant::RunnerLight,
    ] {
        theme::set_active_variant(variant);
        let generic = ChatIcon::generic("message-square.svg");
        assert_eq!(sidebar_icon_color(generic, true), theme::text());
        assert_eq!(
            sidebar_icon_color(generic, false),
            theme::with_alpha(theme::text(), 0.45)
        );

        for accented in [
            ChatIcon::mission(),
            ChatIcon::split(),
            ChatIcon::for_runtime("shell"),
        ] {
            assert_eq!(sidebar_icon_color(accented, true), theme::accent());
            assert_eq!(
                sidebar_icon_color(accented, false),
                theme::with_alpha(theme::text(), 0.45)
            );
        }

        let provider = ChatIcon::for_runtime("claude-code");
        let tint = gpui::rgb(0xd97757).into();
        assert_eq!(sidebar_icon_color(provider, true), tint);
        assert_eq!(
            sidebar_icon_color(provider, false),
            theme::with_alpha(theme::text(), 0.45)
        );
    }
}

#[test]
fn project_header_icon_follows_expansion_state() {
    assert_eq!(project_header_icon(true).asset_path(), "folder.svg");
    assert_eq!(project_header_icon(false).asset_path(), "folder-open.svg");
}

#[test]
fn sidebar_scroll_layout_reports_overflow_and_fills_short_lists() {
    let mut cx = TestAppContext::single();
    let scroll = ScrollHandle::new();
    let test_scroll = scroll.clone();
    let block_wrapper_scroll = ScrollHandle::new();
    let test_block_wrapper_scroll = block_wrapper_scroll.clone();
    let constrained_section_scroll = ScrollHandle::new();
    let test_constrained_section_scroll = constrained_section_scroll.clone();
    let short_scroll = ScrollHandle::new();
    let test_short_scroll = short_scroll.clone();
    let window = cx.add_window(move |_, _| SidebarScrollLayoutTest {
        scroll: test_scroll,
        block_wrapper_scroll: test_block_wrapper_scroll,
        constrained_section_scroll: test_constrained_section_scroll,
        short_scroll: test_short_scroll,
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(window.into(), &cx);
    window.simulate_resize(size(px(960.), px(160.)));
    window.run_until_parked();

    let viewport = f32::from(scroll.bounds().size.height);
    let max_offset = f32::from(scroll.max_offset().y);
    let block_wrapper_viewport = f32::from(block_wrapper_scroll.bounds().size.height);
    let block_wrapper_max_offset = f32::from(block_wrapper_scroll.max_offset().y);
    let constrained_section_max_offset = f32::from(constrained_section_scroll.max_offset().y);
    let constrained_chats_height = f32::from(
        window
            .debug_bounds("TEST_CHATS_test-constrained-section-sidebar-node-scroll")
            .unwrap()
            .size
            .height,
    );
    let short_chats_height = f32::from(
        window
            .debug_bounds("TEST_CHATS_test-short-sidebar-node-scroll")
            .unwrap()
            .size
            .height,
    );
    assert_eq!(viewport, 160.);
    assert_eq!(max_offset, 460.);
    assert_eq!(block_wrapper_viewport, 620.);
    assert_eq!(block_wrapper_max_offset, 0.);
    assert_eq!(constrained_section_max_offset, 100.);
    assert_eq!(constrained_chats_height, 0.);
    assert_eq!(short_chats_height, 100.);
}

#[test]
fn creating_routes_highlight_their_workspace_entry() {
    assert!(WorkspaceEntry::Role.selected(&AppRoute::NewRole));
    assert!(!WorkspaceEntry::Crew.selected(&AppRoute::NewRole));
    assert!(WorkspaceEntry::Crew.selected(&AppRoute::NewCrew));
    assert!(!WorkspaceEntry::Role.selected(&AppRoute::NewCrew));
}

#[test]
fn both_create_menus_show_effective_bindings_and_omit_unbound_keys() {
    let mut overrides = keymap::KeymapOverrides::new();
    overrides.insert(
        "new-chat".into(),
        Some(keymap::entry("command-palette").unwrap().default.clone()),
    );
    overrides.insert("new-terminal".into(), None);
    for entries in [
        sidebar_create_menu_entries(&overrides),
        project_create_menu_entries("project-1", &overrides),
    ] {
        assert_eq!(
            entries[0].0.shortcut.as_deref(),
            Some(keymap::format_combo(&keymap::entry("command-palette").unwrap().default).as_str())
        );
        assert_eq!(
            entries[1].0.shortcut.as_deref(),
            Some(keymap::format_combo(&keymap::entry("new-mission").unwrap().default).as_str())
        );
        assert!(entries[2].0.shortcut.is_none());
    }
}

#[cfg(debug_assertions)]
#[test]
fn empty_tab_rename_source_changes_language_while_named_tabs_stay_literal() {
    crate::i18n::testing::with_locale(crate::i18n::Locale::English, || {
        let mut layout = PaneLayout::single(None, &[]);
        let source = super::elements::tab_label_source(&layout, &[] as &[SessionDisplay], |_| None);
        layout.name = Some("Empty tab {name}".into());
        let literal =
            super::elements::tab_label_source(&layout, &[] as &[SessionDisplay], |_| None);
        assert_eq!(source.resolve(), "Empty tab");
        crate::i18n::set_active_locale(crate::i18n::Locale::SimplifiedChinese);
        assert_eq!(source.resolve(), "空标签页");
        assert_eq!(literal.resolve(), "Empty tab {name}");
        crate::i18n::set_active_locale(crate::i18n::Locale::English);
        assert_eq!(source.resolve(), "Empty tab");
    });
}

fn active_row_names(rows: &[super::active::ActiveSidebarRow], nodes: &[NodeRow]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            let id = row.shortcut_row(nodes).node_id;
            nodes
                .iter()
                .find(|node| node.id == id)
                .and_then(|node| node.name.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or(id)
        })
        .collect()
}

#[test]
fn active_projection_orders_tabs_and_rolls_up_missions_without_drawers() {
    use super::active::ActiveGroup;
    use runner_core::protocol::status::{
        Activity, AgentStatus, HumanInteraction, Lifecycle, ObservationSource, WaitReason,
    };
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (pool, store) = seeded_store(&mut cx, temp.path());
    let conn = pool.get().unwrap();
    let mut split = two_pane_layout("working-old", "terminal");
    split.add_drawer_shell("drawer".into());
    let split_node =
        runner_daemon::repo::node::create_tab(&conn, None, "Mixed", 0, &split.serialize().unwrap())
            .unwrap();
    for (index, id) in [
        "error",
        "wait",
        "working-new",
        "unread",
        "idle",
        "unavailable",
        "member",
        "working-tie",
    ]
    .iter()
    .enumerate()
    {
        let layout = PaneLayout::single(Some(id), &[(*id).into()]);
        runner_daemon::repo::node::create_tab(
            &conn,
            None,
            id,
            index as i64 + 1,
            &layout.serialize().unwrap(),
        )
        .unwrap();
    }
    for (name, created_at) in [
        ("idle", "2026-10-01T00:00:00Z"),
        ("Mixed", "2026-10-01T00:00:01+00:00"),
        ("unread", "2026-10-01T00:00:01.5+00:00"),
        ("error", "2026-10-01T00:00:02Z"),
        ("working-new", "2026-10-01T00:00:03Z"),
        ("working-tie", "2026-10-01T00:00:03Z"),
        ("wait", "2026-10-01T00:00:04Z"),
        ("unavailable", "2026-10-01T00:00:05Z"),
    ] {
        conn.execute(
            "UPDATE nodes SET created_at = ?2 WHERE name = ?1",
            [name, created_at],
        )
        .unwrap();
    }
    drop(conn);
    store.update(&mut cx, |store, cx| {
        store.refresh_nodes(cx).unwrap();
        store.settings.sidebar_active_only = true;
        let working = AgentStatus {
            lifecycle: Lifecycle::Running,
            observation: runner_core::protocol::status::AgentObservation {
                activity: Activity::Working,
                source: ObservationSource::Hook,
                ..Default::default()
            },
            ..Default::default()
        };
        store.sessions = [
            "working-old",
            "terminal",
            "drawer",
            "error",
            "wait",
            "working-new",
            "unread",
            "idle",
            "unavailable",
            "member",
            "working-tie",
        ]
        .into_iter()
        .map(|id| {
            direct_session(
                id,
                if matches!(id, "terminal" | "drawer") {
                    "shell"
                } else {
                    "codex"
                },
                SessionStatus::Running,
            )
        })
        .collect();
        store.session_statuses = store
            .sessions
            .iter()
            .map(|session| (session.session_id.clone(), working.clone()))
            .collect();
        store.session_statuses.get_mut("error").unwrap().error_since = Some(1);
        store
            .session_statuses
            .get_mut("wait")
            .unwrap()
            .observation
            .interactions
            .push(HumanInteraction {
                id: "ask".into(),
                reason: WaitReason::Approval,
                owners: vec![],
                since: 2,
            });
        let unread = store.session_statuses.get_mut("unread").unwrap();
        unread.lifecycle = Lifecycle::Stopped;
        unread.unread_since = Some(4);
        store
            .session_statuses
            .get_mut("idle")
            .unwrap()
            .observation
            .activity = Activity::Ready;
        store
            .session_statuses
            .get_mut("unavailable")
            .unwrap()
            .observation
            .activity = Activity::Unavailable;
        store
            .session_activity
            .insert("terminal".into(), SessionActivityState::Busy);
        store
            .session_activity
            .insert("drawer".into(), SessionActivityState::Busy);
        store.missions[0].session_statuses = vec![("member".into(), working)];
    });
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    let mut tabs = cx.update(|cx| TabSet::from_rows(&store.read(cx).nodes));
    let order = sidebar.update(&mut cx, |sidebar, cx| {
        let rows = sidebar.resolved_active_rows_for_tabs(&tabs, cx);
        let nodes = &store.read(cx).nodes;
        let shortcuts = rows
            .iter()
            .map(|row| row.shortcut_row(nodes))
            .collect::<Vec<_>>();
        let tab_id = |name: &str| {
            nodes
                .iter()
                .find(|node| node.name.as_deref() == Some(name))
                .unwrap()
                .id
                .clone()
        };
        let mut tied = ["working-new", "working-tie"];
        tied.sort_by_key(|name| tab_id(name));
        let mission = nodes
            .iter()
            .find(|node| node.node_type == NodeType::Mission)
            .unwrap()
            .id
            .clone();
        assert_eq!(
            active_row_names(&rows, nodes),
            [
                "idle",
                "Mixed",
                "unread",
                "error",
                tied[0],
                tied[1],
                "wait",
                "unavailable",
                mission.as_str(),
            ]
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row.group() == ActiveGroup::Chats)
                .count(),
            8
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row.group() == ActiveGroup::Missions)
                .count(),
            1
        );
        assert!(rows.iter().all(|row| row.group() != ActiveGroup::Terminals));
        assert_eq!(
            shortcuts[1].target,
            SidebarActivationTarget::Tab {
                tab_id: split_node.id.clone(),
                session_id: "working-old".into()
            }
        );
        sidebar.refresh_shortcut_rows(&tabs, cx);
        assert_eq!(
            sidebar
                .numbered_shortcut_rows
                .iter()
                .map(|row| (&row.node_id, &row.target))
                .collect::<Vec<_>>(),
            shortcuts
                .iter()
                .take(9)
                .map(|row| (&row.node_id, &row.target))
                .collect::<Vec<_>>()
        );
        tabs.activate(&split_node.id);
        tabs.active_mut().unwrap().focus_session("terminal");
        shortcuts
            .into_iter()
            .map(|row| row.node_id)
            .collect::<Vec<_>>()
    });
    store.update(&mut cx, |store, _| {
        store.session_statuses.get_mut("error").unwrap().error_since = None;
        store.session_statuses.get_mut("idle").unwrap().error_since = Some(5);
        store
            .session_statuses
            .get_mut("wait")
            .unwrap()
            .observation
            .interactions
            .clear();
        store
            .session_statuses
            .get_mut("unavailable")
            .unwrap()
            .unread_since = Some(6);
        store
            .sessions
            .iter_mut()
            .find(|session| session.session_id == "unread")
            .unwrap()
            .started_at = Some(chrono::Utc::now());
        store
            .session_activity
            .insert("working-old".into(), SessionActivityState::Busy);
    });
    sidebar.update(&mut cx, |sidebar, cx| {
        let rows = sidebar.resolved_active_rows_for_tabs(&tabs, cx);
        let nodes = &store.read(cx).nodes;
        assert_eq!(
            rows.iter()
                .map(|row| row.shortcut_row(nodes).node_id)
                .collect::<Vec<_>>(),
            order
        );
        sidebar.refresh_shortcut_rows(&tabs, cx);
        assert_eq!(
            sidebar
                .numbered_shortcut_rows
                .iter()
                .map(|row| &row.node_id)
                .collect::<Vec<_>>(),
            order.iter().take(9).collect::<Vec<_>>()
        );
    });
    store.update(&mut cx, |store, _| {
        for status in store.session_statuses.values_mut() {
            status.observation.activity = Activity::Idle;
            status.observation.interactions.clear();
            status.error_since = None;
            status.unread_since = None;
        }
        for session in &mut store.sessions {
            if session.session_id != "terminal" {
                session.status = SessionStatus::Stopped;
            }
        }
        for status in store.session_statuses.values_mut() {
            status.lifecycle = Lifecycle::Stopped;
        }
        store.session_activity.clear();
        store.missions[0].session_statuses.clear();
    });
    sidebar.update(&mut cx, |sidebar, cx| {
        let nodes = &store.read(cx).nodes;
        let retained = sidebar.resolved_active_rows_for_tabs(&tabs, cx);
        assert_eq!(active_row_names(&retained, nodes), ["Mixed"]);
        assert_eq!(
            retained[0].shortcut_row(nodes).target,
            SidebarActivationTarget::Tab {
                tab_id: split_node.id.clone(),
                session_id: "terminal".into()
            }
        );
        tabs.active_mut().unwrap().focus_session("working-old");
        let switched = sidebar.resolved_active_rows_for_tabs(&tabs, cx);
        assert_eq!(active_row_names(&switched, nodes), ["Mixed"]);
        assert_eq!(
            switched[0].shortcut_row(nodes).target,
            SidebarActivationTarget::Tab {
                tab_id: split_node.id.clone(),
                session_id: "working-old".into()
            }
        );
        assert_eq!(
            store
                .read(cx)
                .nodes
                .iter()
                .find(|node| node.id == split_node.id)
                .unwrap()
                .name
                .as_deref(),
            Some("Mixed")
        );
    });
}

#[test]
fn active_includes_live_idle_chats_and_missions_and_removes_stopped_selection() {
    use super::active::ActiveGroup;
    use runner_core::protocol::model::MissionStatus;
    use runner_core::protocol::status::{Activity, AgentStatus, Lifecycle};
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (pool, store) = seeded_store(&mut cx, temp.path());
    let conn = pool.get().unwrap();
    conn.execute_batch(
        "INSERT INTO missions(id, title, status, started_at) VALUES
         ('stale', 'No live members', 'running', '2026-10-19T00:00:00Z');",
    )
    .unwrap();
    runner_daemon::repo::node::ensure_mission_node(&conn, "stale", None).unwrap();
    let mut chat_tab = None;
    for (index, id) in ["idle", "unknown", "stopped", "unread", "member"]
        .iter()
        .enumerate()
    {
        let layout = PaneLayout::single(Some(id), &[(*id).into()]);
        let node = runner_daemon::repo::node::create_tab(
            &conn,
            None,
            id,
            index as i64,
            &layout.serialize().unwrap(),
        )
        .unwrap();
        if *id == "idle" {
            chat_tab = Some(node);
        }
    }
    drop(conn);
    store.update(&mut cx, |store, cx| {
        store.refresh_nodes(cx).unwrap();
        store.missions = store.client.mission_list_summary_impl(None).unwrap();
        store.settings.sidebar_active_only = true;
        store.sessions = ["idle", "unknown", "stopped", "unread", "member"]
            .into_iter()
            .map(|id| {
                direct_session(
                    id,
                    "codex",
                    if matches!(id, "stopped" | "unread") {
                        SessionStatus::Stopped
                    } else {
                        SessionStatus::Running
                    },
                )
            })
            .collect();
        let idle = AgentStatus {
            lifecycle: Lifecycle::Running,
            observation: runner_core::protocol::status::AgentObservation {
                activity: Activity::Ready,
                ..Default::default()
            },
            ..Default::default()
        };
        store.session_statuses.insert("idle".into(), idle.clone());
        store.session_statuses.insert("member".into(), idle.clone());
        store.session_statuses.insert(
            "unread".into(),
            AgentStatus {
                lifecycle: Lifecycle::Stopped,
                unread_since: Some(1),
                ..Default::default()
            },
        );
        let mission = store
            .missions
            .iter_mut()
            .find(|summary| summary.mission.id == "mission")
            .unwrap();
        mission.mission.status = MissionStatus::Running;
        mission.any_session_live = true;
        mission.session_statuses = vec![("member".into(), idle)];
    });
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    let mut tabs = cx.update(|cx| TabSet::from_rows(&store.read(cx).nodes));
    tabs.activate(&chat_tab.unwrap().id);
    sidebar.update(&mut cx, |sidebar, cx| {
        let rows = sidebar.resolved_active_rows_for_tabs(&tabs, cx);
        assert_eq!(
            rows.iter()
                .filter(|row| row.group() == ActiveGroup::Chats)
                .count(),
            3
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row.group() == ActiveGroup::Missions)
                .count(),
            1
        );
        let shortcuts = rows
            .iter()
            .map(|row| row.shortcut_row(&store.read(cx).nodes))
            .collect::<Vec<_>>();
        let names = active_row_names(&rows, &store.read(cx).nodes);
        for name in ["idle", "unknown", "unread"] {
            assert!(names.iter().any(|row| row == name));
        }
        for name in ["stopped", "member"] {
            assert!(!names.iter().any(|row| row == name));
        }
        sidebar.refresh_shortcut_rows(&tabs, cx);
        assert_eq!(
            sidebar
                .numbered_shortcut_rows
                .iter()
                .map(|row| &row.node_id)
                .collect::<Vec<_>>(),
            shortcuts.iter().map(|row| &row.node_id).collect::<Vec<_>>()
        );
    });
    store.update(&mut cx, |store, _| {
        store
            .sessions
            .iter_mut()
            .find(|session| session.session_id == "idle")
            .unwrap()
            .status = SessionStatus::Stopped;
        let mission = store
            .missions
            .iter_mut()
            .find(|summary| summary.mission.id == "mission")
            .unwrap();
        mission.any_session_live = false;
        mission.mission.status = MissionStatus::Completed;
        mission.session_statuses[0].1.lifecycle = Lifecycle::Stopped;
    });
    sidebar.update(&mut cx, |sidebar, cx| {
        let rows = sidebar.resolved_active_rows_for_tabs(&tabs, cx);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.group() == ActiveGroup::Chats));
        assert!(!active_row_names(&rows, &store.read(cx).nodes).contains(&"idle".into()));
        let ids = rows
            .iter()
            .map(|row| row.shortcut_row(&store.read(cx).nodes).node_id)
            .collect::<Vec<_>>();
        sidebar.refresh_shortcut_rows(&tabs, cx);
        assert_eq!(
            sidebar
                .numbered_shortcut_rows
                .iter()
                .map(|row| &row.node_id)
                .collect::<Vec<_>>(),
            ids.iter().collect::<Vec<_>>()
        );
    });
}

#[test]
fn active_location_shows_project_relative_worktree_and_compact_home() {
    use super::active::active_location;
    let project = runner_core::protocol::project::ProjectRow {
        id: "runner".into(),
        name: "Runner".into(),
        cwd: "/home/j/repos/runner".into(),
        position: 0,
        created_at: String::new(),
    };
    assert_eq!(
        active_location(Some(&project), Some(&project.cwd), None),
        "Runner"
    );
    assert_eq!(
        active_location(
            Some(&project),
            Some("/home/j/repos/runner/.worktrees/feat-887"),
            None
        ),
        "Runner › feat-887"
    );
    assert_eq!(
        active_location(
            Some(&project),
            Some("/home/j/repos/runner/crates/app"),
            None
        ),
        "Runner › crates/app"
    );
    assert_eq!(
        active_location(None, Some("/home/j/repos/runner"), Some("/home/j")),
        "~/repos/runner"
    );
    assert_eq!(
        active_location(Some(&project), Some("/tmp/work"), None),
        "Runner › /tmp/work"
    );
}

#[test]
fn active_shortcut_and_row_click_focus_the_split_pane_needing_attention() {
    use runner_core::protocol::status::{Activity, AgentStatus, Lifecycle};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    let conn = core.db.get().unwrap();
    conn.execute_batch("INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES ('first', 'stopped', 'codex', 'codex'), ('second', 'stopped', 'claude-code', 'claude');").unwrap();
    runner_daemon::repo::session_attention::record_completion(&conn, "second", false, 1).unwrap();
    let mut layout = two_pane_layout("first", "second");
    layout.focus_session("first");
    let tab = runner_daemon::repo::node::create_tab(
        &conn,
        None,
        "Split",
        1,
        &layout.serialize().unwrap(),
    )
    .unwrap();
    drop(conn);
    store.update(&mut cx, |store, cx| {
        store.refresh_sessions(cx);
        store.refresh_nodes(cx).unwrap();
        store.settings.sidebar_active_only = true;
        for (id, unread_since) in [("first", None), ("second", Some(1))] {
            store.session_statuses.insert(
                id.into(),
                AgentStatus {
                    lifecycle: Lifecycle::Stopped,
                    observation: runner_core::protocol::status::AgentObservation {
                        activity: Activity::Idle,
                        ..Default::default()
                    },
                    unread_since,
                    ..Default::default()
                },
            );
        }
    });
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "active-pane-routing".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.closing = true;
        root.route = AppRoute::Roles;
        root.root_focus.focus(window, cx);
        root
    });
    cx.update(|cx| keymap::install_bindings(cx, &keymap::KeymapOverrides::new(), false));
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    visual.run_until_parked();
    visual.simulate_keystrokes(&keymap::platform_default("cmd-1"));
    visual.run_until_parked();
    host.read_with(&visual, |root, _| {
        assert_eq!(root.route, AppRoute::Chat);
        assert_eq!(root.tabs.active_tab_id(), Some(tab.id.as_str()));
        assert_eq!(
            root.tabs.active().unwrap().focused_session_id(),
            Some("second")
        );
    })
    .unwrap();
    host.update(&mut visual, |root, _, _| {
        root.route = AppRoute::Roles;
        root.tabs.active_mut().unwrap().focus_session("first");
    })
    .unwrap();
    visual.run_until_parked();
    let row = visual
        .debug_bounds(Box::leak(
            format!("SIDEBAR_ROW active-{}", tab.id).into_boxed_str(),
        ))
        .unwrap();
    visual.simulate_click(row.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    host.read_with(&visual, |root, _| {
        assert_eq!(root.route, AppRoute::Chat);
        assert_eq!(root.tabs.active_tab_id(), Some(tab.id.as_str()));
        assert_eq!(
            root.tabs.active().unwrap().focused_session_id(),
            Some("second")
        );
    })
    .unwrap();
}

struct ActiveListRenderTest {
    sidebar: Entity<Sidebar>,
}

impl Render for ActiveListRenderTest {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.sidebar.update(cx, |sidebar, cx| {
                let tabs = TabSet::from_rows(&sidebar.app_store.read(cx).nodes);
                sidebar.show_shortcut_pills = true;
                sidebar.refresh_shortcut_rows(&tabs, cx);
                sidebar.render_active_groups(sidebar.resolved_active_rows_for_tabs(&tabs, cx), cx)
            }))
            .child(
                div()
                    .id("outside-active-rename")
                    .h(px(40.))
                    .w_full()
                    .on_click(|_, window, cx| window.blur(cx))
                    .debug_selector(|| "OUTSIDE_ACTIVE_RENAME".into()),
            )
            .children(self.sidebar.read(cx).context_menu.clone())
    }
}

#[test]
fn active_rename_edits_only_the_title_and_preserves_enter_escape_and_blur() {
    use runner_core::protocol::status::{AgentStatus, Lifecycle};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for action in ["enter", "escape", "blur"] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (pool, store) = seeded_store(&mut cx, temp.path());
        let conn = pool.get().unwrap();
        conn.execute_batch(
            "INSERT INTO sessions(id, title, cwd, status, agent_runtime, agent_command)
             VALUES ('chat', 'Original chat', '/tmp/active-rename', 'stopped', 'codex', 'codex');",
        )
        .unwrap();
        let layout = PaneLayout::single(Some("chat"), &["chat".into()]);
        let tab =
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
                .unwrap();
        let selector = |kind: &str| -> &'static str {
            Box::leak(format!("{kind} {}", tab.id).into_boxed_str())
        };
        let (title_selector, location_selector, row_selector) = (
            selector("ACTIVE_TITLE"),
            selector("ACTIVE_LOCATION"),
            Box::leak(format!("SIDEBAR_ROW active-{}", tab.id).into_boxed_str()),
        );
        drop(conn);
        store.update(&mut cx, |store, cx| {
            store.refresh_sessions(cx);
            store.refresh_nodes(cx).unwrap();
            store.settings.sidebar_active_only = true;
            store.session_statuses.insert(
                "chat".into(),
                AgentStatus {
                    lifecycle: Lifecycle::Stopped,
                    unread_since: Some(1),
                    ..Default::default()
                },
            );
        });
        let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store, None, cx));
        let host = cx.add_window(|_, _| ActiveListRenderTest {
            sidebar: sidebar.clone(),
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(300.), px(400.)));
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        assert!(visual.debug_bounds(location_selector).is_some());
        host.update(&mut visual, |_, window, cx| {
            sidebar.update(cx, |sidebar, cx| {
                sidebar.begin_sidebar_rename(
                    SidebarRenameTarget::Tab {
                        node_id: tab.id,
                        original: "Original chat".into(),
                        session_id: Some("chat".into()),
                    },
                    "Original chat".into(),
                    "Chat name".into(),
                    window,
                    cx,
                );
            });
            cx.notify();
        })
        .unwrap();
        visual.run_until_parked();
        let title = visual.debug_bounds(title_selector).unwrap();
        let location = visual.debug_bounds(location_selector).unwrap();
        let row = visual.debug_bounds(row_selector).unwrap();
        assert!(title.size.height > px(0.));
        assert!(location.size.height > px(0.));
        assert!(title.bottom() <= location.top());
        assert!(row.contains(&location.origin));
        assert!(location.bottom() <= row.bottom());
        sidebar.update(&mut visual, |sidebar, cx| {
            sidebar
                .rename
                .as_ref()
                .unwrap()
                .input
                .update(cx, |field, cx| {
                    field.set_text("Renamed chat", cx);
                });
        });
        visual.run_until_parked();
        if action == "blur" {
            let outside = visual.debug_bounds("OUTSIDE_ACTIVE_RENAME").unwrap();
            visual.simulate_click(outside.center(), gpui::Modifiers::default());
        } else {
            visual.simulate_keystrokes(action);
        }
        visual.run_until_parked();
        sidebar.read_with(&visual, |sidebar, _| assert!(sidebar.rename.is_none()));
        let title: String = pool
            .get()
            .unwrap()
            .query_row("SELECT title FROM sessions WHERE id = 'chat'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            title,
            if action == "escape" {
                "Original chat"
            } else {
                "Renamed chat"
            }
        );
    }
}

#[test]
fn active_split_rename_stays_on_its_tab_row_across_status_changes() {
    use runner_core::protocol::status::{AgentStatus, Lifecycle};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (pool, store) = seeded_store(&mut cx, temp.path());
    let conn = pool.get().unwrap();
    conn.execute_batch(
        "INSERT INTO sessions(id, title, cwd, status, agent_runtime, agent_command) VALUES
         ('chat', 'Original chat', '/tmp/split-chat', 'stopped', 'codex', 'codex'),
         ('terminal', 'Original terminal', '/tmp/split-terminal', 'stopped', 'shell', '/bin/sh'),
         ('other-chat', 'Other chat', '/tmp/other-chat', 'stopped', 'codex', 'codex');",
    )
    .unwrap();
    for id in ["chat", "terminal", "other-chat"] {
        runner_daemon::repo::session_attention::record_completion(&conn, id, false, 1).unwrap();
    }
    let layout = two_pane_layout("chat", "terminal");
    let tab = runner_daemon::repo::node::create_tab(
        &conn,
        None,
        "Original split",
        0,
        &layout.serialize().unwrap(),
    )
    .unwrap();
    let other = PaneLayout::single(Some("other-chat"), &["other-chat".into()]);
    let other_tab =
        runner_daemon::repo::node::create_tab(&conn, None, "", 1, &other.serialize().unwrap())
            .unwrap();
    for (id, created_at) in [
        (&other_tab.id, "2026-10-01T00:00:00Z"),
        (&tab.id, "2026-10-01T00:00:01Z"),
    ] {
        conn.execute(
            "UPDATE nodes SET created_at = ?2 WHERE id = ?1",
            [id.as_str(), created_at],
        )
        .unwrap();
    }
    drop(conn);
    store.update(&mut cx, |store, cx| {
        store.refresh_sessions(cx);
        store.refresh_nodes(cx).unwrap();
        store.settings.sidebar_active_only = true;
        for (id, since) in [("chat", 1), ("terminal", 1), ("other-chat", 2)] {
            store.session_statuses.insert(
                id.into(),
                AgentStatus {
                    lifecycle: Lifecycle::Stopped,
                    unread_since: Some(since),
                    ..Default::default()
                },
            );
        }
    });
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    let host = cx.add_window(|_, cx| {
        cx.observe(&sidebar, |_, _, cx| cx.notify()).detach();
        ActiveListRenderTest {
            sidebar: sidebar.clone(),
        }
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(400.), px(900.)));
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    let split_row: &str = Box::leak(format!("SIDEBAR_ROW active-{}", tab.id).into_boxed_str());
    let other_row: &str =
        Box::leak(format!("SIDEBAR_ROW active-{}", other_tab.id).into_boxed_str());
    let split = visual.debug_bounds(split_row).unwrap();
    let other = visual.debug_bounds(other_row).unwrap();
    assert!(split.top() > other.top());
    visual.simulate_mouse_down(
        split.center(),
        MouseButton::Right,
        gpui::Modifiers::default(),
    );
    visual.simulate_mouse_up(
        split.center(),
        MouseButton::Right,
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    sidebar.read_with(&visual, |sidebar, _| {
        assert!(
            sidebar.context_menu.is_some(),
            "split context menu did not open"
        )
    });
    visual.simulate_keystrokes("home down enter");
    visual.run_until_parked();
    let split_rename: &str = Box::leak(format!("ACTIVE_RENAME {}", tab.id).into_boxed_str());
    let other_rename: &str = Box::leak(format!("ACTIVE_RENAME {}", other_tab.id).into_boxed_str());
    assert!(visual.debug_bounds(split_rename).is_some());
    assert!(visual.debug_bounds(other_rename).is_none());
    let input = sidebar.read_with(&visual, |sidebar, _| {
        let rename = sidebar.rename.as_ref().unwrap();
        assert!(matches!(&rename.target, SidebarRenameTarget::Tab { node_id, session_id: None, .. } if *node_id == tab.id));
        rename.input.clone()
    });
    store.update(&mut visual, |store, _| {
        store.session_statuses.get_mut("chat").unwrap().error_since = Some(3);
    });
    host.update(&mut visual, |_, _, cx| cx.notify()).unwrap();
    visual.run_until_parked();
    let split = visual.debug_bounds(split_row).unwrap();
    let other = visual.debug_bounds(other_row).unwrap();
    assert!(split.top() > other.top());
    let editor = visual.debug_bounds(split_rename).unwrap();
    let location = visual
        .debug_bounds(Box::leak(
            format!("ACTIVE_LOCATION {}", tab.id).into_boxed_str(),
        ))
        .unwrap();
    assert!(editor.bottom() <= location.top());
    assert!(location.size.height > px(0.));
    assert!(visual.debug_bounds(other_rename).is_none());
    input.update(&mut visual, |field, cx| field.set_text("Renamed split", cx));
    visual.update(|window, cx| assert!(input.read(cx).focus_handle().is_focused(window)));
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    sidebar.read_with(&visual, |sidebar, _| assert!(sidebar.rename.is_none()));
    let conn = pool.get().unwrap();
    let name: String = conn
        .query_row("SELECT name FROM nodes WHERE id = ?1", [&tab.id], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(name, "Renamed split");
    let title: String = conn
        .query_row(
            "SELECT title FROM sessions WHERE id = 'terminal'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(title, "Original terminal");
}

#[test]
fn active_render_handles_256_eligible_rows_and_numbers_only_the_first_nine() {
    use runner_core::protocol::status::{AgentStatus, Lifecycle};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    let conn = core.db.get().unwrap();
    for index in 0..256 {
        let id = format!("chat-{index:03}");
        let layout = PaneLayout::single(Some(&id), std::slice::from_ref(&id));
        runner_daemon::repo::node::create_tab(
            &conn,
            None,
            &id,
            index,
            &layout.serialize().unwrap(),
        )
        .unwrap();
    }
    drop(conn);
    store.update(&mut cx, |store, cx| {
        store.refresh_nodes(cx).unwrap();
        store.settings.sidebar_active_only = true;
        store.sessions = (0..256)
            .map(|index| {
                direct_session(&format!("chat-{index:03}"), "codex", SessionStatus::Stopped)
            })
            .collect();
        store.session_statuses = store
            .sessions
            .iter()
            .map(|session| {
                (
                    session.session_id.clone(),
                    AgentStatus {
                        lifecycle: Lifecycle::Stopped,
                        unread_since: Some(1),
                        ..Default::default()
                    },
                )
            })
            .collect();
    });
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    let host = cx.add_window(|_, _| ActiveListRenderTest {
        sidebar: sidebar.clone(),
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(400.), px(900.)));
    visual.run_until_parked();
    for selector in [
        "SIDEBAR_SHORTCUT_1",
        "SIDEBAR_SHORTCUT_2",
        "SIDEBAR_SHORTCUT_3",
        "SIDEBAR_SHORTCUT_4",
        "SIDEBAR_SHORTCUT_5",
        "SIDEBAR_SHORTCUT_6",
        "SIDEBAR_SHORTCUT_7",
        "SIDEBAR_SHORTCUT_8",
        "SIDEBAR_SHORTCUT_9",
    ] {
        assert!(visual.debug_bounds(selector).is_some());
    }
    assert!(visual.debug_bounds("SIDEBAR_SHORTCUT_10").is_none());
    sidebar.read_with(&visual, |sidebar, cx| {
        let tabs = TabSet::from_rows(&store.read(cx).nodes);
        let rows = sidebar.resolved_active_rows_for_tabs(&tabs, cx);
        assert_eq!(rows.len(), 256);
        assert_eq!(sidebar.numbered_shortcut_rows.len(), 9);
        assert_eq!(
            sidebar.numbered_shortcut_rows.last().unwrap().node_id,
            rows[8].shortcut_row(&store.read(cx).nodes).node_id
        );
    });
}

#[test]
fn active_group_collapse_hides_rows_and_skips_their_shortcuts() {
    use runner_core::protocol::status::{Activity, AgentStatus, Lifecycle};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    let conn = core.db.get().unwrap();
    let [chat_node, terminal_node] = [("chat", 0), ("terminal", 1)].map(|(id, position)| {
        let layout = PaneLayout::single(Some(id), &[id.into()]);
        runner_daemon::repo::node::create_tab(
            &conn,
            None,
            id,
            position,
            &layout.serialize().unwrap(),
        )
        .unwrap()
    });
    drop(conn);
    store.update(&mut cx, |store, cx| {
        store.refresh_nodes(cx).unwrap();
        store.settings.sidebar_active_only = true;
        store.sessions = vec![
            direct_session("chat", "codex", SessionStatus::Stopped),
            direct_session("terminal", "shell", SessionStatus::Running),
        ];
        store.session_statuses.insert(
            "chat".into(),
            AgentStatus {
                lifecycle: Lifecycle::Stopped,
                unread_since: Some(1),
                ..Default::default()
            },
        );
        store.session_statuses.insert(
            "terminal".into(),
            AgentStatus {
                lifecycle: Lifecycle::Running,
                observation: runner_core::protocol::status::AgentObservation {
                    activity: Activity::Idle,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    });
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    let host = cx.add_window(|_, _| ActiveListRenderTest {
        sidebar: sidebar.clone(),
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(400.), px(900.)));
    visual.run_until_parked();
    let chat_title: &'static str =
        Box::leak(format!("ACTIVE_TITLE {}", chat_node.id).into_boxed_str());
    let numbered = |visual: &mut VisualTestContext| {
        sidebar.read_with(visual, |sidebar, _| {
            sidebar
                .numbered_shortcut_rows
                .iter()
                .map(|row| row.node_id.clone())
                .collect::<Vec<_>>()
        })
    };
    assert!(visual.debug_bounds(chat_title).is_some());
    assert_eq!(
        numbered(&mut visual),
        [chat_node.id.clone(), terminal_node.id.clone()]
    );

    let toggle = visual.debug_bounds("ACTIVE_GROUP_TOGGLE chats").unwrap();
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds(chat_title).is_none());
    assert!(visual.debug_bounds("SIDEBAR_SHORTCUT_1").is_some());
    assert!(visual.debug_bounds("SIDEBAR_SHORTCUT_2").is_none());
    assert_eq!(
        numbered(&mut visual),
        std::slice::from_ref(&terminal_node.id)
    );
    store.read_with(&visual, |store, _| {
        assert!(store
            .settings
            .sidebar_collapsed_active_groups
            .contains("chats"));
    });

    let toggle = visual.debug_bounds("ACTIVE_GROUP_TOGGLE chats").unwrap();
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds(chat_title).is_some());
    assert_eq!(
        numbered(&mut visual),
        [chat_node.id.clone(), terminal_node.id.clone()]
    );
}

#[test]
fn active_lists_unselected_idle_shells_until_they_exit() {
    use runner_core::protocol::status::{Activity, AgentStatus, Lifecycle};
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (pool, store) = seeded_store(&mut cx, temp.path());
    let conn = pool.get().unwrap();
    let [chat_node, terminal_node] = [("chat", 0), ("terminal", 1)].map(|(id, position)| {
        let layout = PaneLayout::single(Some(id), &[id.into()]);
        runner_daemon::repo::node::create_tab(
            &conn,
            None,
            id,
            position,
            &layout.serialize().unwrap(),
        )
        .unwrap()
    });
    drop(conn);
    store.update(&mut cx, |store, cx| {
        store.refresh_nodes(cx).unwrap();
        store.settings.sidebar_active_only = true;
        store.sessions = vec![
            direct_session("chat", "codex", SessionStatus::Stopped),
            direct_session("terminal", "shell", SessionStatus::Running),
        ];
        store.session_statuses.insert(
            "chat".into(),
            AgentStatus {
                lifecycle: Lifecycle::Stopped,
                unread_since: Some(1),
                ..Default::default()
            },
        );
        store.session_statuses.insert(
            "terminal".into(),
            AgentStatus {
                lifecycle: Lifecycle::Running,
                observation: runner_core::protocol::status::AgentObservation {
                    activity: Activity::Idle,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    });
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    let mut tabs = cx.update(|cx| TabSet::from_rows(&store.read(cx).nodes));
    tabs.activate(&chat_node.id);
    sidebar.update(&mut cx, |sidebar, cx| {
        sidebar.refresh_shortcut_rows(&tabs, cx);
        assert_eq!(
            sidebar
                .numbered_shortcut_rows
                .iter()
                .map(|row| row.node_id.as_str())
                .collect::<Vec<_>>(),
            [chat_node.id.as_str(), terminal_node.id.as_str()]
        );
    });
    store.update(&mut cx, |store, _| {
        store
            .sessions
            .iter_mut()
            .find(|session| session.session_id == "terminal")
            .unwrap()
            .status = SessionStatus::Stopped;
        let terminal = store.session_statuses.get_mut("terminal").unwrap();
        terminal.lifecycle = Lifecycle::Stopped;
        terminal.exit_code = Some(1);
    });
    sidebar.update(&mut cx, |sidebar, cx| {
        let rows = sidebar.resolved_active_rows_for_tabs(&tabs, cx);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].group(), super::active::ActiveGroup::Chats);
        sidebar.refresh_shortcut_rows(&tabs, cx);
        assert_eq!(sidebar.numbered_shortcut_rows.len(), 1);
        assert_eq!(sidebar.numbered_shortcut_rows[0].node_id, chat_node.id);
    });
    store.update(&mut cx, |store, _| {
        store
            .sessions
            .iter_mut()
            .find(|session| session.session_id == "terminal")
            .unwrap()
            .status = SessionStatus::Crashed;
        let terminal = store.session_statuses.get_mut("terminal").unwrap();
        terminal.lifecycle = Lifecycle::Error;
        terminal.error_since = Some(2);
    });
    sidebar.update(&mut cx, |sidebar, cx| {
        let rows = sidebar.resolved_active_rows_for_tabs(&tabs, cx);
        assert_eq!(
            rows.iter()
                .filter(|row| row.group() == super::active::ActiveGroup::Terminals)
                .count(),
            1
        );
    });
    store.update(&mut cx, |store, _| {
        store
            .session_statuses
            .get_mut("terminal")
            .unwrap()
            .error_since = None;
        store.session_statuses.get_mut("chat").unwrap().unread_since = None;
    });
    sidebar.update(&mut cx, |sidebar, cx| {
        assert!(sidebar.resolved_active_rows_for_tabs(&tabs, cx).is_empty());
    });
}

#[test]
fn active_mixed_split_is_one_chat_row_targeting_the_working_chat() {
    use runner_core::protocol::status::{Activity, AgentStatus, Lifecycle};
    for (first, second, expected_label) in [
        ("chat", "terminal", "Chat topic"),
        ("terminal", "chat", "Shell command"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (pool, store) = seeded_store(&mut cx, temp.path());
        let conn = pool.get().unwrap();
        let layout = two_pane_layout(first, second);
        let node =
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
                .unwrap();
        drop(conn);
        store.update(&mut cx, |store, cx| {
            store.refresh_nodes(cx).unwrap();
            store.sessions = vec![
                direct_session(
                    second,
                    if second == "chat" { "codex" } else { "shell" },
                    SessionStatus::Running,
                ),
                direct_session(
                    first,
                    if first == "chat" { "codex" } else { "shell" },
                    SessionStatus::Running,
                ),
            ];
            store.session_statuses.insert(
                "chat".into(),
                AgentStatus {
                    lifecycle: Lifecycle::Running,
                    observation: runner_core::protocol::status::AgentObservation {
                        activity: Activity::Working,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
            store
                .session_activity
                .insert("terminal".into(), SessionActivityState::Busy);
        });
        let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
        let mut tabs = cx.update(|cx| TabSet::from_rows(&store.read(cx).nodes));
        tabs.activate(&node.id);
        for focused in ["chat", "terminal"] {
            tabs.active_mut().unwrap().focus_session(focused);
            sidebar.update(&mut cx, |sidebar, cx| {
                let rows = sidebar.resolved_active_rows_for_tabs(&tabs, cx);
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].group(), super::active::ActiveGroup::Chats);
                for row in &rows {
                    let (label, icon) = row.title_and_icon(|id| {
                        Some(
                            if id == "chat" {
                                "Chat topic"
                            } else {
                                "Shell command"
                            }
                            .into(),
                        )
                    });
                    assert_eq!(label, expected_label);
                    assert_eq!(icon.asset_path(), "columns-2.svg");
                    assert_eq!(
                        row.shortcut_row(&store.read(cx).nodes).target,
                        SidebarActivationTarget::Tab {
                            tab_id: node.id.clone(),
                            session_id: "chat".into()
                        }
                    );
                }
            });
        }
        tabs.active_mut().unwrap().name = Some("My mixed split".into());
        sidebar.update(&mut cx, |sidebar, cx| {
            for row in sidebar.resolved_active_rows_for_tabs(&tabs, cx) {
                assert_eq!(
                    row.title_and_icon(|_| Some("Later title".into())).0,
                    "My mixed split"
                );
            }
        });
    }
}

#[test]
fn remote_tab_rename_placeholder_uses_the_remote_layout_even_with_colliding_local_ids() {
    use crate::surfaces::remote_client::tests::{
        add_window, choose_host, enable_front_page, fixture,
    };
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for colliding_local_tab in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, clients) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        let window = add_window(&mut cx, &store, temp.path(), "remote-rename-placeholder");
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        clients.update(&mut cx, |clients, cx| {
            let mut overview = clients.overviews["host"].clone();
            overview.tabs[0].name.clear();
            overview.tabs[0].panes = serde_json::from_value(serde_json::json!({"split":"column","sizes":[0.5,0.5],"a":{"session":"s"},"b":{"session":null}})).unwrap();
            overview.sessions.get_mut("s").unwrap().title = "Host derived title".into();
            clients.apply_event(AppEvent {
                name: "remote/overview".into(),
                payload: serde_json::json!({"computer":"host","overview":overview}),
            });
            cx.notify();
        });
        cx.run_until_parked();
        if colliding_local_tab {
            window.update(&mut cx, |root, _, _| {
                let mut local = PaneLayout::single(Some("s"), &[]);
                local.split("p1", SplitOrientation::Column).unwrap();
                let row = serde_json::from_value(serde_json::json!({"id":"remote-tab","parent_id":null,"position":0,"type":"tab","name":"Local collision label","ref_id":null,"layout":local.serialize().unwrap(),"pinned_position":null,"last_completed_at":null,"last_viewed_at":null,"created_at":"2026-10-24T00:00:00Z"})).unwrap();
                root.tabs = TabSet::from_rows(&[row]);
            }).unwrap();
        }
        let sidebar = window
            .read_with(&cx, |root, _| root.front_page_sidebar().clone())
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.update(|window, cx| {
            window.activate_window();
            sidebar.update(cx, |sidebar, cx| {
                sidebar.handle_sidebar_menu_action(
                    SidebarMenuAction::Rename(SidebarRenameTarget::Tab {
                        node_id: "remote-tab".into(),
                        original: String::new(),
                        session_id: None,
                    }),
                    window,
                    cx,
                )
            });
        });
        visual.run_until_parked();
        sidebar.read_with(&visual, |sidebar, cx| {
            let field = sidebar.rename.as_ref().unwrap().input.read(cx);
            assert_eq!(field.placeholder(), "Host derived title");
            assert!(field.text().is_empty());
        });
        if colliding_local_tab {
            window
                .read_with(&visual, |root, _| {
                    assert_eq!(
                        root.tabs.tabs()[0].name.as_deref(),
                        Some("Local collision label")
                    );
                })
                .unwrap();
        }
    }
}
