use super::archive::archive_session_plan;
use super::archive::pane_close_after_archive;
use super::archive::take_pending_pane_closes;
use super::elements::chat_tab_row_active;
use super::elements::command_held_alone;
use super::elements::default_session_label_parts;
use super::elements::other_modifiers_held;
use super::elements::sidebar_fork_menu_target;
use super::elements::tab_label_live;
use super::menus::mission_menu_entries;
use super::menus::project_create_menu_entries;
use super::menus::project_menu_entries;
use super::menus::sidebar_create_menu_entries;
use super::menus::sidebar_tab_icon;
use super::menus::tab_menu_entries;
use super::view::sidebar_scroll_container;
use super::view::sidebar_scroll_frame;
use super::*;
use gpui::{
    prelude::*, size, Context, Render, ScrollHandle, TestAppContext, VisualTestContext, Window,
};
use runner_backend::events::AppEvent;

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
                    .on_click(|_, window, _| window.blur())
                    .debug_selector(|| "OUTSIDE_RENAME".into()),
            )
    }
}

#[test]
fn mission_rename_commits_when_the_field_loses_focus() {
    use runner_backend::{db, event_bus, events, mcp, router, session, shell_path, windows};
    use std::sync::{Mutex, RwLock};

    let temp = tempfile::tempdir().unwrap();
    let pool = Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap());
    pool.get()
        .unwrap()
        .execute_batch(
            "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('crew', 'Crew', '2026-09-06T00:00:00Z', '2026-09-06T00:00:00Z');
                 INSERT INTO missions (id, crew_id, title, status, started_at)
                 VALUES ('mission', 'crew', 'Original mission', 'completed', '2026-09-06T00:00:00Z');",
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
            core,
            temp.path().join("settings.json"),
            AppSettings::default(),
            None,
            cx,
        )
    });
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

fn direct_session(id: &str, runtime: &str, status: SessionStatus) -> DirectSessionEntry {
    DirectSessionEntry {
        session_id: id.into(),
        project_id: None,
        runner_id: None,
        handle: None,
        agent_runtime: runtime.into(),
        agent_command: runtime.into(),
        display_name: runtime.into(),
        status,
        title: None,
        live_title: None,
        cwd: None,
        started_at: None,
        stopped_at: None,
        resumable: false,
        native_fork: matches!(runtime, "codex" | "claude-code"),
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
            tab_label_live(&layout, &[entry.clone()], |_| live.map(str::to_owned)),
            expected
        );
    }
    entry.live_title = Some("Old topic".into());
    assert_eq!(
        tab_label_live(&layout, &[entry.clone()], |_| Some("New topic".into())),
        "New topic"
    );
    entry.title = Some("My chat".into());
    assert_eq!(
        tab_label_live(&layout, &[entry], |_| Some("New topic".into())),
        "My chat"
    );
}

#[test]
fn split_tab_titles_follow_layout_order_and_manual_names_instead_of_focus() {
    let first = direct_session("first", "codex", SessionStatus::Running);
    let second = direct_session("second", "claude-code", SessionStatus::Running);
    let sessions = [second, first];
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
        name: "event/appended",
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
            WorkspaceEntry::Runner,
            WorkspaceEntry::Crew,
        ]
    );
    assert!(!WorkspaceEntry::NewTab.selectable());
    assert!(!WorkspaceEntry::NewTab.selected(&AppRoute::Runners));
    assert!(!WorkspaceEntry::NewTab.selected(&AppRoute::Crews));
    assert!(!WorkspaceEntry::NewTab.selected(&AppRoute::Settings));
    assert!(WorkspaceEntry::Runner.selectable());
    assert!(WorkspaceEntry::Crew.selectable());
}

#[test]
fn shortcut_pills_use_the_platform_primary_modifier_alone() {
    let mut modifiers = gpui::Modifiers {
        control: cfg!(windows),
        platform: !cfg!(windows),
        ..Default::default()
    };
    assert!(command_held_alone(modifiers));
    assert!(!other_modifiers_held(modifiers));
    modifiers.shift = true;
    assert!(!command_held_alone(modifiers));
    assert!(other_modifiers_held(modifiers));
    assert!(!command_held_alone(gpui::Modifiers {
        control: !cfg!(windows),
        platform: cfg!(windows),
        ..Default::default()
    }));
}

