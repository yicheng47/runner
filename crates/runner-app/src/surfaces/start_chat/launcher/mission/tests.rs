use super::*;
use crate::surfaces::start_chat::tests::{modal_harness, project_fixture, ModalHarness};

fn harness() -> ModalHarness {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    harness.act(|root, _, cx| {
        root.start_chat_modal = None;
        root.set_route(AppRoute::Chat, cx);
    });
    harness
}

/// A crew with one codex role per handle; none leaves it without roles.
fn crew(root: &NativeRoot, name: &str, handles: &[&str], cx: &App) -> String {
    let core = root.core(cx);
    let crew = core
        .crew_create(runner_core::protocol::crew::CreateCrewInput {
            name: name.into(),
            ..Default::default()
        })
        .unwrap();
    for handle in handles {
        let role = core
                .role_create(
                    serde_json::from_value(serde_json::json!({
                        "handle": format!("{}-{handle}", name.split(' ').next().unwrap().to_lowercase()),
                        "display_name": format!("Role {handle}"),
                        "runtime": "codex",
                        "command": "codex",
                    }))
                    .unwrap(),
                )
                .unwrap();
        core.slot_create(
            serde_json::from_value(serde_json::json!({
                "crew_id": crew.id,
                "role_id": role.id,
                "slot_handle": handle,
            }))
            .unwrap(),
        )
        .unwrap();
    }
    crew.id
}

fn active_launcher(root: &NativeRoot, cx: &App) -> Entity<ChatLauncher> {
    root.launchers
        .values()
        .find(|launcher| {
            let launcher = launcher.read(cx);
            launcher.pane.is_none() && launcher.client.target == root.front_page_client(cx).target
        })
        .cloned()
        .expect("the launcher page")
}

fn step(root: &NativeRoot, cx: &App) -> Step {
    active_launcher(root, cx).read(cx).step.clone()
}

fn choices(root: &NativeRoot, cx: &App) -> Vec<Choice> {
    let launcher = active_launcher(root, cx);
    let launcher = launcher.read(cx);
    launcher
        .rows(cx)
        .0
        .into_iter()
        .map(|row| row.choice)
        .collect()
}

fn shown(harness: &mut ModalHarness, selector: &'static str) -> bool {
    harness.visual.debug_bounds(selector).is_some()
}

#[test]
fn mission_entry_points_show_a_page_and_dismiss_back_through_steps() {
    let mut harness = harness();
    harness.act(|root, window, cx| root.open_crews(window, cx));
    harness.act(|root, window, cx| root.new_mission_action(&NewMission, window, cx));
    harness.act(|root, _, cx| {
        assert_eq!(root.route, AppRoute::Launcher);
        assert!(root.tabs.tabs().is_empty());
        assert_eq!(step(root, cx), Step::Crews);
        assert_eq!(choices(root, cx).last(), Some(&Choice::EmptyMission));
    });
    assert!(shown(&mut harness, "LAUNCHER_BRAND"));
    assert!(!shown(&mut harness, "SIDEBAR_ROW sidebar-launcher-tab"));
    harness.visual.simulate_input("find a crew");
    harness.visual.simulate_keystrokes("escape");
    harness.visual.run_until_parked();
    harness.act(|root, _, cx| assert_eq!(step(root, cx), Step::Start));
    harness.visual.simulate_keystrokes("escape");
    harness.visual.run_until_parked();
    harness.act(|root, _, _| assert_eq!(root.route, AppRoute::Crews));
}

#[test]
fn new_chat_and_mission_reuse_the_page_and_escape_restores_the_previous_tab() {
    let mut harness = harness();
    let mut previous = String::new();
    harness.act(|root, window, cx| {
        root.test_open_empty_tab(window, cx);
        previous = root.tabs.active_tab_id().unwrap().to_owned();
        root.open_new_tab_modal(&NewTab, window, cx);
    });
    harness.act(|root, window, cx| {
        assert_eq!(root.route, AppRoute::Launcher);
        assert!(root.front_page_layout().is_none());
        assert_eq!(root.tabs.tabs().len(), 1);
        active_launcher(root, cx).update(cx, |launcher, cx| {
            launcher.start(Choice::Mission, false, window, cx)
        });
    });
    harness.visual.simulate_keystrokes("backspace");
    harness.visual.run_until_parked();
    harness.act(|root, _, cx| assert_eq!(step(root, cx), Step::Start));
    harness.visual.simulate_input("a search draft");
    harness.visual.simulate_keystrokes("escape");
    harness.visual.run_until_parked();
    harness.act(|root, _, _| {
        assert_eq!(root.route, AppRoute::Chat);
        assert_eq!(root.tabs.active_tab_id(), Some(previous.as_str()));
        assert_eq!(root.tabs.tabs().len(), 1);
    });
}

