use super::*;
use crate::surfaces::remote_client::tests::{
    self as remote_test, action_trace, add_window, choose_host, Fake,
};
use gpui::{TestAppContext, VisualTestContext};
use std::sync::Arc;

fn catalog() -> serde_json::Value {
    serde_json::json!({"roles":[{"id":"host-role","name":"Host coder","handle":"host-coder","runtime":"codex","model":"host-model","effort":"high","speed":null}],"runtimes":[{"name":"codex","label":"Codex","available":true,"nativeFork":true,"defaultModel":null,"defaultEffort":null,"models":[],"efforts":[],"speeds":[]}],"accounts":[],"activeAccount":"","crews":[{"id":"empty-host-crew","name":"Empty host crew","slots":[]},{"id":"host-crew","name":"Host duo","slots":[{"handle":"lead","role":"host-role","lead":true},{"handle":"reviewer","role":"host-role","lead":false}]}],"projects":[{"id":"host-project","name":"Host project","path":"\\\\nas\\share"}],"recentFolders":[]})
}

fn overview() -> runner_core::companion::Overview {
    serde_json::from_value(serde_json::json!({"projects":[{"id":"host-project","name":"Host project"}],"tabs":[{"id":"first","project":null,"name":"First","pinned":false,"panes":{"session":null},"shells":[]}],"missions":[],"sessions":{}})).unwrap()
}