#[test]
fn project_create_menu_uses_short_labels_and_project_targets() {
    let root_entries = sidebar_create_menu_entries();
    assert_eq!(
        menu_labels(&root_entries),
        ["New chat", "New terminal", "New mission"]
    );
    assert_eq!(
        root_entries
            .iter()
            .map(|(_, action)| action.clone())
            .collect::<Vec<_>>(),
        [
            SidebarMenuAction::NewChat(None),
            SidebarMenuAction::NewTerminal(None),
            SidebarMenuAction::NewMission(None),
        ]
    );

    let entries = project_create_menu_entries("project-1");
    assert_eq!(
        menu_labels(&entries),
        ["New chat", "New terminal", "New mission"]
    );
    assert_eq!(
        entries
            .iter()
            .map(|(_, action)| action.clone())
            .collect::<Vec<_>>(),
        [
            SidebarMenuAction::NewChat(Some("project-1".into())),
            SidebarMenuAction::NewTerminal(Some("project-1".into())),
            SidebarMenuAction::NewMission(Some("project-1".into())),
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
    );
    assert_eq!(
        menu_labels(&multi_pane_entries),
        ["Pin", "Rename tab", "Archive all"]
    );
    assert!(multi_pane_entries[2].0.destructive);

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
    );
    assert_eq!(
        menu_labels(&terminal_only_entries),
        ["Pin", "Rename tab", "Close terminal"]
    );
    assert!(terminal_only_entries[2].0.destructive);
    assert_eq!(
        terminal_only_entries[2].1,
        SidebarMenuAction::CloseTerminalTab {
            tab_id: "tab-1".into(),
            session_id: "shell-1".into(),
        }
    );
    assert_eq!(sidebar_tab_icon(1, Some("shell")), "square-terminal.svg");
    assert_eq!(sidebar_tab_icon(1, Some("codex")), "message-square.svg");
    assert_eq!(sidebar_tab_icon(2, Some("shell")), "columns-2.svg");
    assert_eq!(sidebar_tab_icon(3, Some("shell")), "columns-3.svg");

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
        ),
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
        ),
        Some("This archives 2 chats and permanently closes 2 terminals. Archived chats can be restored from Settings → Archived; closed terminals cannot.".into()),
    );
    assert_eq!(
        archive_all_confirmation_body(&["shell-1".into()], &sessions),
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
    assert_eq!(target.disabled_reason, None);

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
    );
    assert_eq!(
        menu_labels(&entries),
        ["Pin", "Rename tab", "Fork chat", "Archive"]
    );
    assert!(!entries[2].0.disabled);
    assert_eq!(entries[2].1, SidebarMenuAction::ForkChat("chat".into()));

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
    );
    assert!(busy_entries[2].0.disabled);

    let mut waiting = direct_session("chat", "codex", SessionStatus::Running);
    waiting.native_fork = true;
    let waiting_members = vec![waiting];
    let waiting_target = sidebar_fork_menu_target(&layout, &waiting_members).unwrap();
    assert_eq!(
        waiting_target.disabled_reason,
        Some("No session key captured yet")
    );
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
    );
    assert!(waiting_entries[2].0.disabled);
    assert_eq!(
        waiting_entries[2].0.description.clone(),
        Some("No session key captured yet".into())
    );

    let trae_members = vec![direct_session("chat", "trae", SessionStatus::Running)];
    let trae_target = sidebar_fork_menu_target(&layout, &trae_members).unwrap();
    assert_eq!(
        trae_target.disabled_reason,
        Some("Forking needs claude-code or codex")
    );
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
    );
    assert!(trae_entries[2].0.disabled);
    assert_eq!(
        trae_entries[2].0.description.clone(),
        Some("Forking needs claude-code or codex".into())
    );

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
        "ask_human",
        "human_question",
        "human_response",
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
            name: "event/appended",
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
    let max_offset = f32::from(scroll.max_offset().height);
    let block_wrapper_viewport = f32::from(block_wrapper_scroll.bounds().size.height);
    let block_wrapper_max_offset = f32::from(block_wrapper_scroll.max_offset().height);
    let constrained_section_max_offset = f32::from(constrained_section_scroll.max_offset().height);
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