#[test]
fn a_split_keeps_its_panes_and_mission_and_new_chat_show_the_launcher_page() {
    let mut harness = harness();
    let mut previous = String::new();
    harness.act(|root, window, cx| {
        root.test_open_empty_tab(window, cx);
        let pane = root.tabs.active().unwrap().focused_pane_id.clone();
        root.split_pane(&pane, SplitOrientation::Row, window, cx);
        let layout = root.tabs.active().unwrap().clone();
        previous = layout.id.clone();
        let view = root.render_launcher(
            Some(PaneKey::new(&layout.id, &layout.focused_pane_id)),
            false,
            window,
            cx,
        );
        assert!(view
            .read(cx)
            .rows(cx)
            .0
            .iter()
            .all(|row| row.choice != Choice::Mission));
    });
    assert!(!shown(&mut harness, "LAUNCHER_BRAND"));
    harness.act(|root, window, cx| root.new_mission_action(&NewMission, window, cx));
    harness.act(|root, _, cx| {
        assert_eq!(step(root, cx), Step::Crews);
        assert!(root.front_page_layout().is_none());
        assert_eq!(root.tabs.tabs().len(), 1);
        assert_eq!(root.tabs.active().unwrap().root.leaves().len(), 2);
    });
    assert!(shown(&mut harness, "LAUNCHER_BRAND"));
    harness.visual.simulate_keystrokes("escape escape");
    harness.visual.run_until_parked();
    harness.act(|root, _, _| {
        assert_eq!(root.tabs.active_tab_id(), Some(previous.as_str()));
        assert_eq!(root.route, AppRoute::Chat);
    });
    harness
        .visual
        .simulate_keystrokes(&keymap::platform_default("cmd-n"));
    harness.visual.run_until_parked();
    harness.act(|root, _, _| {
        assert_eq!(root.route, AppRoute::Launcher);
        assert_eq!(root.tabs.tabs().len(), 1);
    });
}

#[test]
fn project_choices_and_folder_changes_save_no_launcher_tab_on_any_step() {
    let mut harness = harness();
    let projects = project_fixture(&mut harness);
    harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
    for (next, project) in [
        (Step::Start, projects[0].id.as_str()),
        (Step::Crews, projects[1].id.as_str()),
        (Step::Goal(None), ""),
    ] {
        harness.act(|root, window, cx| {
            active_launcher(root, cx).update(cx, |launcher, cx| {
                launcher.open_step(next, window, cx);
                launcher.choose_project(project, cx);
            });
            assert!(root.tabs.tabs().is_empty());
            assert!(root
                .core(cx)
                .node_list()
                .unwrap()
                .iter()
                .all(|node| node.node_type != runner_core::protocol::node::NodeType::Tab));
        });
        assert!(shown(&mut harness, "LAUNCHER_BRAND"));
        assert!(!shown(&mut harness, "SIDEBAR_ROW sidebar-launcher-tab"));
    }
    harness.act(|root, window, cx| {
        active_launcher(root, cx).update(cx, |launcher, cx| launcher.change_directory(window, cx))
    });
    let folder = PathBuf::from(&projects[0].cwd).join(".worktrees/feature");
    harness
        .visual
        .simulate_path_prompt_response(|_| Some(vec![folder]));
    harness.visual.run_until_parked();
    harness.act(|root, _, cx| {
        assert_eq!(
            active_launcher(root, cx).read(cx).scope(),
            ProjectScope::Project(projects[0].id.clone())
        );
        assert!(root.tabs.tabs().is_empty());
    });
}