fn setup(
    features: Option<&[&str]>,
) -> (
    tempfile::TempDir,
    TestAppContext,
    gpui::WindowHandle<NativeRoot>,
    Arc<Fake>,
) {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
    remote_test::enable_missions(&fake);
    if let Some(features) = features {
        fake.set_features(features);
    }
    fake.set_overview(overview());
    let window = add_window(&mut cx, &store, temp.path(), "host-mission-launcher");
    window.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Tab("first".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();
    (temp, cx, window, fake)
}

fn launcher(
    root: &mut NativeRoot,
    window: &mut Window,
    cx: &mut Context<NativeRoot>,
) -> Entity<ChatLauncher> {
    root.render_launcher(None, true, window, cx)
}

#[test]
fn the_crew_step_lists_the_hosts_crews_and_starts_the_mission_there() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, fake) = setup(None);
    fake.queue_result("catalog", Ok(catalog()));
    window
        .update(&mut cx, |root, window, cx| {
            root.new_mission_action(&NewMission, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let view = window
        .update(&mut cx, |root, window, cx| {
            root.new_mission_action(&NewMission, window, cx);
            let view = launcher(root, window, cx);
            assert!(view
                .read(cx)
                .crews
                .iter()
                .flat_map(|crew| crew.slots.iter().flatten())
                .all(|slot| slot.setup.is_empty()));
            let rows = view.read(cx).rows(cx).0;
            let choices = rows
                .iter()
                .map(|row| row.choice.clone())
                .collect::<Vec<_>>();
            assert_eq!(
                choices,
                [
                    Choice::Crew("empty-host-crew".into()),
                    Choice::Crew("host-crew".into()),
                ]
            );
            assert!(rows[0].dimmed);
            assert!(!rows[1].dimmed);
            view.update(cx, |launcher, cx| {
                launcher.choose_project("host-project", cx);
                launcher.start(Choice::Crew("host-crew".into()), false, window, cx);
                launcher.goal.update(cx, |goal, cx| {
                    goal.set_text("\nShip the host change\nthen review", cx)
                });
            });
            view
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    assert!(visual.debug_bounds("LAUNCHER_SLOT lead").is_some());
    assert!(visual.debug_bounds("LAUNCHER_SLOT reviewer").is_some());
    let mut size = (0, 0);
    window
        .update(&mut visual, |root, window, cx| {
            size = root
                .remote_client
                .workspace()
                .unwrap()
                .mission_workspace
                .read(cx)
                .estimated_mission_terminal_size(window, cx);
            view.update(cx, |launcher, cx| launcher.start_mission(window, cx));
        })
        .unwrap();
    visual.run_until_parked();
    let starts = action_trace(&fake, "startMission");
    assert_eq!(starts.len(), 1);
    let request = &starts[0];
    assert_eq!(request["crew"], "host-crew");
    assert_eq!(request["title"], "Ship the host change");
    assert_eq!(request["goal"], "Ship the host change\nthen review");
    assert_eq!(request["project"], "host-project");
    assert_eq!(request["cwd"], r"\\nas\share");
    assert_eq!(request["cols"], size.0.max(20));
    assert_eq!(request["rows"], size.1.max(8));
    assert!(action_trace(&fake, "archive").is_empty());
    assert!(action_trace(&fake, "layout").is_empty());
    window
        .read_with(&visual, |root, _| {
            let remote = root.remote_client.workspace().unwrap();
            assert_eq!(remote.activate_mission.as_deref(), Some("created-mission"));
        })
        .unwrap();
}

#[test]
fn without_catalog_and_start_mission_no_entry_point_opens_a_launcher() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for features in [vec!["catalog", "layout"], vec!["startMission", "layout"]] {
        let (_temp, mut cx, window, fake) = setup(Some(&features));
        let layouts = fake.request_count("layout");
        window
            .update(&mut cx, |root, window, cx| {
                let view = launcher(root, window, cx);
                assert!(view
                    .read(cx)
                    .rows(cx)
                    .0
                    .iter()
                    .all(|row| row.choice != Choice::Mission));
                root.new_mission_action(&NewMission, window, cx);
                root.open_mission_launcher(
                    Some("host-crew".into()),
                    ProjectScope::Root,
                    window,
                    cx,
                );
                assert!(!root.launcher_page_visible());
                assert_eq!(view.read(cx).step, Step::Start);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(fake.request_count("layout"), layouts);
        assert_eq!(fake.request_count("startMission"), 0);
    }
}

#[test]
fn remote_empty_missions_are_unavailable_in_rows_keys_and_start_requests() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, fake) = setup(None);
    let view = window
        .update(&mut cx, |root, window, cx| {
            root.new_mission_action(&NewMission, window, cx);
            launcher(root, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_keystrokes("shift-enter");
    visual.run_until_parked();
    window
        .update(&mut visual, |_, window, cx| {
            view.update(cx, |launcher, cx| {
                assert_eq!(launcher.step, Step::Crews);
                assert!(launcher
                    .rows(cx)
                    .0
                    .iter()
                    .all(|row| row.choice != Choice::EmptyMission));
                launcher.start(Choice::EmptyMission, false, window, cx);
                assert_eq!(launcher.step, Step::Crews);
                launcher.open_step(Step::Goal(None), window, cx);
                launcher.start_mission(window, cx);
                assert!(launcher.pending.is_none());
            });
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(fake.request_count("startMission"), 0);
}

#[test]
fn new_chat_page_sends_only_the_start_request_when_a_chat_is_chosen() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, fake) = setup(None);
    window
        .update(&mut cx, |root, window, cx| {
            root.open_new_tab_modal(&NewTab, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert!(action_trace(&fake, "layout").is_empty());
    window
        .update(&mut cx, |root, window, cx| {
            assert!(root.launcher_page_visible());
            assert!(root.front_page_layout().is_none());
            root.front_page_sidebar().update(cx, |sidebar, _| {
                let settings = sidebar.remote_settings.as_mut().unwrap();
                settings.sidebar_chats_open = false;
                settings.sidebar_projects_open = false;
            });
            launcher(root, window, cx).update(cx, |launcher, cx| {
                launcher.start(
                    Choice::Chat(EntryKey::Agent("codex".into())),
                    false,
                    window,
                    cx,
                )
            });
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.request_count("startChat"), 1);
    assert!(action_trace(&fake, "layout").is_empty());
    assert!(action_trace(&fake, "archive").is_empty());
    window
        .read_with(&cx, |root, cx| {
            assert_eq!(
                root.remote_client
                    .workspace()
                    .unwrap()
                    .activate_session
                    .as_deref(),
                Some("started-session")
            );
            let sidebar = root.front_page_sidebar();
            let settings = sidebar.read(cx).remote_settings.as_ref().unwrap();
            assert!(settings.sidebar_chats_open);
            assert!(!settings.sidebar_projects_open);
        })
        .unwrap();
}

/// The row, read with the root free, since the sidebar reads it back.
fn host_row(window: &gpui::WindowHandle<NativeRoot>, cx: &TestAppContext) -> Option<ProjectScope> {
    window
        .read_with(cx, |root, cx| {
            root.front_page_sidebar().read(cx).launcher_row_scope(cx)
        })
        .unwrap()
}

fn host_page(
    window: &gpui::WindowHandle<NativeRoot>,
    cx: &mut TestAppContext,
) -> (ProjectScope, bool, String, bool) {
    window
        .update(cx, |root, window, cx| {
            let page = launcher(root, window, cx);
            let page = page.read(cx);
            let select = page.project.select();
            (
                page.scope(),
                select.read(cx).is_disabled(),
                page.cwd.read(cx).text().to_owned(),
                page.loading,
            )
        })
        .unwrap()
}

#[test]
fn new_tab_uses_the_hosts_opening_tab_project_and_picker_close_restores_navigation() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, fake) = setup(None);
    let mut data = overview();
    data.tabs[0].project = Some("host-project".into());
    let mut recent = data.tabs[0].clone();
    recent.id = "recent".into();
    data.tabs.push(recent.clone());
    fake.set_overview(data);
    cx.run_until_parked();
    fake.queue_result("catalog", Ok(catalog()));
    window
        .update(&mut cx, |root, window, cx| {
            root.select_remote_item(RemoteItem::Tab("recent".into()), cx);
            root.select_remote_item(RemoteItem::Tab("first".into()), cx);
            root.open_new_tab_modal(&NewTab, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        host_page(&window, &mut cx),
        (
            ProjectScope::Project("host-project".into()),
            false,
            r"\\nas\share".into(),
            false
        )
    );
    assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
    window
        .read_with(&cx, |root, cx| assert_eq!(root.active_project_id(cx), None))
        .unwrap();

    for item in [RemoteItem::Roles, RemoteItem::Crews] {
        fake.queue_result("catalog", Ok(catalog()));
        window
            .update(&mut cx, |root, window, cx| {
                root.select_remote_item(item, cx);
                root.open_new_tab_modal(&NewTab, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let first = host_page(&window, &mut cx);
        assert_eq!(
            first,
            (
                ProjectScope::Project("host-project".into()),
                false,
                r"\\nas\share".into(),
                false
            )
        );
        window
            .update(&mut cx, |root, window, cx| {
                root.open_new_tab_modal(&NewTab, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(host_page(&window, &mut cx), first);
    }

    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    for close in ["mouse", "keyboard", "escape"] {
        let picker = visual
            .debug_bounds("STYLED_SELECT_launcher-project")
            .unwrap();
        visual.simulate_click(picker.center(), Default::default());
        visual.run_until_parked();
        match close {
            "mouse" => {
                let option = visual.debug_bounds("STYLED_SELECT_OPTION_0").unwrap();
                visual.simulate_click(option.center(), Default::default());
            }
            "keyboard" => visual.simulate_keystrokes("end enter"),
            _ => visual.simulate_keystrokes("escape"),
        }
        visual.run_until_parked();
        let selected = window
            .update(&mut visual, |root, window, cx| {
                let page = launcher(root, window, cx);
                assert!(
                    page.read(cx)
                        .query
                        .read(cx)
                        .focus_handle()
                        .is_focused(window),
                    "{close}"
                );
                page.read(cx).selected
            })
            .unwrap();
        visual.simulate_keystrokes("down");
        visual.run_until_parked();
        window
            .update(&mut visual, |root, window, cx| {
                assert_ne!(
                    launcher(root, window, cx).read(cx).selected,
                    selected,
                    "{close}"
                );
            })
            .unwrap();
    }
    window
        .update(&mut visual, |root, window, cx| {
            launcher(root, window, cx).update(cx, |page, cx| page.choose_project("", cx));
            root.open_new_tab_modal(&NewTab, window, cx);
        })
        .unwrap();
    visual.run_until_parked();
    window
        .update(&mut visual, |root, window, cx| {
            assert_eq!(
                launcher(root, window, cx).read(cx).scope(),
                ProjectScope::Project("host-project".into())
            );
            root.select_remote_item(RemoteItem::Tab("first".into()), cx);
        })
        .unwrap();
    let mut no_project = overview();
    no_project.tabs.push(recent.clone());
    fake.set_overview(no_project);
    visual.run_until_parked();
    window
        .update(&mut visual, |root, window, cx| {
            root.open_new_tab_modal(&NewTab, window, cx);
            assert_eq!(
                launcher(root, window, cx).read(cx).scope(),
                ProjectScope::Root
            );
        })
        .unwrap();

    let mut remaining = overview();
    remaining.tabs = vec![recent];
    fake.set_overview(remaining);
    visual.run_until_parked();
    window
        .update(&mut visual, |root, window, cx| {
            root.open_new_tab_modal(&NewTab, window, cx);
            assert_eq!(
                launcher(root, window, cx).read(cx).scope(),
                ProjectScope::Project("host-project".into())
            );
        })
        .unwrap();
}

#[test]
fn a_locked_page_keeps_its_row_while_the_hosts_catalog_loads_and_ends_on_its_project() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, fake) = setup(None);
    let project = ProjectScope::Project("host-project".into());
    fake.queue_result("catalog", Ok(catalog()));
    window
        .update(&mut cx, |root, window, cx| {
            root.open_sidebar_chat_modal(project.clone(), window, cx);
            assert!(launcher(root, window, cx).read(cx).loading);
        })
        .unwrap();
    assert_eq!(host_row(&window, &cx), Some(project.clone()));
    cx.run_until_parked();
    assert_eq!(host_row(&window, &cx), Some(project.clone()));
    assert_eq!(
        host_page(&window, &mut cx),
        (project.clone(), true, "\\\\nas\\share".to_owned(), false)
    );

    // ⌘N takes the row to Recents and frees the picker.
    window
        .update(&mut cx, |root, window, cx| {
            root.open_new_tab_modal(&NewTab, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
    assert_eq!(
        host_page(&window, &mut cx),
        (ProjectScope::Root, false, String::new(), false)
    );

    // A project deleted on the host gives a locked page back.
    window
        .update(&mut cx, |root, window, cx| {
            root.open_sidebar_chat_modal(project.clone(), window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert!(host_page(&window, &mut cx).1);
    let mut gone = overview();
    gone.projects.clear();
    fake.set_overview(gone);
    cx.run_until_parked();
    assert_eq!(
        host_page(&window, &mut cx),
        (ProjectScope::Root, false, String::new(), false)
    );
    cx.run_until_parked();
    assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
}

#[test]
fn a_mission_entry_frees_a_locked_host_page_and_keeps_its_project() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, fake) = setup(None);
    let project = ProjectScope::Project("host-project".into());
    fake.queue_result("catalog", Ok(catalog()));
    window
        .update(&mut cx, |root, window, cx| {
            root.open_sidebar_chat_modal(project.clone(), window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(host_row(&window, &cx), Some(project.clone()));
    window
        .update(&mut cx, |root, window, cx| {
            root.new_mission_action(&NewMission, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
    assert_eq!(
        host_page(&window, &mut cx),
        (project, false, "\\\\nas\\share".to_owned(), false)
    );
}

#[test]
fn a_reconnect_keeps_a_locked_page_on_its_row_and_project_until_the_project_is_deleted() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, fake) = setup(None);
    let project = ProjectScope::Project("host-project".into());
    let path = "\\\\nas\\share".to_owned();
    fake.queue_result("catalog", Ok(catalog()));
    window
        .update(&mut cx, |root, window, cx| {
            root.open_sidebar_chat_modal(project.clone(), window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        host_page(&window, &mut cx),
        (project.clone(), true, path.clone(), false)
    );
    let before = window
        .read_with(&cx, |root, cx| root.front_page_client(cx).target)
        .unwrap();

    // The connection's generation moves on, and the page's launcher with it.
    for _ in 0..4 {
        fake.queue_result("catalog", Ok(catalog()));
    }
    window
        .update(&mut cx, |root, _, cx| {
            let clients = root.app_store.read(cx).remote_clients.clone();
            clients.update(cx, |clients, _| clients.invalidate_capabilities("host"));
            root.sync_remote_state(cx);
            assert_ne!(root.front_page_client(cx).target, before);
        })
        .unwrap();
    assert_eq!(host_row(&window, &cx), Some(project.clone()));

    // Rebuilt, it holds the row and the lock before the host's capabilities
    // are back, then while its catalog loads.
    let (scope, locked, _, _) = host_page(&window, &mut cx);
    assert_eq!((scope, locked), (project.clone(), true));
    assert_eq!(host_row(&window, &cx), Some(project.clone()));
    window
        .read_with(&cx, |root, cx| {
            let target = root.front_page_client(cx).target;
            assert!(root
                .launchers
                .values()
                .filter(|launcher| launcher.read(cx).pane.is_none())
                .all(|launcher| launcher.read(cx).client.target == target));
        })
        .unwrap();
    let mut saw_catalog_loading = false;
    while cx.background_executor.tick() {
        let (scope, locked, _, loading) = host_page(&window, &mut cx);
        assert_eq!((scope, locked), (project.clone(), true));
        assert_eq!(host_row(&window, &cx), Some(project.clone()));
        saw_catalog_loading |= loading;
    }
    assert!(saw_catalog_loading);

    cx.run_until_parked();
    assert_eq!(
        host_page(&window, &mut cx),
        (project.clone(), true, path, false)
    );
    assert_eq!(host_row(&window, &cx), Some(project));

    // A deleted project still gives the page back.
    let mut gone = overview();
    gone.projects.clear();
    fake.set_overview(gone);
    cx.run_until_parked();
    assert_eq!(
        host_page(&window, &mut cx),
        (ProjectScope::Root, false, String::new(), false)
    );
    cx.run_until_parked();
    assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
}

#[test]
fn a_reconnect_does_not_bring_an_old_lock_back_into_a_new_page_or_the_empty_state() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, fake) = setup(None);
    let project = ProjectScope::Project("host-project".into());
    for _ in 0..4 {
        fake.queue_result("catalog", Ok(catalog()));
    }
    let reconnect = |window: &gpui::WindowHandle<NativeRoot>, cx: &mut TestAppContext| {
        window
            .update(cx, |root, _, cx| {
                let clients = root.app_store.read(cx).remote_clients.clone();
                clients.update(cx, |clients, _| clients.invalidate_capabilities("host"));
                root.sync_remote_state(cx);
            })
            .unwrap();
    };
    // Left under a project, and replaced while another tab shows.
    window
        .update(&mut cx, |root, window, cx| {
            root.open_sidebar_chat_modal(project.clone(), window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Tab("first".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();
    reconnect(&window, &mut cx);
    window
        .update(&mut cx, |root, window, cx| {
            root.prune_launchers(cx);
            root.open_new_tab_modal(&NewTab, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
    let (scope, locked, _, _) = host_page(&window, &mut cx);
    assert_eq!((scope, locked), (ProjectScope::Root, false));

    // With no tab or mission left, the empty state starts clean too.
    window
        .update(&mut cx, |root, window, cx| {
            root.open_sidebar_chat_modal(project.clone(), window, cx);
            root.select_remote_item(RemoteItem::Tab("first".into()), cx);
        })
        .unwrap();
    cx.run_until_parked();
    reconnect(&window, &mut cx);
    let mut empty = overview();
    empty.tabs.clear();
    fake.set_overview(empty);
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| assert!(root.launcher_is_empty_state()))
        .unwrap();
    assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
    let (scope, locked, _, _) = host_page(&window, &mut cx);
    assert_eq!((scope, locked), (ProjectScope::Root, false));
}

#[test]
fn a_catalog_without_the_locked_project_leaves_the_page_in_recents_unlocked() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, fake) = setup(None);
    let mut without = catalog();
    without["projects"] = serde_json::json!([]);
    fake.queue_result("catalog", Ok(without));
    window
        .update(&mut cx, |root, window, cx| {
            root.open_sidebar_chat_modal(ProjectScope::Project("host-project".into()), window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        host_page(&window, &mut cx),
        (ProjectScope::Root, false, String::new(), false)
    );
    assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
}

#[test]
fn a_closed_host_tab_lands_on_the_last_viewed_one_and_guards_the_next_close() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, fake) = setup(None);
    let tab = |id: &str| -> runner_core::companion::Tab {
        serde_json::from_value(serde_json::json!({"id":id,"project":null,"name":id,"pinned":false,"panes":{"session":null},"shells":[]})).unwrap()
    };
    let mut three = overview();
    three.tabs.extend([tab("second"), tab("third")]);
    fake.set_overview(three.clone());
    cx.run_until_parked();
    let select = |cx: &mut TestAppContext, item: &str| {
        window
            .update(cx, |root, _, cx| {
                root.select_remote_item(RemoteItem::Tab(item.into()), cx)
            })
            .unwrap();
        cx.run_until_parked();
    };
    select(&mut cx, "third");
    select(&mut cx, "first");
    window
        .update(&mut cx, |root, _, _| {
            root.remote_client.workspace_mut().unwrap().landed_at = None;
            assert!(!root.tab_close_guarded());
        })
        .unwrap();
    three.tabs.remove(0);
    fake.set_overview(three);
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| {
            assert!(root.remote_client.active_item() == Some(&RemoteItem::Tab("third".into())));
            assert!(root.tab_close_guarded());
        })
        .unwrap();
    window
        .update(&mut cx, |root, window, cx| {
            crate::surfaces::windowing::close_tab(root, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert!(action_trace(&fake, "archive").is_empty());
    window
        .update(&mut cx, |root, window, cx| {
            root.remote_client.workspace_mut().unwrap().landed_at = None;
            crate::surfaces::windowing::close_tab(root, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(action_trace(&fake, "archive").len(), 1);
}

#[test]
fn switching_computers_after_showing_the_page_never_creates_or_archives_a_tab() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, fake) = setup(None);
    window
        .update(&mut cx, |root, window, cx| {
            root.open_new_tab_modal(&NewTab, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, window, cx| {
            root.return_to_this_computer(window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    window.update(&mut cx, choose_host).unwrap();
    cx.run_until_parked();
    assert!(action_trace(&fake, "layout").is_empty());
    assert!(action_trace(&fake, "archive").is_empty());
}

#[test]
fn a_host_crew_page_start_shows_the_launcher_page_on_that_crews_goal_step() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, fake) = setup(None);
    fake.queue_result("catalog", Ok(catalog()));
    window
        .update(&mut cx, |root, window, cx| root.open_crews(window, cx))
        .unwrap();
    cx.run_until_parked();
    fake.queue_result("catalog", Ok(catalog()));
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let start = visual.debug_bounds("CREW_ROW_START:host-crew").unwrap();
    visual.simulate_click(start.center(), Default::default());
    visual.run_until_parked();
    window
        .read_with(&visual, |root, cx| {
            assert!(matches!(
                root.remote_client.active_item(),
                Some(RemoteItem::Launcher)
            ));
            assert!(root.mission_launcher_open(cx));
        })
        .unwrap();
    assert_eq!(action_trace(&fake, "layout").len(), 0);
    assert_eq!(fake.request_count("startMission"), 0);
}