#[test]
fn sidebar_selection_leaves_the_page_without_saving_an_empty_tab() {
    let mut harness = harness();
    let mut tab = String::new();
    let mut sidebar = None;
    harness.act(|root, window, cx| {
            let core = root.app_store.read(cx).test_core.clone();
            let conn = core.db.get().unwrap();
            conn.execute(
                "INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES ('previous-chat', 'stopped', 'codex', 'codex')",
                [],
            )
            .unwrap();
            let layout = PaneLayout::single(Some("previous-chat"), &["previous-chat".into()]);
            tab = runner_daemon::repo::node::create_tab(
                &conn, None, "Previous chat", 0, &layout.serialize().unwrap(),
            )
            .unwrap()
            .id;
            drop(conn);
            root.app_store.update(cx, |store, cx| {
                store.refresh_sessions(cx);
                store.refresh_nodes(cx).unwrap();
            });
            root.reload_tabs(cx).unwrap();
            root.open_new_tab_modal(&NewTab, window, cx);
            sidebar = Some(root.front_page_sidebar());
        });
    harness.visual.update(|window, cx| {
        sidebar
            .unwrap()
            .update(cx, |sidebar, cx| sidebar.select_shortcut_row(1, window, cx));
    });
    harness.visual.run_until_parked();
    harness.act(|root, _, cx| {
        assert_eq!(root.route, AppRoute::Chat);
        assert_eq!(root.tabs.active_tab_id(), Some(tab.as_str()));
        assert_eq!(root.tabs.tabs().len(), 1);
        assert!(!root.launcher_page_visible());
        assert_eq!(
            root.core(cx)
                .node_list()
                .unwrap()
                .iter()
                .filter(|node| node.node_type == runner_core::protocol::node::NodeType::Tab)
                .count(),
            1
        );
    });
}

#[test]
fn successful_chat_terminal_and_mission_starts_reveal_their_collapsed_destination() {
    for kind in ["chat", "terminal", "mission"] {
        for in_project in [false, true] {
            let mut harness = harness();
            let projects = project_fixture(&mut harness);
            let scope = ProjectScope::or_root(in_project.then(|| projects[0].id.clone()));
            harness.act(|root, window, cx| {
                root.app_store.update(cx, |store, _| {
                    store.settings.sidebar_projects_open = false;
                    store.settings.sidebar_chats_open = false;
                    store
                        .settings
                        .sidebar_collapsed_projects
                        .insert(projects[0].id.clone());
                });
                root.open_launcher_page(scope.clone(), Step::Start, false, window, cx);
            });
            harness.act(|root, _, cx| {
                assert!(!root.settings(cx).sidebar_projects_open);
                assert!(!root.settings(cx).sidebar_chats_open);
                assert!(root.tabs.tabs().is_empty());
            });
            harness.act(|root, window, cx| match kind {
                    "chat" => {
                        let parent = scope.project_id().map(|id| {
                            root.core(cx).node_list().unwrap().into_iter()
                                .find(|node| node.ref_id.as_deref() == Some(id))
                                .unwrap().id
                        });
                        let core = root.app_store.read(cx).test_core.clone();
                        let conn = core.db.get().unwrap();
                        conn.execute(
                            "INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES ('started-chat', 'stopped', 'codex', 'codex')",
                            [],
                        ).unwrap();
                        let layout = PaneLayout::single(Some("started-chat"), &["started-chat".into()]);
                        runner_daemon::repo::node::create_tab(
                            &conn, parent.as_deref(), "Started chat", 0, &layout.serialize().unwrap(),
                        ).unwrap();
                        drop(conn);
                        root.finish_local_chat_launch(
                            Some("started-chat".into()),
                            Ok(crate::surfaces::front_page::client::ChatLaunched {
                                session: "started-chat".into(), layout: None,
                            }),
                            window, cx,
                        );
                    }
                    "terminal" => active_launcher(root, cx).update(cx, |launcher, cx| {
                        launcher.start(Choice::Terminal, false, window, cx)
                    }),
                    "mission" => active_launcher(root, cx).update(cx, |launcher, cx| {
                        launcher.open_step(Step::Goal(None), window, cx);
                        launcher.start_mission(window, cx);
                    }),
                    _ => unreachable!(),
                });
            let mut selector = String::new();
            harness.act(|root, _, cx| {
                let settings = root.settings(cx);
                if in_project {
                    assert!(settings.sidebar_projects_open, "{kind}");
                    assert!(
                        !settings
                            .sidebar_collapsed_projects
                            .contains(&projects[0].id),
                        "{kind}"
                    );
                    assert!(!settings.sidebar_chats_open);
                } else {
                    assert!(settings.sidebar_chats_open, "{kind}");
                    assert!(!settings.sidebar_projects_open);
                }
                let node = root
                    .core(cx)
                    .node_list()
                    .unwrap()
                    .into_iter()
                    .find(|node| node.node_type != runner_core::protocol::node::NodeType::Project)
                    .unwrap();
                selector = if kind == "mission" {
                    format!("SIDEBAR_ROW sidebar-mission-{}", node.ref_id.unwrap())
                } else {
                    format!("SIDEBAR_ROW sidebar-tab-{}", node.id)
                };
            });
            assert!(
                shown(&mut harness, Box::leak(selector.into_boxed_str())),
                "{kind}, project={in_project}"
            );
            if kind == "terminal" {
                harness.act(|root, _, cx| {
                    let session = root
                        .tabs
                        .active()
                        .unwrap()
                        .focused_session_id()
                        .unwrap()
                        .to_owned();
                    root.core(cx).session_close(&session).unwrap();
                });
            }
        }
    }
}

struct RefusedMissionStart(
    runner_daemon::daemon::InProcessTransport,
    Option<runner_core::protocol::RuntimeVersionGate>,
);

impl runner_core::protocol::Transport for RefusedMissionStart {
    fn call(
        &self,
        request: runner_core::protocol::Request,
    ) -> runner_core::protocol::ClientResult<runner_core::protocol::Response> {
        if matches!(
            request,
            runner_core::protocol::Request::mission_start_impl_with_size { .. }
        ) {
            return Ok(
                runner_core::protocol::Response::mission_start_impl_with_size(Err(
                    runner_core::protocol::ClientError {
                        message: "bounded start failure".into(),
                        version_gate: self.1.clone(),
                    },
                )),
            );
        }
        self.0.call(request)
    }

    fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
        self.0.subscribe()
    }
}

#[test]
fn a_refused_mission_keeps_its_request_for_explicit_update_recovery() {
    let mut harness = harness();
    harness.act(|root, window, cx| {
        root.app_store.update(cx, |store, _| {
            store.client =
                runner_core::protocol::DaemonClient::new(std::sync::Arc::new(RefusedMissionStart(
                    runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
                    Some(runner_core::protocol::RuntimeVersionGate {
                        runtime: Runtime::Codex,
                        installed_version: "0.147.0".into(),
                        min_version: "0.149.0".into(),
                    }),
                )));
        });
        root.reset_launcher_page(cx);
        root.open_launcher_page(ProjectScope::Root, Step::Start, false, window, cx);
        active_launcher(root, cx).update(cx, |launcher, cx| {
            launcher.open_step(Step::Goal(None), window, cx);
            launcher.start_mission(window, cx);
        });
    });
    harness.act(|root, _, cx| {
        let launcher = active_launcher(root, cx);
        let retry = launcher.read(cx).update_retry.as_ref().unwrap();
        assert!(!retry.requested);
        assert!(matches!(&retry.action, UpdateAction::Mission(request) if request.crew.is_empty()));
        assert_eq!(retry.gate.as_ref().unwrap().installed_version, "0.147.0");
        assert!(root.agent_update.is_none());
        assert!(root.core(cx).mission_list(None).unwrap().is_empty());
    });
}

#[test]
fn failed_and_canceled_starts_keep_the_destination_collapsed() {
    for kind in ["chat", "terminal", "mission", "cancel"] {
        for in_project in [false, true] {
            let mut harness = harness();
            let projects = project_fixture(&mut harness);
            let scope = ProjectScope::or_root(in_project.then(|| projects[0].id.clone()));
            let invalid = PathBuf::from(&projects[0].cwd).join("not-a-directory");
            std::fs::write(&invalid, "fixture").unwrap();
            harness.act(|root, window, cx| {
                root.app_store.update(cx, |store, _| {
                    store.settings.sidebar_projects_open = false;
                    store.settings.sidebar_chats_open = false;
                    store
                        .settings
                        .sidebar_collapsed_projects
                        .insert(projects[0].id.clone());
                });
                if kind == "mission" {
                    root.app_store.update(cx, |store, _| {
                        store.client = runner_core::protocol::DaemonClient::new(
                            std::sync::Arc::new(RefusedMissionStart(
                                runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
                                None,
                            )),
                        );
                    });
                }
                root.open_crews(window, cx);
                root.open_launcher_page(scope.clone(), Step::Start, false, window, cx);
                if kind == "chat" {
                    root.finish_local_chat_launch(
                        None,
                        Err(runner_core::protocol::ClientError::msg(
                            "bounded start failure",
                        )),
                        window,
                        cx,
                    );
                }
                active_launcher(root, cx).update(cx, |launcher, cx| {
                    launcher.cwd.update(cx, |cwd, cx| {
                        cwd.set_text(invalid.to_string_lossy().into_owned(), cx)
                    });
                    if kind == "terminal" {
                        launcher.start(Choice::Terminal, false, window, cx);
                    } else if kind == "mission" {
                        launcher.open_step(Step::Goal(None), window, cx);
                        launcher.start_mission(window, cx);
                    }
                });
            });
            if kind == "cancel" {
                harness.visual.simulate_keystrokes("escape");
                harness.visual.run_until_parked();
            }
            harness.act(|root, _, cx| {
                assert!(!root.settings(cx).sidebar_projects_open, "{kind}");
                assert!(!root.settings(cx).sidebar_chats_open, "{kind}");
                assert!(root
                    .settings(cx)
                    .sidebar_collapsed_projects
                    .contains(&projects[0].id));
                assert!(root.tabs.tabs().is_empty());
                assert!(root
                    .core(cx)
                    .node_list()
                    .unwrap()
                    .iter()
                    .all(|node| node.node_type == runner_core::protocol::node::NodeType::Project));
                if kind == "cancel" {
                    assert_eq!(root.route, AppRoute::Crews);
                } else if kind == "chat" {
                    assert!(root.chat_error.is_some());
                } else {
                    assert!(active_launcher(root, cx).read(cx).error.is_some());
                }
            });
        }
    }
}

#[test]
fn crew_step_lists_crews_with_roles_and_a_roleless_crew_dimmed_that_opens_its_page() {
    let mut harness = harness();
    let mut ids = (String::new(), String::new());
    harness.act(|root, _, cx| {
        ids = (
            crew(root, "Alpha crew", &["impl", "review"], cx),
            crew(root, "Empty crew", &[], cx),
        )
    });
    let (alpha, empty) = ids;
    harness.act(|root, window, cx| root.open_crews(window, cx));
    harness.act(|root, window, cx| root.new_mission_action(&NewMission, window, cx));
    harness.act(|root, window, cx| {
        let launcher = active_launcher(root, cx);
        let rows = launcher.read(cx).rows(cx).0;
        let row = |id: &str| {
            rows.iter()
                .find(|row| row.choice == Choice::Crew(id.into()))
                .unwrap()
        };
        assert!(!row(&alpha).dimmed);
        assert_eq!(row(&alpha).detail, "impl, review");
        assert!(row(&empty).dimmed);
        assert_eq!(row(&empty).detail, "No roles yet");
        assert_eq!(rows.last().unwrap().choice, Choice::EmptyMission);
        launcher.update(cx, |launcher, cx| {
            launcher.start(Choice::Crew(empty.clone()), false, window, cx)
        });
    });
    harness.act(|root, _, _| {
        assert_eq!(root.route, AppRoute::CrewEditor(empty.clone()));
        assert!(
            root.tabs.tabs().is_empty(),
            "the launcher page never saved a tab"
        );
    });
}

#[test]
fn an_empty_mission_starts_from_the_launcher_page_in_the_chosen_project() {
    let mut harness = harness();
    let projects = project_fixture(&mut harness);
    let project = projects[0].clone();
    let mut sidebar = None;
    harness.act(|root, _, _| sidebar = Some(root.front_page_sidebar()));
    let scope = ProjectScope::Project(project.id.clone());
    harness.visual.update(|window, cx| {
        sidebar.clone().unwrap().update(cx, |sidebar, cx| {
            sidebar.open_launcher_in(scope, window, cx)
        })
    });
    harness.visual.run_until_parked();
    harness.act(|root, _, cx| {
        assert!(root.tabs.tabs().is_empty());
        assert_eq!(root.active_project_id(cx), Some(project.id.clone()));
        assert_eq!(step(root, cx), Step::Start);
    });
    harness.act(|root, window, cx| root.new_mission_action(&NewMission, window, cx));
    harness.visual.simulate_keystrokes("shift-enter");
    harness.visual.run_until_parked();
    harness.act(|root, _, cx| assert_eq!(step(root, cx), Step::Goal(None)));
    assert!(shown(&mut harness, "LAUNCHER-CREW-CHIP"));
    assert!(shown(&mut harness, "LAUNCHER_BRAND"));
    assert!(!shown(&mut harness, "LAUNCHER_SLOTS"));
    harness.visual.simulate_keystrokes("enter");
    harness.visual.run_until_parked();
    harness.act(|root, _, cx| {
        let AppRoute::Mission(id) = &root.route else {
            panic!("the mission opened");
        };
        let mission = root.core(cx).mission_get(id).unwrap();
        assert!(mission.crew_id.is_none());
        assert_eq!(
            mission.title,
            format!("Mission {}", chrono::Local::now().format("%Y-%m-%d"))
        );
        assert_eq!(mission.project_id.as_deref(), Some(project.id.as_str()));
        assert_eq!(mission.cwd.as_deref(), Some(project.cwd.as_str()));
        assert!(root.core(cx).session_list(id).unwrap().is_empty());
        assert!(
            root.tabs.tabs().is_empty(),
            "no empty launcher tab was saved"
        );
        assert!(
            root.launchers
                .values()
                .all(|launcher| launcher.read(cx).pane.is_none()),
            "no launcher is left on the tab"
        );
    });
    assert!(!shown(&mut harness, "SIDEBAR_ROW sidebar-launcher-tab"));
}

#[test]
fn typing_finds_crews_after_agents_and_the_goal_step_shows_slots_and_steps_back() {
    let mut harness = harness();
    let mut alpha = String::new();
    harness.act(|root, _, cx| alpha = crew(root, "Alpha crew", &["impl", "review"], cx));
    harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
    harness.visual.simulate_input("alpha");
    harness.visual.run_until_parked();
    harness.act(|root, window, cx| {
        let launcher = active_launcher(root, cx);
        let rows = launcher.read(cx).rows(cx).0;
        let crew = rows
            .iter()
            .position(|row| row.choice == Choice::Crew(alpha.clone()))
            .expect("a matching crew");
        assert_eq!(rows[crew].detail, "Crew · impl, review");
        assert!(rows[crew + 1..].iter().all(|row| row.choice.pinned()));
        launcher.update(cx, |launcher, cx| {
            launcher.start(Choice::Crew(alpha.clone()), false, window, cx)
        });
    });
    harness.act(|root, _, cx| assert_eq!(step(root, cx), Step::Goal(Some(alpha.clone()))));
    for selector in [
        "LAUNCHER_GOAL",
        "LAUNCHER_SLOT impl",
        "LAUNCHER_SLOT review",
        "LAUNCHER_BRAND",
    ] {
        assert!(shown(&mut harness, selector), "{selector}");
    }
    harness.visual.simulate_input("First");
    harness.visual.simulate_keystrokes("shift-enter");
    harness.visual.simulate_input("Second");
    harness.visual.run_until_parked();
    harness.act(|root, _, cx| {
        let launcher = active_launcher(root, cx);
        assert_eq!(launcher.read(cx).goal.read(cx).text(), "First\nSecond");
        assert!(launcher.read(cx).pending.is_none());
    });
    harness.visual.simulate_keystrokes("escape");
    harness.visual.run_until_parked();
    harness.act(|root, _, cx| {
        assert_eq!(step(root, cx), Step::Crews, "Esc goes back to the crews");
        assert_eq!(
            active_launcher(root, cx).read(cx).goal.read(cx).text(),
            "First\nSecond",
            "the goal waits for the next crew"
        );
    });
    let chip = harness
        .visual
        .debug_bounds("LAUNCHER-MISSION-CHIP")
        .unwrap();
    harness.visual.simulate_click(
        chip.top_right() + gpui::point(px(-6.), px(10.)),
        Default::default(),
    );
    harness.visual.run_until_parked();
    harness.act(|root, _, cx| {
        assert_eq!(step(root, cx), Step::Start, "the chip's × goes home");
        assert!(root.tabs.tabs().is_empty());
    });
}
