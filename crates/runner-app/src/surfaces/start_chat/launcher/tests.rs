use super::*;
use crate::surfaces::start_chat::tests::{modal_harness, project_fixture, ModalHarness};
use runner_core::protocol::model::Runtime;

fn agent() -> RuntimeChoice {
    RuntimeChoice {
        installed_version: None,
        available_version: None,
        min_version: None,
        below_minimum: false,
        name: Runtime::Codex,
        display_name: "Codex".into(),
        command: Some("codex".into()),
        available: true,
        default_model: Some("illustrative-default".into()),
        default_effort: Some("high".into()),
        models: Vec::new(),
        efforts: Vec::new(),
        speed: true,
        effort_needs_launch_model: false,
    }
}
fn role() -> RoleChoice {
    RoleChoice {
        id: "role".into(),
        handle: "coder".into(),
        display_name: "Bounded role".into(),
        runtime: "codex".into(),
        model: Some("model".into()),
        effort: Some("low".into()),
        codex_speed: None,
        working_dir: Some("/role-directory".into()),
    }
}

struct UpdateFixture {
    inner: runner_daemon::daemon::InProcessTransport,
    starts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl runner_core::protocol::Transport for UpdateFixture {
    fn call(
        &self,
        request: runner_core::protocol::Request,
    ) -> runner_core::protocol::ClientResult<runner_core::protocol::Response> {
        if matches!(
            request,
            runner_core::protocol::Request::runtime_task_start { .. }
        ) {
            self.starts
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(runner_core::protocol::Response::runtime_task_start(Err(
                runner_core::protocol::ClientError::msg("scratch updater is offline"),
            )));
        }
        if let runner_core::protocol::Request::session_start_runtime_with_speed { cwd, .. } =
            &request
        {
            let conn = self.inner.0.db.get().unwrap();
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sessions WHERE id LIKE 'updated-chat%'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let id = if count == 0 {
                "updated-chat".to_owned()
            } else {
                format!("updated-chat-{count}")
            };
            conn.execute("INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES (?1, 'stopped', 'codex', 'codex')", [&id]).unwrap();
            return Ok(
                runner_core::protocol::Response::session_start_runtime_with_speed(Ok(
                    runner_core::protocol::session::StartDirectSessionOutput {
                        session: runner_core::protocol::session::SpawnedSession {
                            id,
                            mission_id: None,
                            role_id: None,
                            handle: "codex".into(),
                            pid: None,
                        },
                        project_id: None,
                        cwd: cwd.clone(),
                    },
                )),
            );
        }
        self.inner.call(request)
    }
    fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
        self.inner.subscribe()
    }
}

#[test]
fn required_updates_mark_only_agents_and_defer_agent_and_role_launch_intents() {
    for key in [
        EntryKey::Agent("codex".into()),
        EntryKey::Role("role".into()),
    ] {
        let mut catalog = RuntimeCatalogEntry::for_runtime(Runtime::Codex).unwrap();
        catalog.available = true;
        let mut harness = modal_harness(1200., 900., Vec::new(), vec![catalog], ChatMode::Runtime);
        let starts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut page = None;
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.app_store.update(cx, |store, _| {
                store.client =
                    runner_core::protocol::DaemonClient::new(std::sync::Arc::new(UpdateFixture {
                        inner: runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
                        starts: starts.clone(),
                    }));
            });
            root.open_launcher_page(ProjectScope::Root, Step::Start, false, window, cx);
            let launcher = root.render_launcher(None, true, window, cx);
            launcher.update(cx, |view, cx| {
                let mut runtime = agent();
                runtime.installed_version = Some("0.147.0".into());
                runtime.min_version = Some("0.149.0".into());
                runtime.available_version = Some("0.167.0".into());
                runtime.below_minimum = true;
                view.runtimes = vec![runtime];
                view.roles = vec![role()];
                cx.notify();
            });
            page = Some(launcher);
        });
        assert!(harness.bounds("LAUNCHER_UPDATE_NEEDED:Codex").is_some());
        assert!(harness
            .bounds("LAUNCHER_UPDATE_NEEDED:Bounded role")
            .is_none());
        assert!(harness.bounds("LAUNCHER_VERSION_NOTICE").is_some());
        let launcher = page.unwrap();
        harness.act(|root, window, cx| {
                launcher.update(cx, |view, _| view.pending = Some(Choice::Chat(key.clone())));
                root.launch_from_launcher(launcher.clone(), Some(key.clone()), None, ProjectScope::Root, None, false, window, cx);
                assert!(!root.launcher_start_pending(cx));
                assert!(launcher.read(cx).pane.is_some());
                assert!(root.agent_update.is_none());
                let retry = launcher.read(cx).update_retry.as_ref().unwrap();
                assert_eq!(retry.runtime, Runtime::Codex);
                assert!(retry.requested);
                assert!(matches!(&retry.action, UpdateAction::Chat { key: Some(saved), .. } if saved == &key));
                assert_eq!(retry.gate.as_ref().unwrap().min_version, "0.149.0");
                assert!(root.core(cx).session_list_recent_direct().unwrap().is_empty());
            });
        assert_eq!(starts.load(std::sync::atomic::Ordering::Relaxed), 1);
    }
}

#[test]
fn a_gated_pane_launches_after_success_without_replacing_new_work_or_focus() {
    use runner_core::protocol::{RuntimeUpdateState, RuntimeUpdateTask};
    let mut harness = modal_harness(1200., 900., Vec::new(), Vec::new(), ChatMode::Runtime);
    let mut waiting = String::new();
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        root.app_store.update(cx, |store, _| {
            store.client =
                runner_core::protocol::DaemonClient::new(std::sync::Arc::new(UpdateFixture {
                    inner: runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
                    starts: Default::default(),
                }));
        });
        root.reset_launcher_page(cx);
        let launcher = root.render_launcher(None, true, window, cx);
        launcher.update(cx, |view, _| {
            let mut runtime = agent();
            runtime.below_minimum = true;
            view.runtimes = vec![runtime];
            view.pending = Some(Choice::Chat(EntryKey::Agent("codex".into())));
        });
        root.launch_from_launcher(
            launcher.clone(),
            Some(EntryKey::Agent("codex".into())),
            None,
            ProjectScope::Root,
            None,
            false,
            window,
            cx,
        );
        waiting = launcher.read(cx).pane.as_ref().unwrap().tab_id.clone();
        assert!(!root.launcher_start_pending(cx));
        root.open_new_tab_modal(&NewTab, window, cx);
        assert!(launcher.read(cx).update_retry.is_some());
    });
    let mut focus = None;
    harness.act(|root, window, cx| {
        focus = window.focused(cx);
        assert!(root.launcher_page_visible());
        root.apply_agent_tasks(
            vec![RuntimeUpdateTask {
                id: "successful-update".into(),
                runtime: Runtime::Codex,
                state: RuntimeUpdateState::Succeeded,
                from_version: Some("0.147.0".into()),
                to_version: Some("0.167.0".into()),
                installed_version: Some("0.167.0".into()),
                step: "Done".into(),
                message: None,
                exit_code: Some(0),
                running_sessions: 0,
            }],
            window,
            cx,
        );
    });
    harness.act(|root, window, cx| {
        assert!(root.launcher_page_visible());
        assert_eq!(window.focused(cx), focus);
        let tab = root
            .tabs
            .tabs()
            .iter()
            .find(|tab| tab.id == waiting)
            .unwrap();
        assert!(tab.contains_session("updated-chat"));
        assert!(root.agent_update.is_none());
    });
}

#[test]
fn shared_update_queues_both_panes_behind_the_tabs_active_start() {
    use runner_core::protocol::{RuntimeUpdateState, RuntimeUpdateTask};
    let mut harness = modal_harness(1200., 900., Vec::new(), Vec::new(), ChatMode::Runtime);
    let mut waiting_tab = String::new();
    let mut guard = None;
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        root.app_store.update(cx, |store, _| {
            store.client =
                runner_core::protocol::DaemonClient::new(std::sync::Arc::new(UpdateFixture {
                    inner: runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
                    starts: Default::default(),
                }))
        });
        root.reset_launcher_page(cx);
        root.test_open_empty_tab(window, cx);
        let mut layout = root.tabs.active().unwrap().clone();
        layout.split("p1", SplitOrientation::Row).unwrap();
        root.front_page_client(cx)
            .launcher_layout(&layout, None)
            .unwrap();
        root.reload_tabs(cx).unwrap();
        root.tabs.activate(&layout.id);
        waiting_tab = layout.id.clone();
        for leaf in layout.root.leaves() {
            let pane = PaneKey::new(&layout.id, &leaf.id);
            let launcher = root.render_launcher(Some(pane.clone()), false, window, cx);
            launcher.update(cx, |view, _| {
                let mut runtime = agent();
                runtime.below_minimum = true;
                view.runtimes = vec![runtime];
            });
            root.launch_from_launcher(
                launcher,
                Some(EntryKey::Agent("codex".into())),
                Some(pane),
                ProjectScope::Root,
                None,
                false,
                window,
                cx,
            );
        }
        guard = TabStart::begin(
            root.app_store.read(cx).launcher_tabs_pending.clone(),
            format!("local:{}", layout.id),
            layout,
        );
        root.apply_agent_tasks(
            vec![RuntimeUpdateTask {
                id: "one-update".into(),
                runtime: Runtime::Codex,
                state: RuntimeUpdateState::Succeeded,
                from_version: None,
                to_version: None,
                installed_version: Some("0.167.0".into()),
                step: "Done".into(),
                message: None,
                exit_code: Some(0),
                running_sessions: 0,
            }],
            window,
            cx,
        );
        assert!(root
            .core(cx)
            .session_list_recent_direct()
            .unwrap()
            .is_empty());
        assert_eq!(
            root.launchers
                .values()
                .filter(|view| view
                    .read(cx)
                    .update_retry
                    .as_ref()
                    .is_some_and(|retry| retry.ready))
                .count(),
            2
        );
    });
    drop(guard);
    harness.act(|root, window, cx| root.drain_ready_launches(window, cx));
    harness.act(|root, _, cx| {
        let tab = root
            .tabs
            .tabs()
            .iter()
            .find(|tab| tab.id == waiting_tab)
            .unwrap();
        assert_eq!(tab.session_ids().len(), 2);
        assert!(tab.contains_session("updated-chat"));
        assert!(tab.contains_session("updated-chat-1"));
        assert_eq!(root.core(cx).session_list_recent_direct().unwrap().len(), 2);
    });
}

struct DeferredOrigin {
    base: std::sync::Arc<crate::surfaces::remote_client::tests::Fake>,
    core: runner_daemon::AppCore,
    calls: std::sync::atomic::AtomicUsize,
}
impl runner_core::protocol::Transport for DeferredOrigin {
    fn call(
        &self,
        request: runner_core::protocol::Request,
    ) -> runner_core::protocol::ClientResult<runner_core::protocol::Response> {
        use runner_core::protocol::{Request, Response};
        use std::sync::atomic::Ordering;
        if matches!(request, Request::runtime_task_start { .. }) {
            let mut task: runner_core::protocol::RuntimeUpdateTask =
                serde_json::from_value(crate::surfaces::remote_client::tests::completed_update())
                    .unwrap();
            task.state = runner_core::protocol::RuntimeUpdateState::Running;
            self.base.emit(
                "runtime/task-changed",
                serde_json::to_value(vec![task.clone()]).unwrap(),
            );
            return Ok(Response::runtime_task_start(Ok(task)));
        }
        if matches!(
            request,
            Request::session_fork { .. } | Request::mission_start_impl_with_size { .. }
        ) {
            let attempt = self.calls.fetch_add(1, Ordering::Relaxed);
            if attempt == 0 {
                let error = crate::surfaces::remote_client::tests::update_gate_error();
                return Ok(match request {
                    Request::session_fork { .. } => Response::session_fork(Err(error)),
                    _ => Response::mission_start_impl_with_size(Err(error)),
                });
            }
            if matches!(request, Request::session_fork { .. }) {
                let conn = self.core.db.get().unwrap();
                conn.execute("INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES ('forked-origin', 'stopped', 'codex', 'codex')", []).unwrap();
                let layout = PaneLayout::single(Some("forked-origin"), &["forked-origin".into()]);
                runner_daemon::repo::node::create_tab(
                    &conn,
                    None,
                    "Fork result",
                    1,
                    &layout.serialize().unwrap(),
                )
                .unwrap();
                return Ok(Response::session_fork(Ok(
                    runner_core::protocol::session::SpawnedSession {
                        id: "forked-origin".into(),
                        mission_id: None,
                        role_id: None,
                        handle: "codex".into(),
                        pid: None,
                    },
                )));
            }
        }
        self.base.call(request)
    }
    fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
        self.base.subscribe()
    }
}

#[test]
fn deferred_mission_and_fork_keep_the_saved_host_and_later_work_focused() {
    use crate::surfaces::remote_client::tests as remote_test;
    use std::sync::atomic::Ordering;
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for kind in ["mission", "fork"] {
        for switch_host in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = gpui::TestAppContext::single();
            let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
            remote_test::enable_front_page(&fake);
            let core = store.read_with(&cx, |store, _| store.test_core.clone());
            let trace = std::sync::Arc::new(DeferredOrigin {
                base: fake.clone(),
                core: core.clone(),
                calls: Default::default(),
            });
            store.update(&mut cx, |store, _| {
                store.client = runner_core::protocol::DaemonClient::new(trace.clone())
            });
            let window = remote_test::add_window(&mut cx, &store, temp.path(), kind);
            window.update(&mut cx, |root, window, cx| {
                    if kind == "mission" {
                        root.open_launcher_page(ProjectScope::Root, Step::Goal(None), false, window, cx);
                        let launcher = root.render_launcher(None, true, window, cx);
                        launcher.update(cx, |view, cx| view.start_mission(window, cx));
                    } else {
                        core.db.get().unwrap().execute("INSERT INTO sessions(id, status, agent_runtime, agent_command, agent_session_key) VALUES ('origin', 'stopped', 'codex', 'codex', 'key')", []).unwrap();
                        let mut layout = PaneLayout::single(Some("origin"), &["origin".into()]);
                        layout.id = runner_core::event_log::UlidGen::new().next().unwrap();
                        root.front_page_client(cx).launcher_layout(&layout, None).unwrap();
                        root.refresh_sessions(cx);
                        root.reload_tabs(cx).unwrap();
                        root.tabs.activate_session("origin");
                        root.set_route(AppRoute::Chat, cx);
                        root.fork_chat("origin", window, cx);
                        root.confirm_fork_chat(window, cx);
                    }
                }).unwrap();
            cx.run_until_parked();
            assert_eq!(
                trace.calls.load(Ordering::Relaxed),
                1,
                "{kind}: original request reaches the local gate"
            );
            let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
            visual.run_until_parked();
            let update = visual
                .debug_bounds("LAUNCHER_UPDATE_ACTION")
                .expect("refused origin has recovery action");
            visual.simulate_click(update.center(), Default::default());
            visual.run_until_parked();
            window
                .update(&mut visual, |root, window, cx| {
                    if switch_host {
                        remote_test::choose_host(root, window, cx);
                    } else {
                        root.open_new_tab_modal(&NewTab, window, cx);
                    }
                })
                .unwrap();
            visual.run_until_parked();
            if !switch_host {
                let (visible, waiting, old_query, old_goal) = window
                    .read_with(&visual, |root, cx| {
                        let visible = root.page_launcher(cx).unwrap().clone();
                        let waiting = root
                            .launchers
                            .iter()
                            .find(|(key, _)| key.starts_with("update-wait:"))
                            .unwrap()
                            .1
                            .clone();
                        let old_query = waiting.read(cx).query.read(cx).text().to_owned();
                        let old_goal = waiting.read(cx).goal.read(cx).text().to_owned();
                        (visible, waiting, old_query, old_goal)
                    })
                    .unwrap();
                window
                    .update(&mut visual, |root, window, cx| {
                        root.open_this_computer_settings(Some("agents"), window, cx)
                    })
                    .unwrap();
                visual.run_until_parked();
                window
                    .update(&mut visual, |root, window, cx| {
                        root.leave_settings(window, cx)
                    })
                    .unwrap();
                visual.run_until_parked();
                window
                    .update(&mut visual, |root, window, cx| {
                        assert_eq!(root.page_launcher(cx), Some(&visible));
                        assert!(visible
                            .read(cx)
                            .query
                            .read(cx)
                            .focus_handle()
                            .is_focused(window));
                    })
                    .unwrap();
                visual.simulate_keystrokes("v i s i b l e");
                visual.run_until_parked();
                window
                    .read_with(&visual, |_, cx| {
                        assert_eq!(visible.read(cx).query.read(cx).text(), "visible");
                        assert_eq!(waiting.read(cx).query.read(cx).text(), old_query);
                        assert_eq!(waiting.read(cx).goal.read(cx).text(), old_goal);
                    })
                    .unwrap();
            }
            let mut before = None;
            window
                .update(&mut visual, |root, window, cx| {
                    before = Some((
                        root.front_page_client(cx).target,
                        root.route.clone(),
                        window.focused(cx),
                    ));
                    let task = serde_json::from_value(remote_test::completed_update()).unwrap();
                    root.apply_agent_tasks(vec![task], window, cx);
                })
                .unwrap();
            visual.run_until_parked();
            assert_eq!(
                trace.calls.load(Ordering::Relaxed),
                2,
                "{kind} retried exactly once"
            );
            assert_eq!(
                fake.request_count(if kind == "mission" {
                    "startMission"
                } else {
                    "fork"
                }),
                0,
                "a local request must never be sent to the selected host"
            );
            window
                .read_with(&visual, |root, cx| {
                    let (target, route, _) = before.as_ref().unwrap();
                    assert_eq!(&root.front_page_client(cx).target, target);
                    assert_eq!(&root.route, route);
                })
                .unwrap();
            window
                .update(&mut visual, |_, window, cx| {
                    assert_eq!(window.focused(cx), before.as_ref().unwrap().2)
                })
                .unwrap();
            if kind == "mission" {
                assert_eq!(
                    runner_daemon::ops::mission::mission_list(&core, None)
                        .unwrap()
                        .len(),
                    1
                );
            } else {
                assert!(runner_daemon::repo::session::get_row(
                    &core.db.get().unwrap(),
                    "forked-origin"
                )
                .unwrap()
                .is_some());
            }
        }
    }
}

#[test]
fn optional_updates_only_tag_the_agent_row_and_keep_the_launcher_available() {
    let mut catalog = RuntimeCatalogEntry::for_runtime(Runtime::Codex).unwrap();
    catalog.available = true;
    let mut harness = modal_harness(1200., 900., Vec::new(), vec![catalog], ChatMode::Runtime);
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        root.open_launcher_page(ProjectScope::Root, Step::Start, false, window, cx);
        let launcher = root.render_launcher(None, true, window, cx);
        launcher.update(cx, |view, cx| {
            let mut runtime = agent();
            runtime.installed_version = Some("0.166.0".into());
            runtime.available_version = Some("0.167.0".into());
            view.runtimes = vec![runtime];
            view.roles = vec![role()];
            assert!(view.update_retry.is_none());
            let (rows, _) = view.rows(cx);
            assert!(rows
                .iter()
                .filter(|row| matches!(row.choice, Choice::Chat(_)))
                .all(|row| !row.unavailable));
            cx.notify();
        });
    });
    assert!(harness.bounds("LAUNCHER_OPTIONAL_UPDATE:codex").is_some());
    assert!(harness
        .bounds("LAUNCHER_UPDATE_NEEDED:Bounded role")
        .is_none());
    assert!(harness.bounds("LAUNCHER_VERSION_NOTICE").is_none());
    harness.read_root(|root, _| assert!(root.agent_update.is_none()));
}

#[test]
fn requests_have_no_form_overrides_and_directory_change_wins_over_role_directory() {
    for key in [
        EntryKey::Agent("codex".into()),
        EntryKey::Role("role".into()),
    ] {
        let request = launcher_input(
            &key,
            Some("project".into()),
            Some("/shown".into()),
            (90, 30),
        );
        assert_eq!(request.project.as_deref(), Some("project"));
        assert_eq!(request.cwd.as_deref(), Some("/shown"));
        assert!(request.model.is_none() && request.effort.is_none() && request.speed.is_none());
        assert_eq!(request.runtime.is_some(), matches!(key, EntryKey::Agent(_)));
        assert_eq!(request.role.is_some(), matches!(key, EntryKey::Role(_)));
    }
    let mut role = role();
    assert_eq!(
        launcher_directory(Some(&role), Some("/shown".into()), false).as_deref(),
        Some("/role-directory")
    );
    assert_eq!(
        launcher_directory(Some(&role), Some("/changed".into()), true).as_deref(),
        Some("/changed")
    );
    role.working_dir = None;
    assert_eq!(
        launcher_directory(Some(&role), Some("/shown".into()), false).as_deref(),
        Some("/shown")
    );
}

#[test]
fn recency_identity_ignores_connection_generation_but_separates_computers() {
    let remote = |computer: &str, generation| FrontPageTarget::Remote {
        computer: computer.into(),
        fingerprint: "fingerprint".into(),
        generation,
    };
    assert_eq!(computer_key(&remote("a", 1)), computer_key(&remote("a", 2)));
    assert_ne!(computer_key(&remote("a", 1)), computer_key(&remote("b", 1)));
    assert_ne!(
        computer_key(&FrontPageTarget::Local),
        computer_key(&remote("a", 1))
    );
}

fn row_scope(harness: &ModalHarness) -> Option<ProjectScope> {
    harness.read_root(|root, cx| root.front_page_sidebar().read(cx).launcher_row_scope(cx))
}

fn assert_row_under(harness: &mut ModalHarness, project: &str) {
    assert_eq!(
        row_scope(harness),
        Some(ProjectScope::Project(project.to_owned()))
    );
    let row = harness
        .bounds("SIDEBAR_LAUNCHER_TAB")
        .expect("launcher row");
    let header = harness
        .bounds(&format!("SIDEBAR_PROJECT:{project}"))
        .expect("project header");
    assert!(row.origin.y > header.origin.y);
    assert!(row.origin.y < header.origin.y + px(48.));
}

fn assert_row_in_recents(harness: &mut ModalHarness) {
    assert_eq!(row_scope(harness), Some(ProjectScope::Root));
    let row = harness
        .bounds("SIDEBAR_LAUNCHER_TAB")
        .expect("launcher row");
    let header = harness
        .bounds("SIDEBAR_SECTION:RECENTS")
        .expect("Recents header");
    assert!(row.origin.y > header.origin.y);
}

/// The project the page's picker shows, and whether it is locked there.
fn page_picker(harness: &mut ModalHarness) -> (ProjectScope, bool) {
    let mut picker = None;
    harness.act(|root, window, cx| {
        let launcher = root.render_launcher(None, true, window, cx);
        let launcher = launcher.read(cx);
        let select = launcher.project.select();
        let select = select.read(cx);
        assert_eq!(
            select.value(),
            launcher.scope().project_id().unwrap_or_default()
        );
        picker = Some((launcher.scope(), select.is_disabled()));
    });
    picker.unwrap()
}

fn page_directory(harness: &mut ModalHarness) -> String {
    let mut directory = String::new();
    harness.act(|root, window, cx| {
        let launcher = root.render_launcher(None, true, window, cx);
        directory = launcher.read(cx).cwd.read(cx).text().to_owned();
    });
    directory
}

fn open_tab_in(harness: &mut ModalHarness, project: Option<&str>) -> String {
    let project = project.map(str::to_owned);
    let mut tab = String::new();
    harness.act(|root, _, cx| {
        root.start_chat_modal = None;
        let mut layout = PaneLayout::single(None, &[]);
        layout.id = runner_core::event_log::UlidGen::new().next().unwrap();
        root.front_page_client(cx)
            .launcher_layout(&layout, project.clone())
            .unwrap();
        root.reload_tabs(cx).unwrap();
        root.tabs.activate(&layout.id);
        root.set_route(AppRoute::Chat, cx);
        root.sync_active_project_from_active_tab(cx);
        assert_eq!(root.active_project_id(cx), project);
        tab = layout.id;
    });
    tab
}

fn open_plus(harness: &mut ModalHarness, scope: ProjectScope) {
    harness.act(move |root, window, cx| {
        root.front_page_sidebar().update(cx, |sidebar, cx| {
            sidebar.open_launcher_in(scope, window, cx);
        });
    });
}

/// Saved chats or terminals, each in a tab of its own, the last one viewed.
fn saved_tabs(harness: &mut ModalHarness, sessions: &[(&str, &str)]) -> Vec<String> {
    let mut tabs = Vec::new();
    harness.act(|root, _, cx| {
            root.start_chat_modal = None;
            let core = root.app_store.read(cx).test_core.clone();
            let conn = core.db.get().unwrap();
            for (position, (session, runtime)) in sessions.iter().enumerate() {
                conn.execute(
                    "INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES (?1, 'stopped', ?2, ?2)",
                    [session, runtime],
                )
                .unwrap();
                let layout = PaneLayout::single(Some(session), &[(*session).into()]);
                tabs.push(
                    runner_daemon::repo::node::create_tab(
                        &conn,
                        None,
                        session,
                        position as i64,
                        &layout.serialize().unwrap(),
                    )
                    .unwrap()
                    .id,
                );
            }
            drop(conn);
            root.app_store.update(cx, |store, cx| {
                store.refresh_sessions(cx);
                store.refresh_nodes(cx).unwrap();
            });
            root.reload_tabs(cx).unwrap();
            root.tabs.activate(tabs.last().unwrap());
            root.set_route(AppRoute::Chat, cx);
        });
    tabs
}

/// Another window with `session` on screen.
fn open_in_second_window(harness: &mut ModalHarness, session: &str) {
    let session = session.to_owned();
    harness.act(move |root, _, cx| {
        let core = root.core(cx);
        core.window_register("second").unwrap();
        core.report_subjects(
            "second",
            vec![runner_core::protocol::window::Subject::DirectChat(session)],
            None,
        )
        .unwrap();
    });
}

fn assert_landed_on(harness: &mut ModalHarness, tab: &str, session: &str) {
    let (tab, session) = (tab.to_owned(), session.to_owned());
    harness.act(move |root, window, _| {
        assert_eq!(root.route, AppRoute::Chat);
        assert_eq!(root.tabs.active_tab_id(), Some(tab.as_str()));
        assert!(!root.launcher_page_visible());
        assert!(root.chat_focus.is_focused(window));
        assert_eq!(
            root.runtime_navigation_history.last(),
            Some(&RuntimeLocation::Chat(session))
        );
    });
}

#[test]
fn new_chat_entries_show_a_page_without_saving_a_tab() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let projects = project_fixture(&mut harness);
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        root.open_new_tab_modal(&NewTab, window, cx);
        root.open_new_tab_modal(&NewTab, window, cx);
        assert_eq!(root.route, AppRoute::Launcher);
        assert!(root.tabs.tabs().is_empty());
        assert!(root
            .core(cx)
            .node_list()
            .unwrap()
            .iter()
            .all(|node| node.node_type != runner_core::protocol::node::NodeType::Tab));
        root.open_sidebar_chat_modal(ProjectScope::Project(projects[0].id.clone()), window, cx);
        let launcher = root.render_launcher(None, true, window, cx);
        assert_eq!(
            launcher.read(cx).scope(),
            ProjectScope::Project(projects[0].id.clone())
        );
        assert_eq!(launcher.read(cx).cwd.read(cx).text(), projects[0].cwd);
        assert!(root.tabs.tabs().is_empty());
    });
}

fn update_settings(
    harness: &mut ModalHarness,
    update: impl FnOnce(&mut AppSettings) -> bool + 'static,
) {
    harness.act(move |root, _, cx| {
        root.app_store
            .update(cx, |store, cx| store.update_settings(update, false, cx));
    });
}

#[test]
fn launcher_page_row_stays_where_the_page_opened_and_plus_reveals_it() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let projects = project_fixture(&mut harness);
    let (alpha, beta) = (projects[0].id.clone(), projects[1].id.clone());
    let (alpha_path, beta_path) = (projects[0].cwd.clone(), projects[1].cwd.clone());
    harness.act(|root, _, cx| {
        root.start_chat_modal = None;
        let nodes = root.core(cx).node_list().unwrap();
        root.app_store.update(cx, |store, cx| {
            store.nodes = nodes;
            cx.notify();
        });
    });
    let collapsed = beta.clone();
    update_settings(&mut harness, move |settings| {
        settings.sidebar_collapsed_projects.insert(collapsed)
    });

    // Opening the page under a collapsed project leaves it collapsed, so
    // there is no row and New tab keeps the selection.
    let scope = ProjectScope::Project(beta.clone());
    harness.act(move |root, window, cx| root.open_sidebar_chat_modal(scope, window, cx));
    assert_eq!(row_scope(&harness), None);
    assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB").is_none());

    // + on a collapsed project expands it, so the row is visible.
    open_plus(&mut harness, ProjectScope::Project(beta.clone()));
    assert_row_under(&mut harness, &beta);

    // + on another project moves the row while the page already shows.
    open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
    assert_row_under(&mut harness, &alpha);

    // Neither the locked picker nor a folder in another project moves it.
    let (chosen, folder) = (beta.clone(), beta_path.clone());
    harness.act(move |root, window, cx| {
        let launcher = root.render_launcher(None, true, window, cx);
        launcher.update(cx, |launcher, cx| {
            launcher.choose_project(&chosen, cx);
            launcher.project.picked(&folder, cx);
        });
    });
    assert_row_under(&mut harness, &alpha);
    assert_eq!(
        page_picker(&mut harness),
        (ProjectScope::Project(alpha.clone()), true)
    );

    // ⌘N puts it in Recents, where the free picker and folder don't move it.
    harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
    assert_row_in_recents(&mut harness);
    let chosen = beta.clone();
    harness.act(move |root, window, cx| {
        let launcher = root.render_launcher(None, true, window, cx);
        launcher.update(cx, |launcher, cx| {
            let picker = launcher.project.select();
            picker.update(cx, |select, cx| select.set_value(chosen.clone(), cx));
            launcher.choose_project(&chosen, cx);
        });
    });
    assert_row_in_recents(&mut harness);
    assert_eq!(
        page_picker(&mut harness),
        (ProjectScope::Project(beta.clone()), false)
    );
    harness.act(move |root, window, cx| {
        let launcher = root.render_launcher(None, true, window, cx);
        launcher.update(cx, |launcher, cx| launcher.project.picked(&alpha_path, cx));
    });
    assert_row_in_recents(&mut harness);
    assert_eq!(
        page_picker(&mut harness),
        (ProjectScope::Project(alpha.clone()), false)
    );

    // + on Recents is the same, from a page locked to a project too.
    open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
    open_plus(&mut harness, ProjectScope::Root);
    assert_row_in_recents(&mut harness);
    assert_eq!(page_picker(&mut harness), (ProjectScope::Root, false));

    // With nowhere to draw it, New tab keeps the selection.
    update_settings(&mut harness, |settings| {
        settings.sidebar_active_only = true;
        true
    });
    assert_eq!(row_scope(&harness), None);
    assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB").is_none());
    update_settings(&mut harness, |settings| {
        settings.sidebar_active_only = false;
        true
    });

    // Leaving the page removes it.
    harness.act(|root, window, cx| root.open_roles(window, cx));
    assert_eq!(row_scope(&harness), None);
    assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB").is_none());
}

#[test]
fn new_tab_entries_use_the_recent_tabs_project_and_mission_entries_keep_the_active_one() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let projects = project_fixture(&mut harness);
    let alpha = projects[0].id.clone();
    for entry in [
        "shortcut",
        "sidebar row",
        "recents plus",
        "shortcut on a locked page",
    ] {
        open_tab_in(&mut harness, Some(&alpha));
        let default_directory =
            harness.read_root(|root, cx| local_default_directory(root.settings(cx)).unwrap());
        match entry {
            "shortcut" => {
                harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
            }
            "sidebar row" => {
                let row = harness.bounds("SIDEBAR_WORKSPACE_NEW_TAB").unwrap();
                harness
                    .visual
                    .simulate_click(row.center(), gpui::Modifiers::default());
                harness.visual.run_until_parked();
            }
            "recents plus" => open_plus(&mut harness, ProjectScope::Root),
            _ => {
                open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
                assert_row_under(&mut harness, &alpha);
                assert_eq!(
                    page_picker(&mut harness),
                    (ProjectScope::Project(alpha.clone()), true)
                );
                assert_eq!(page_directory(&mut harness), projects[0].cwd);
                harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
            }
        }
        harness.read_root(|root, _| assert_eq!(root.route, AppRoute::Launcher, "{entry}"));
        assert_row_in_recents(&mut harness);
        let project = (entry != "recents plus").then(|| alpha.clone());
        assert_eq!(
            page_picker(&mut harness),
            (ProjectScope::or_root(project.clone()), false),
            "{entry}"
        );
        assert_eq!(
            page_directory(&mut harness),
            if project.is_some() {
                projects[0].cwd.clone()
            } else {
                default_directory
            },
            "{entry}"
        );
        harness.read_root(|root, cx| assert_eq!(root.active_project_id(cx), None));
    }

    // ⇧⌘M keeps the project the tab is in, still editable, with the row in
    // Recents.
    open_tab_in(&mut harness, Some(&alpha));
    harness.act(|root, window, cx| root.new_mission_action(&NewMission, window, cx));
    harness.read_root(|root, _| assert_eq!(root.route, AppRoute::Launcher));
    assert_row_in_recents(&mut harness);
    assert_eq!(
        page_picker(&mut harness),
        (ProjectScope::Project(alpha.clone()), false)
    );

    // So do it and a crew start from a page a project's + locked, keeping
    // the page's folder and where it closes back to.
    for crew in [None, Some("crew")] {
        let tab = open_tab_in(&mut harness, Some(&alpha));
        open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
        assert_row_under(&mut harness, &alpha);
        let folder = tempfile::tempdir().unwrap();
        let folder = folder.path().to_string_lossy().into_owned();
        let changed = folder.clone();
        harness.act(move |root, window, cx| {
            let launcher = root.render_launcher(None, true, window, cx);
            launcher.update(cx, |launcher, cx| {
                launcher.project.picked(&changed, cx);
                launcher
                    .cwd
                    .update(cx, |cwd, cx| cwd.reset(changed.clone(), cx));
            });
        });
        let scope = ProjectScope::Project(alpha.clone());
        harness.act(move |root, window, cx| {
            root.open_mission_launcher(crew.map(str::to_owned), scope, window, cx);
        });
        assert_row_in_recents(&mut harness);
        assert_eq!(
            page_picker(&mut harness),
            (ProjectScope::Project(alpha.clone()), false)
        );
        assert_eq!(page_directory(&mut harness), folder);
        harness.act(close_tab);
        harness.read_root(|root, _| {
            assert_eq!(root.route, AppRoute::Chat);
            assert_eq!(root.tabs.active_tab_id(), Some(tab.as_str()));
        });
    }

    // The page's own Mission row is not an entry: the lock stays.
    open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
    harness.act(|root, window, cx| {
        let launcher = root.render_launcher(None, true, window, cx);
        launcher.update(cx, |launcher, cx| {
            launcher.start(Choice::Mission, false, window, cx);
        });
    });
    assert_row_under(&mut harness, &alpha);
    assert_eq!(
        page_picker(&mut harness),
        (ProjectScope::Project(alpha), true)
    );
}

#[test]
fn new_tab_uses_the_opening_tab_instead_of_the_pickers_project_and_falls_back_if_gone() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let projects = project_fixture(&mut harness);
    let alpha = projects[0].id.clone();
    let beta = projects[1].id.clone();
    let fallback = open_tab_in(&mut harness, Some(&beta));
    let opening = open_tab_in(&mut harness, Some(&alpha));
    harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
    let chosen = beta.clone();
    harness.act(move |root, window, cx| {
        let launcher = root.render_launcher(None, true, window, cx);
        launcher.update(cx, |launcher, cx| launcher.choose_project(&chosen, cx));
        root.open_new_tab_modal(&NewTab, window, cx);
    });
    assert_eq!(
        page_picker(&mut harness),
        (ProjectScope::Project(alpha), false)
    );
    assert_eq!(page_directory(&mut harness), projects[0].cwd);
    harness.act(move |root, window, cx| {
        root.front_page_client(cx)
            .archive(runner_core::companion::ActionTarget {
                kind: runner_core::companion::ActionKind::Tab,
                id: opening,
            })
            .unwrap();
        root.reload_tabs(cx).unwrap();
        assert_eq!(root.recently_viewed_tab_id(cx), Some(fallback));
        root.open_new_tab_modal(&NewTab, window, cx);
    });
    assert_eq!(
        page_picker(&mut harness),
        (ProjectScope::Project(beta), false)
    );
    assert_eq!(page_directory(&mut harness), projects[1].cwd);

    open_tab_in(&mut harness, None);
    harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
    assert_eq!(page_picker(&mut harness), (ProjectScope::Root, false));
    assert_row_in_recents(&mut harness);
}

#[test]
fn a_preselected_project_keeps_sidebar_selection_in_recents_when_the_row_is_hidden() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let alpha = project_fixture(&mut harness)[0].id.clone();
    for active_only in [false, true] {
        open_tab_in(&mut harness, Some(&alpha));
        let collapsed = alpha.clone();
        update_settings(&mut harness, move |settings| {
            settings.sidebar_chats_open = active_only;
            settings.sidebar_active_only = active_only;
            settings.sidebar_collapsed_projects.insert(collapsed);
            true
        });
        harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
        assert_eq!(
            page_picker(&mut harness),
            (ProjectScope::Project(alpha.clone()), false)
        );
        assert_eq!(row_scope(&harness), None);
        assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB").is_none());
        harness.read_root(|root, cx| {
            assert_eq!(root.active_project_id(cx), None);
            assert!(root
                .settings(cx)
                .sidebar_collapsed_projects
                .contains(&alpha));
        });
    }
}

#[test]
fn closing_the_project_picker_returns_focus_to_search_and_arrows_move_launcher_rows() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    project_fixture(&mut harness);
    open_tab_in(&mut harness, None);
    harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
    for close in ["mouse", "keyboard", "escape"] {
        let picker = harness.bounds("STYLED_SELECT_launcher-project").unwrap();
        harness
            .visual
            .simulate_click(picker.center(), Default::default());
        harness.visual.run_until_parked();
        match close {
            "mouse" => {
                let option = harness.bounds("STYLED_SELECT_OPTION_1").unwrap();
                harness
                    .visual
                    .simulate_click(option.center(), Default::default());
            }
            "keyboard" => harness.visual.simulate_keystrokes("home down enter"),
            _ => harness.visual.simulate_keystrokes("escape"),
        }
        harness.visual.run_until_parked();
        let mut selected = 0;
        harness.act(|root, window, cx| {
            let launcher = root.render_launcher(None, true, window, cx);
            assert!(
                launcher
                    .read(cx)
                    .query
                    .read(cx)
                    .focus_handle()
                    .is_focused(window),
                "{close}"
            );
            selected = launcher.read(cx).selected;
        });
        harness.visual.simulate_keystrokes("down");
        harness.visual.run_until_parked();
        harness.act(|root, window, cx| {
            let launcher = root.render_launcher(None, true, window, cx);
            assert_ne!(launcher.read(cx).selected, selected, "{close}");
        });
    }
}

#[test]
fn page_close_row_close_and_close_tab_return_to_where_the_launcher_page_opened() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);

    // The window's empty state has nothing behind it: no ×, and ⌘W keeps it.
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        root.open_new_tab_modal(&NewTab, window, cx);
    });
    assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB").is_some());
    assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB_CLOSE").is_none());
    assert!(harness.bounds("LAUNCHER_PAGE_CLOSE").is_none());
    harness.act(close_tab);
    assert!(harness.read_root(|root, _| root.launcher_page_visible()));

    let mut previous = String::new();
    harness.act(|root, window, cx| {
        root.test_open_empty_tab(window, cx);
        previous = root.tabs.active_tab_id().unwrap().to_owned();
        root.open_new_tab_modal(&NewTab, window, cx);
    });

    // The page's × sits in its top-right corner, and the row's does the same.
    for selector in ["LAUNCHER_PAGE_CLOSE", "SIDEBAR_LAUNCHER_TAB_CLOSE"] {
        let close = harness.bounds(selector).expect("close button");
        if selector == "LAUNCHER_PAGE_CLOSE" {
            let page = harness.bounds("LAUNCHER_PAGE").expect("launcher page");
            assert!(close.right() <= page.right());
            assert!(page.right() - close.right() < px(16.));
            assert!(close.top() - page.top() < px(16.));
        }
        harness
            .visual
            .simulate_click(close.center(), gpui::Modifiers::default());
        harness.visual.run_until_parked();
        harness.read_root(|root, _| {
            assert_eq!(root.route, AppRoute::Chat);
            assert_eq!(root.tabs.active_tab_id(), Some(previous.as_str()));
        });
        assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB").is_none());
        assert!(harness.bounds("LAUNCHER_PAGE_CLOSE").is_none());
        harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
    }
    harness.act(close_tab);

    harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
    harness.act(close_tab);
    harness.read_root(|root, _| {
        assert_eq!(root.route, AppRoute::Chat);
        assert_eq!(root.tabs.active_tab_id(), Some(previous.as_str()));
        assert_eq!(root.tabs.tabs().len(), 1);
    });

    // ⌘W from the goal step closes the page too, and returns to a page.
    harness.act(|root, window, cx| {
        root.open_crews(window, cx);
        root.open_launcher_page(ProjectScope::Root, Step::Goal(None), false, window, cx);
    });
    harness.act(close_tab);
    harness.read_root(|root, _| assert_eq!(root.route, AppRoute::Crews));
}

#[test]
fn closing_the_active_tab_lands_on_the_last_viewed_tab_with_no_launcher_between() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let tabs = saved_tabs(
        &mut harness,
        &[("first-chat", "codex"), ("viewed-chat", "codex")],
    );
    let close_active_empty_tab = |harness: &mut ModalHarness| {
        harness.act(|root, window, cx| {
            root.test_open_empty_tab(window, cx);
            root.tab_landed_at = None;
        });
        harness.act(close_tab);
        harness.read_root(|root, _| assert_eq!(root.tabs.tabs().len(), 2));
    };

    close_active_empty_tab(&mut harness);
    assert_landed_on(&mut harness, &tabs[1], "viewed-chat");
    assert!(harness.bounds("LAUNCHER_PAGE").is_none());
    assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB").is_none());

    // The tab in another window is passed over, though this window is the
    // more recently focused and would keep the chat.
    open_in_second_window(&mut harness, "viewed-chat");
    close_active_empty_tab(&mut harness);
    assert_landed_on(&mut harness, &tabs[0], "first-chat");

    // With every tab open elsewhere, the last viewed wins.
    harness.act(|root, window, cx| {
        root.tabs.activate(&tabs[1]);
        root.ensure_active_tab_attached(window, cx).unwrap();
        root.settle_active_tab(window, cx);
        root.core(cx).window_register("third").unwrap();
        root.core(cx)
            .report_subjects(
                "third",
                vec![runner_core::protocol::window::Subject::DirectChat(
                    "first-chat".into(),
                )],
                None,
            )
            .unwrap();
    });
    close_active_empty_tab(&mut harness);
    assert_landed_on(&mut harness, &tabs[1], "viewed-chat");
    harness.act(|root, _, cx| {
        for window in ["second", "third"] {
            root.core(cx)
                .report_subjects(window, Vec::new(), None)
                .unwrap();
        }
    });

    // With no tab in the navigation history, it lands on the first tab.
    harness.act(|root, window, cx| {
        root.test_open_empty_tab(window, cx);
        root.tab_landed_at = None;
        root.runtime_navigation_history.clear();
        root.runtime_navigation_index = None;
    });
    harness.act(close_tab);
    assert_landed_on(&mut harness, &tabs[0], "first-chat");
}

#[test]
fn archiving_a_mission_on_screen_lands_on_the_last_viewed_tab() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let tabs = saved_tabs(
        &mut harness,
        &[("first-chat", "codex"), ("viewed-chat", "codex")],
    );
    harness.act(|root, window, cx| {
        root.set_route(AppRoute::Mission("mission".into()), cx);
        root.tabs.activate(&tabs[0]);
        root.leave_archived_mission("mission", Some(false), true, window, cx);
    });
    assert_landed_on(&mut harness, &tabs[1], "viewed-chat");
    harness.read_root(|root, _| assert!(root.tab_close_guarded()));
}

#[test]
fn with_no_tab_left_the_launcher_is_recents_with_no_project_and_nothing_to_close() {
    for path in ["empty tab", "mission", "esc"] {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let projects = project_fixture(&mut harness);
        let alpha = projects[0].id.clone();
        let tab = open_tab_in(&mut harness, Some(&alpha));
        // The page launcher is cached between visits, locked to the project.
        open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
        assert_row_under(&mut harness, &alpha);
        match path {
            "empty tab" => {
                let tab = tab.clone();
                harness.act(move |root, window, cx| {
                    root.tabs.activate(&tab);
                    root.set_route(AppRoute::Chat, cx);
                    root.ensure_active_tab_attached(window, cx).unwrap();
                });
                harness.act(close_tab);
            }
            "mission" => {
                let tab = tab.clone();
                harness.act(move |root, window, cx| {
                    root.set_route(AppRoute::Mission("mission".into()), cx);
                    root.front_page_client(cx)
                        .archive(runner_core::companion::ActionTarget {
                            kind: runner_core::companion::ActionKind::Tab,
                            id: tab,
                        })
                        .unwrap();
                    root.reload_tabs(cx).unwrap();
                    root.leave_archived_mission("mission", Some(false), true, window, cx);
                });
            }
            _ => {
                let tab = tab.clone();
                harness.act(move |root, _, cx| {
                    root.front_page_client(cx)
                        .archive(runner_core::companion::ActionTarget {
                            kind: runner_core::companion::ActionKind::Tab,
                            id: tab,
                        })
                        .unwrap();
                    root.reload_tabs(cx).unwrap();
                    assert_eq!(root.route, AppRoute::Launcher);
                });
                harness.visual.simulate_keystrokes("escape");
                harness.visual.run_until_parked();
            }
        }
        harness.read_root(|root, _| {
            assert_eq!(root.route, AppRoute::Chat, "{path}");
            assert!(root.tabs.tabs().is_empty(), "{path}");
            assert!(root.launcher_is_empty_state(), "{path}");
        });
        assert_row_in_recents(&mut harness);
        assert_eq!(
            page_picker(&mut harness),
            (ProjectScope::Root, false),
            "{path}"
        );
        assert_ne!(page_directory(&mut harness), projects[0].cwd, "{path}");
        assert!(harness.bounds("LAUNCHER_PAGE_CLOSE").is_none(), "{path}");
        assert!(
            harness.bounds("SIDEBAR_LAUNCHER_TAB_CLOSE").is_none(),
            "{path}"
        );
    }
}

#[test]
fn closing_the_last_tab_while_the_page_shows_leaves_the_page_alone() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let projects = project_fixture(&mut harness);
    let alpha = projects[0].id.clone();
    let tab = open_tab_in(&mut harness, Some(&alpha));
    open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
    harness.act(|root, _, cx| {
        root.front_page_client(cx)
            .archive(runner_core::companion::ActionTarget {
                kind: runner_core::companion::ActionKind::Tab,
                id: tab,
            })
            .unwrap();
        root.reload_tabs(cx).unwrap();
        assert_eq!(root.route, AppRoute::Launcher);
    });
    assert_row_under(&mut harness, &alpha);
    assert_eq!(
        page_picker(&mut harness),
        (ProjectScope::Project(alpha), true)
    );
}

#[test]
fn a_locked_page_files_under_its_project_retargets_and_falls_back_when_it_is_deleted() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let projects = project_fixture(&mut harness);
    let (alpha, beta) = (projects[0].id.clone(), projects[1].id.clone());
    let outside = tempfile::tempdir().unwrap();
    let outside = outside.path().to_string_lossy().into_owned();
    harness.act(|root, _, cx| {
        root.start_chat_modal = None;
        let nodes = root.core(cx).node_list().unwrap();
        root.app_store.update(cx, |store, cx| {
            store.nodes = nodes;
            cx.notify();
        });
    });

    // A folder outside the project stays filed under it, and the menu is off.
    open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
    let folder = outside.clone();
    harness.act(move |root, window, cx| {
        let launcher = root.render_launcher(None, true, window, cx);
        launcher.update(cx, |launcher, cx| {
            launcher.project.picked(&folder, cx);
            launcher
                .cwd
                .update(cx, |cwd, cx| cwd.reset(folder.clone(), cx));
            launcher.changed_directory = true;
        });
    });
    assert_eq!(
        page_picker(&mut harness),
        (ProjectScope::Project(alpha.clone()), true)
    );
    assert_eq!(page_directory(&mut harness), outside);
    let trigger = harness.bounds("STYLED_SELECT_launcher-project").unwrap();
    harness
        .visual
        .simulate_click(trigger.center(), gpui::Modifiers::default());
    harness.visual.run_until_parked();
    assert!(harness.bounds("STYLED_SELECT_MENU").is_none());

    // + on another project takes the row, the lock and the folder with it.
    open_plus(&mut harness, ProjectScope::Project(beta.clone()));
    assert_row_under(&mut harness, &beta);
    assert_eq!(
        page_picker(&mut harness),
        (ProjectScope::Project(beta.clone()), true)
    );
    assert_eq!(page_directory(&mut harness), projects[1].cwd);

    // A deleted project gives the page back.
    let deleted = beta.clone();
    harness.act(move |root, _, cx| {
        root.app_store.update(cx, |store, cx| {
            store.projects.retain(|project| project.id != deleted);
            cx.notify();
        });
    });
    harness.act(|_, _, _| {});
    assert_row_in_recents(&mut harness);
    assert_eq!(page_picker(&mut harness), (ProjectScope::Root, false));
    assert_ne!(page_directory(&mut harness), projects[1].cwd);
}

#[cfg(unix)]
#[test]
fn a_second_close_right_after_a_landing_leaves_the_terminal_it_landed_on() {
    use std::time::{Duration, Instant};

    fn close_until_gone(harness: &mut ModalHarness, session: &'static str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut gone = false;
            harness.act(|root, _, cx| {
                root.reload_tabs(cx).unwrap();
                gone = root.session_entry(session, cx).is_none()
                    && !root.stopping_sessions.contains(session);
            });
            if gone {
                return;
            }
            assert!(Instant::now() < deadline, "{session} did not close");
        }
    }

    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let tabs = saved_tabs(
        &mut harness,
        &[
            ("kept-shell", "shell"),
            ("landing-shell", "shell"),
            ("closing-shell", "shell"),
        ],
    );
    // The history runs kept, landing, closing.
    harness.act(|root, window, cx| {
        for tab in [&tabs[0], &tabs[1], &tabs[2]] {
            root.tabs.activate(tab);
            root.ensure_active_tab_attached(window, cx).unwrap();
            root.settle_active_tab(window, cx);
        }
    });

    harness.act(close_tab);
    close_until_gone(&mut harness, "closing-shell");
    assert_landed_on(&mut harness, &tabs[1], "landing-shell");

    // ⌘W and the pane's × do nothing inside the guard.
    harness.act(close_tab);
    let close = harness.bounds("PANE_CLOSE:p1").expect("pane close");
    harness
        .visual
        .simulate_click(close.center(), gpui::Modifiers::default());
    harness.visual.run_until_parked();
    harness.act(|root, _, cx| {
        root.reload_tabs(cx).unwrap();
        assert!(root.stopping_sessions.is_empty());
        assert!(root.session_entry("landing-shell", cx).is_some());
        assert_eq!(root.tabs.active_tab_id(), Some(tabs[1].as_str()));
    });

    // A later press closes it, and lands again.
    harness.act(|root, _, _| {
        root.tab_landed_at = Some(Instant::now() - TAB_LANDING_CLOSE_GUARD);
    });
    harness.act(close_tab);
    close_until_gone(&mut harness, "landing-shell");
    assert_landed_on(&mut harness, &tabs[0], "kept-shell");
    harness.read_root(|root, _| assert!(root.tab_close_guarded()));
}

#[test]
fn sidebar_plus_keeps_reused_page_project_picker_and_start_scope_in_sync() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let projects = project_fixture(&mut harness);
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        root.open_new_tab_modal(&NewTab, window, cx);
    });
    for (step, project) in [
        (Step::Start, Some(projects[0].id.clone())),
        (Step::Crews, Some(projects[1].id.clone())),
        (Step::Goal(None), None),
    ] {
        harness.act(|root, window, cx| {
            let launcher = root.render_launcher(None, true, window, cx);
            launcher.update(cx, |launcher, cx| launcher.open_step(step, window, cx));
            root.open_sidebar_chat_modal(ProjectScope::or_root(project.clone()), window, cx);
        });
        harness.act(|root, window, cx| {
            let launcher = root.render_launcher(None, true, window, cx);
            let launcher = launcher.read(cx);
            assert_eq!(launcher.step, Step::Start);
            assert_eq!(launcher.scope(), ProjectScope::or_root(project.clone()));
            assert_eq!(
                launcher.project.select().read(cx).value(),
                project.as_deref().unwrap_or_default()
            );
            assert_eq!(
                launcher.project.select().read(cx).is_disabled(),
                project.is_some()
            );
            assert_eq!(root.launcher_page_project(cx), project);
            if let Some(project) = &project {
                assert_eq!(
                    launcher.cwd.read(cx).text(),
                    projects
                        .iter()
                        .find(|entry| &entry.id == project)
                        .unwrap()
                        .cwd
                );
            }
            assert!(root.tabs.tabs().is_empty());
        });
    }
}

#[test]
fn cached_launchers_follow_split_and_collapse_while_preserving_explicit_change() {
    for changed in [false, true] {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
            root.test_open_empty_tab(window, cx);
            let original = root.tabs.active().unwrap().clone();
            let key = PaneKey::new(&original.id, &original.focused_pane_id);
            let first = root.render_launcher(Some(key.clone()), true, window, cx);
            first.update(cx, |launcher, cx| {
                launcher
                    .query
                    .update(cx, |field, cx| field.reset("find", cx));
                if changed {
                    launcher
                        .cwd
                        .update(cx, |field, cx| field.reset("/chosen", cx));
                    launcher.changed_directory = true;
                }
            });
            root.split_pane(&key.pane_id, SplitOrientation::Row, window, cx);
            let sibling = root.tabs.active().unwrap().focused_pane_id.clone();
            let split = root.render_launcher(Some(key.clone()), false, window, cx);
            assert_ne!(first, split);
            let split_state = split.read(cx);
            assert!(!split_state.default_page);
            assert!(split_state.project.select().read(cx).is_disabled());
            assert_eq!(split_state.query.read(cx).text(), "find");
            if changed {
                assert_eq!(split_state.cwd.read(cx).text(), "/chosen");
            }
            root.close_pane(&sibling, window, cx);
            let single = root.render_launcher(Some(key), true, window, cx);
            assert_ne!(split, single);
            let single = single.read(cx);
            assert!(single.default_page);
            assert!(!single.project.select().read(cx).is_disabled());
            assert_eq!(single.query.read(cx).text(), "find");
            assert_eq!(single.changed_directory, changed);
            if changed {
                assert_eq!(single.cwd.read(cx).text(), "/chosen");
            }
        });
    }
}

struct HeldLocalStart {
    base: runner_daemon::daemon::InProcessTransport,
    calls: std::sync::atomic::AtomicUsize,
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}
impl runner_core::protocol::Transport for HeldLocalStart {
    fn call(
        &self,
        request: runner_core::protocol::Request,
    ) -> Result<runner_core::protocol::Response, runner_core::protocol::ClientError> {
        if matches!(
            &request,
            runner_core::protocol::Request::session_start_runtime_with_speed { .. }
        ) {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            return Ok(
                runner_core::protocol::Response::session_start_runtime_with_speed(Err(
                    runner_core::protocol::ClientError::msg("bounded start failure"),
                )),
            );
        }
        self.base.call(request)
    }
    fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
        self.base.subscribe()
    }
}

#[test]
fn delayed_local_start_blocks_other_panes_and_releases_its_reservation_on_failure() {
    for split in [false, true] {
        use std::sync::{atomic::Ordering, mpsc, Arc, Mutex};
        let mut harness = modal_harness(1800., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let (attempted, attempts) = mpsc::channel();
        let observer = std::thread::spawn(move || {
            let started = started.recv_timeout(std::time::Duration::from_secs(5));
            let attempted = attempts.recv_timeout(std::time::Duration::from_secs(5));
            let _ = release.send(());
            (started, attempted)
        });
        let mut trace = None;
        let mut tab = String::new();
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
            let core = root.app_store.read(cx).test_core.clone();
            let transport = Arc::new(HeldLocalStart {
                base: runner_daemon::daemon::InProcessTransport(core),
                calls: Default::default(),
                entered,
                release: Mutex::new(blocked),
            });
            root.app_store.update(cx, |store, _| {
                store.client = runner_core::protocol::DaemonClient::new(transport.clone())
            });
            trace = Some(transport);
            root.test_open_empty_tab(window, cx);
            let first = root.front_page_layout().unwrap().focused_pane_id.clone();
            if split {
                root.split_pane(&first, SplitOrientation::Row, window, cx);
            }
            let original = root.front_page_layout().unwrap().clone();
            tab = original.id.clone();
            let second = original.focused_pane_id.clone();
            for pane in [&first, &second] {
                let view = root.render_launcher(Some(PaneKey::new(&tab, pane)), false, window, cx);
                view.update(cx, |launcher, _| launcher.runtimes = vec![agent()]);
            }
            root.focus_pane(&first, cx);
            root.test_start_launcher(window, cx);
            assert!(root.launcher_tab_pending(&tab, cx));
            root.focus_pane(&second, cx);
            root.test_start_launcher(window, cx);
            crate::surfaces::windowing::close_tab(root, window, cx);
            root.close_pane(&second, window, cx);
            root.split_pane(&second, SplitOrientation::Row, window, cx);
            assert_eq!(root.front_page_layout().unwrap().root, original.root);
            attempted.send(()).unwrap();
        });
        let (started, attempted) = observer.join().unwrap();
        started.unwrap();
        attempted.unwrap();
        assert_eq!(trace.unwrap().calls.load(Ordering::SeqCst), 1);
        harness.act(|root, _, cx| assert!(!root.launcher_tab_pending(&tab, cx)));
    }
}

#[test]
fn daemon_role_changes_refresh_the_catalog_in_an_existing_launcher() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let mut view = None;
    let mut role_id = String::new();
    harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
            view = Some(root.render_launcher(None, true, window, cx));
            role_id = root.core(cx).role_create(serde_json::from_value(serde_json::json!({
                "handle": "qa-launcher", "display_name": "QA launcher", "runtime": "codex", "command": "codex", "working_dir": "/created"
            })).unwrap()).unwrap().id;
        });
    harness.act(|root, window, cx| {
        let cached = root.render_launcher(None, true, window, cx);
        assert_eq!(cached, *view.as_ref().unwrap());
        let role = cached
            .read(cx)
            .roles
            .iter()
            .find(|role| role.id == role_id)
            .expect("created role reaches the launcher through its daemon event");
        assert_eq!(role.working_dir.as_deref(), Some("/created"));
        root.core(cx)
            .role_update(
                &role_id,
                serde_json::from_value(
                    serde_json::json!({"display_name":"Edited launcher", "working_dir":"/edited"}),
                )
                .unwrap(),
            )
            .unwrap();
    });
    harness.act(|root, window, cx| {
        let cached = root.render_launcher(None, true, window, cx);
        let role = cached
            .read(cx)
            .roles
            .iter()
            .find(|role| role.id == role_id)
            .unwrap();
        assert_eq!(role.display_name, "Edited launcher");
        assert_eq!(role.working_dir.as_deref(), Some("/edited"));
        root.core(cx).role_delete(&role_id).unwrap();
    });
    harness.act(|root, window, cx| {
        let cached = root.render_launcher(None, true, window, cx);
        assert!(!cached.read(cx).roles.iter().any(|role| role.id == role_id));
    });
}

#[test]
fn cli_project_create_and_rename_update_an_open_launcher_picker() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let directory = tempfile::tempdir().unwrap();
    let mut view = None;
    let mut project_id = String::new();
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        root.set_route(AppRoute::Chat, cx);
        view = Some(root.render_launcher(None, true, window, cx));
        project_id = root
            .core(cx)
            .project_create_checked(
                "CLI project".into(),
                directory.path().to_string_lossy().into_owned(),
            )
            .unwrap()
            .id;
    });
    harness.act(|root, window, cx| {
        let cached = root.render_launcher(None, true, window, cx);
        assert_eq!(cached, *view.as_ref().unwrap());
        cached.update(cx, |launcher, cx| {
            launcher.project.choose(&project_id, "");
            let picker = launcher.project.select();
            picker.update(cx, |select, cx| select.set_value(project_id.clone(), cx));
            assert_eq!(picker.read(cx).selected().unwrap().label, "CLI project");
        });
        root.core(cx)
            .project_rename(project_id.clone(), "Renamed CLI project".into())
            .unwrap();
    });
    harness.act(|root, window, cx| {
        let cached = root.render_launcher(None, true, window, cx);
        assert_eq!(cached, *view.as_ref().unwrap());
        let picker = cached.read(cx).project.select();
        assert_eq!(
            picker.read(cx).selected().unwrap().label,
            "Renamed CLI project"
        );
    });
}

#[test]
fn cached_launcher_reconciles_role_and_project_edits_without_losing_search_or_change() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let projects = project_fixture(&mut harness);
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        let original = root.render_launcher(None, true, window, cx);
        original.update(cx, |launcher, cx| {
            launcher
                .query
                .update(cx, |field, cx| field.reset("coder", cx));
            launcher
                .cwd
                .update(cx, |field, cx| field.reset("/changed", cx));
            launcher.changed_directory = true;
            launcher.project.choose(&projects[0].id, "/changed");
        });
        let role = crate::surfaces::start_chat::tests::test_role("coder", "codex");
        root.app_store
            .update(cx, |store, cx| store.replace_roles(vec![role.clone()], cx));
        let reused = root.render_launcher(None, true, window, cx);
        assert_eq!(original, reused);
        assert_eq!(reused.read(cx).roles.len(), 1);
        root.app_store.update(cx, |store, _| {
            store.roles[0].working_dir = Some("/edited-role".into());
            store.projects[0].name = "Renamed project".into();
            store.revisions.roles += 1;
            store.revisions.projects += 1;
        });
        root.render_launcher(None, true, window, cx);
        let cached = reused.read(cx);
        assert_eq!(cached.roles[0].working_dir.as_deref(), Some("/edited-role"));
        assert_eq!(cached.query.read(cx).text(), "coder");
        assert_eq!(cached.cwd.read(cx).text(), "/changed");
        assert!(cached.changed_directory);
        assert_eq!(cached.project.chosen().unwrap().name, "Renamed project");
        reused.update(cx, |launcher, _| {
            launcher.project.choose(&projects[0].id, "/changed");
        });
        root.app_store.update(cx, |store, _| {
            store.roles.clear();
            store.projects.clear();
            store.revisions.roles += 1;
            store.revisions.projects += 1;
        });
        root.render_launcher(None, true, window, cx);
        assert!(reused.read(cx).roles.is_empty());
        assert!(reused.read(cx).project.chosen().is_none());
    });
}

#[test]
fn recency_is_cached_across_redraws_and_reloads_after_a_recorded_use() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let mut view = None;
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        root.set_route(AppRoute::Chat, cx);
        root.app_store.update(cx, |store, cx| {
            store.replace_roles(
                vec![crate::surfaces::start_chat::tests::test_role(
                    "coder", "codex",
                )],
                cx,
            )
        });
        let launcher = root.render_launcher(None, true, window, cx);
        launcher.update(cx, |launcher, _| {
            launcher.runtimes = vec![agent()];
            launcher.default_runtime = "codex".into();
        });
        let directory = launcher.read(cx).app_data_dir.clone();
        let mut disk = RecentStore::default();
        disk.record(
            "local",
            EntryKey::Role("role-coder".into()),
            chrono::Utc::now().timestamp(),
        );
        disk.save(&directory).unwrap();
        for _ in 0..3 {
            root.render_launcher(None, true, window, cx);
            assert_eq!(
                launcher.read(cx).rows(cx).0[0].choice,
                Choice::Chat(EntryKey::Agent("codex".into()))
            );
        }
        launcher.update(cx, |launcher, cx| launcher.reload_recents(cx));
        assert_eq!(
            launcher.read(cx).rows(cx).0[0].choice,
            Choice::Chat(EntryKey::Role("role-coder".into()))
        );
        record_launcher_use(
            &directory,
            &FrontPageTarget::Local,
            EntryKey::Agent("codex".into()),
        );
        root.render_launcher(None, true, window, cx);
        assert_eq!(
            launcher.read(cx).rows(cx).0[0].choice,
            Choice::Chat(EntryKey::Agent("codex".into()))
        );
        view = Some(launcher);
    });
    assert!(harness.visual.debug_bounds("LAUNCHER_BADGE:0").is_some());
    harness.act(|_, _, cx| {
        view.as_ref().unwrap().update(cx, |launcher, cx| {
            launcher
                .query
                .update(cx, |field, cx| field.reset("Role coder", cx))
        })
    });
    assert!(harness.visual.debug_bounds("LAUNCHER_BADGE:0").is_none());
    assert!(
        harness.visual.debug_bounds("LAUNCHER_BADGE:1").is_some(),
        "Terminal keeps its shortcut badge while searching"
    );
}

#[test]
fn recency_under_one_hour_reads_just_now() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for seconds in [0, 60, 3599] {
        assert_eq!(
            last_use(Some(chrono::Utc::now().timestamp() - seconds)),
            crate::i18n::text("launcher.just_now")
        );
    }
}

#[test]
fn no_tab_terminal_click_and_shortcut_persist_a_nonempty_tab_id() {
    for shortcut in [false, true] {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
            let view = root.render_launcher(None, true, window, cx);
            if shortcut {
                root.new_terminal_action(&NewTerminal, window, cx);
            } else {
                view.update(cx, |launcher, cx| {
                    launcher.start(Choice::Terminal, false, window, cx)
                });
            }
        });
        harness.act(|root, _, cx| {
            let layout = root.tabs.active().expect("terminal has its own tab");
            assert!(!layout.id.is_empty());
            let session = layout
                .focused_session_id()
                .expect("terminal is assigned")
                .to_owned();
            assert_eq!(root.tabs.tabs().len(), 1);
            root.core(cx).session_close(&session).unwrap();
        });
    }
}

#[test]
fn search_digits_escape_and_arrows_use_the_real_text_field_without_starting_unavailable_roles() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let mut launcher = None;
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        root.set_route(AppRoute::Chat, cx);
        root.test_open_empty_tab(window, cx);
        let first = root.tabs.active().unwrap().focused_pane_id.clone();
        root.split_pane(&first, SplitOrientation::Row, window, cx);
        let layout = root.front_page_layout().unwrap();
        let pane = PaneKey::new(&layout.id, &layout.focused_pane_id);
        let view = root.render_launcher(Some(pane), false, window, cx);
        view.update(cx, |launcher, cx| {
            launcher.roles = vec![role()];
            launcher.runtimes.clear();
            launcher
                .query
                .update(cx, |query, cx| query.reset("Bounded", cx));
            launcher.focus(window, cx);
        });
        launcher = Some(view);
    });
    harness.visual.simulate_keystrokes("2");
    harness.visual.run_until_parked();
    harness.act(|_, _, cx| {
        assert_eq!(
            launcher.as_ref().unwrap().read(cx).query.read(cx).text(),
            "Bounded2"
        )
    });
    harness.visual.simulate_keystrokes("escape down up enter");
    harness.visual.run_until_parked();
    harness.act(|_, _, cx| {
        let launcher = launcher.as_ref().unwrap().read(cx);
        assert_eq!(launcher.query.read(cx).text(), "");
        assert!(launcher.pending.is_none());
        assert_eq!(launcher.selected, 0);
    });
}

#[test]
fn launcher_takes_focus_back_from_the_window_and_tab_so_arrows_move_the_selection() {
    for tab in [false, true] {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let mut launcher = None;
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
            if tab {
                root.open_new_tab_modal(&NewTab, window, cx);
            }
            let pane = root
                .front_page_layout()
                .map(|layout| PaneKey::new(&layout.id, &layout.focused_pane_id));
            let view = root.render_launcher(pane, true, window, cx);
            view.update(cx, |launcher, _| {
                launcher.runtimes = vec![agent()];
                launcher.roles = vec![role()];
            });
            if tab {
                root.chat_focus.focus(window, cx);
            } else {
                root.root_focus.focus(window, cx);
            }
            launcher = Some(view);
        });
        harness.visual.run_until_parked();
        for (keys, expected) in [("down", 1), ("down", 2), ("up up", 0), ("up", usize::MAX)] {
            harness.visual.simulate_keystrokes(keys);
            harness.visual.run_until_parked();
            harness.act(|_, _, cx| {
                let launcher = launcher.as_ref().unwrap().read(cx);
                assert_eq!(
                    launcher.selected,
                    expected.min(launcher.rows(cx).0.len() - 1),
                    "after {keys} with the {} focused",
                    if tab { "tab" } else { "window" }
                )
            });
        }
    }
}

#[test]
fn clicking_the_sidebar_search_keeps_launcher_keys_after_the_palette_closes() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let mut launcher = None;
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        root.set_route(AppRoute::Chat, cx);
        root.open_new_tab_modal(&NewTab, window, cx);
        let view = root.render_launcher(None, true, window, cx);
        view.update(cx, |launcher, cx| {
            launcher.runtimes = vec![agent()];
            launcher.roles = vec![role()];
            launcher.focus(window, cx);
        });
        launcher = Some(view);
    });
    harness.visual.run_until_parked();
    let search = harness.visual.debug_bounds("SIDEBAR_SEARCH").unwrap();
    harness
        .visual
        .simulate_click(search.center(), gpui::Modifiers::default());
    harness.visual.run_until_parked();
    harness.visual.simulate_keystrokes("escape");
    harness.visual.run_until_parked();
    harness.visual.simulate_keystrokes("down");
    harness.visual.run_until_parked();
    harness.act(|_, window, cx| {
        let launcher = launcher.as_ref().unwrap().read(cx);
        assert!(launcher.query.read(cx).focus_handle().is_focused(window));
        assert_eq!(launcher.selected, 1);
    });
}

#[test]
fn command_palette_opens_from_the_launcher_search_and_returns_to_it() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let mut launcher = None;
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        root.set_route(AppRoute::Chat, cx);
        root.open_new_tab_modal(&NewTab, window, cx);
        let view = root.render_launcher(None, true, window, cx);
        view.update(cx, |launcher, cx| launcher.focus(window, cx));
        launcher = Some(view);
    });
    harness.visual.run_until_parked();
    for (keys, focused) in [
        (keymap::platform_default("cmd-k"), false),
        ("escape".to_owned(), true),
    ] {
        harness.visual.simulate_keystrokes(&keys);
        harness.visual.run_until_parked();
        harness.act(|_, window, cx| {
            let launcher = launcher.as_ref().unwrap().read(cx);
            assert_eq!(
                launcher.query.read(cx).focus_handle().is_focused(window),
                focused,
                "after {keys}"
            );
        });
    }
}

#[test]
fn clicking_search_after_a_directory_picker_keeps_launcher_keys_active() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let chosen = tempfile::tempdir().unwrap();
    let mut launcher = None;
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        root.set_route(AppRoute::Chat, cx);
        let view = root.render_launcher(None, true, window, cx);
        view.update(cx, |launcher, cx| {
            launcher.roles = vec![role()];
            launcher.runtimes.clear();
            launcher.change_directory(window, cx);
        });
        launcher = Some(view);
    });
    assert!(harness.visual.did_prompt_for_paths());
    harness
        .visual
        .simulate_path_prompt_response(|_| Some(vec![chosen.path().to_owned()]));
    harness.visual.run_until_parked();
    let bounds = harness.visual.debug_bounds("LAUNCHER_SEARCH").unwrap();
    for position in [
        gpui::point(bounds.center().x, bounds.top() + px(2.)),
        bounds.center(),
    ] {
        harness
            .visual
            .simulate_click(position, gpui::Modifiers::default());
        harness.visual.run_until_parked();
        harness.act(|_, window, cx| {
            assert!(
                launcher
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .query
                    .read(cx)
                    .focus_handle()
                    .is_focused(window),
                "the entire search field focuses its input"
            )
        });
        harness.visual.simulate_keystrokes("1");
        harness.visual.run_until_parked();
        harness.act(|_, _, cx| {
            assert_eq!(
                launcher.as_ref().unwrap().read(cx).query.read(cx).text(),
                ""
            )
        });
        harness.visual.simulate_keystrokes("q 1");
        harness.visual.run_until_parked();
        harness.act(|_, _, cx| {
            assert_eq!(
                launcher.as_ref().unwrap().read(cx).query.read(cx).text(),
                "q1"
            );
            launcher.as_ref().unwrap().update(cx, |launcher, cx| {
                launcher.query.update(cx, |query, cx| query.reset("", cx))
            });
        });
    }
}

#[test]
fn project_trigger_is_inline_with_one_location_icon_and_keeps_its_menu() {
    let mut harness = modal_harness(1800., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    let projects = project_fixture(&mut harness);
    let long_project = tempfile::tempdir().unwrap();
    let long_path = long_project
        .path()
        .join("project-with-a-long-folder-name-for-preserving-the-project-picker-menu-width");
    std::fs::create_dir_all(&long_path).unwrap();
    let mut launcher = None;
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        root.set_route(AppRoute::Chat, cx);
        root.chat_panel_visibility = SidebarVisibilityTransition::new(false);
        root.core(cx)
            .project_create_checked(
                "Zeta destination with a long project name and directory preview".into(),
                long_path.to_string_lossy().into_owned(),
            )
            .unwrap();
        root.open_launcher_page(
            ProjectScope::Project(projects[0].id.clone()),
            Step::Start,
            false,
            window,
            cx,
        );
        launcher = Some(root.render_launcher(None, true, window, cx));
    });
    for zoom in [1., 1.3] {
        harness.act(|root, window, cx| root.set_zoom(zoom, window, cx));
        let line = harness.visual.debug_bounds("LAUNCHER_LOCATION").unwrap();
        let trigger = harness
            .visual
            .debug_bounds("STYLED_SELECT_launcher-project")
            .unwrap();
        let icon = harness
            .visual
            .debug_bounds("LAUNCHER_LOCATION_FOLDER")
            .unwrap();
        assert!((line.size.height - px(20. * zoom)).abs() < px(0.1));
        assert!((trigger.size.height - px(20. * zoom)).abs() < px(0.1));
        assert!((trigger.center().y - line.center().y).abs() < px(0.1));
        assert!(
            (icon.size.height - px(16. * zoom)).abs() <= px(1.),
            "zoom {zoom}: {icon:?}"
        );
        assert!(
            harness
                .visual
                .debug_bounds("STYLED_SELECT_TRIGGER_LEADING")
                .is_none(),
            "the location line owns the single folder icon"
        );
    }
    let trigger = harness
        .visual
        .debug_bounds("STYLED_SELECT_launcher-project")
        .unwrap();
    harness
        .visual
        .simulate_click(trigger.center(), gpui::Modifiers::default());
    harness.visual.run_until_parked();
    let menu = harness.visual.debug_bounds("STYLED_SELECT_MENU").unwrap();
    let trigger = harness
        .visual
        .debug_bounds("STYLED_SELECT_launcher-project")
        .unwrap();
    assert!(
        (menu.left() - trigger.left()).abs() <= px(1.),
        "the existing menu stays left-anchored: trigger={trigger:?}, menu={menu:?}"
    );
    assert!(
        menu.size.width > px(240. * 1.3 + 1.),
        "long metadata retains content-based width: {menu:?}"
    );
    assert!(menu.size.width <= px(480. * 1.3 + 1.));
    harness.visual.simulate_keystrokes("down enter");
    harness.visual.run_until_parked();
    harness.act(|root, window, cx| {
        assert_eq!(
            launcher.as_ref().unwrap().read(cx).scope(),
            ProjectScope::Project(projects[1].id.clone())
        );
        root.test_open_empty_tab(window, cx);
        let pane = root.front_page_layout().unwrap().focused_pane_id.clone();
        root.split_pane(&pane, SplitOrientation::Row, window, cx);
    });
    assert!(harness
        .visual
        .debug_bounds("STYLED_SELECT_launcher-project")
        .is_none());
    assert!(harness
        .visual
        .debug_bounds("STYLED_SELECT_TRIGGER_LEADING")
        .is_none());
    assert!(harness.visual.debug_bounds("LAUNCHER_LOCATION").is_none());
    assert!(
        harness.visual.debug_bounds("LAUNCHER_SEARCH").is_some(),
        "a split pane shows its search field with no project or folder control"
    );
}

#[test]
fn a_short_split_measures_its_launcher_and_drops_recents_to_fit() {
    let mut harness = modal_harness(1200., 440., Vec::new(), Vec::new(), ChatMode::Runtime);
    let mut launcher = None;
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        root.set_route(AppRoute::Chat, cx);
        root.test_open_empty_tab(window, cx);
        let first = root.front_page_layout().unwrap().focused_pane_id.clone();
        root.split_pane(&first, SplitOrientation::Column, window, cx);
        let layout = root.front_page_layout().unwrap().clone();
        let pane = PaneKey::new(&layout.id, &layout.focused_pane_id);
        let view = root.render_launcher(Some(pane), false, window, cx);
        view.update(cx, |launcher, _| {
            launcher.runtimes = vec![agent()];
            launcher.roles = (1..=4)
                .map(|n| RoleChoice {
                    id: format!("role-{n}"),
                    display_name: format!("Role {n}"),
                    ..role()
                })
                .collect();
        });
        launcher = Some(view);
    });
    harness.visual.run_until_parked();
    harness.act(|_, _, cx| {
        let launcher = launcher.as_ref().unwrap().read(cx);
        let height = launcher.height.expect("the launcher measures its pane");
        let rows = launcher.rows(cx).0;
        let ranked = rows
            .iter()
            .filter(|row| matches!(row.choice, Choice::Chat(_)))
            .count();
        assert!(height < 225., "the split is short: {height}");
        assert!((1..4).contains(&ranked), "{ranked} recents in {height}");
        assert_eq!((ranked, rows.len() - ranked), model::fit_rows(height, 4, 1));
    });
    harness.act(|root, _, cx| {
        let layout = root.tabs.active_mut().unwrap();
        let focused = layout.focused_pane_id.clone();
        let runner_core::pane_layout::PaneNode::Split(split) = &layout.root else {
            panic!("a split tab");
        };
        let sizes = if split.a.leaves()[0].id == focused {
            [15., 85.]
        } else {
            [85., 15.]
        };
        let id = split.id.clone();
        assert!(layout.set_split_sizes(&id, sizes));
        cx.notify();
    });
    harness.visual.run_until_parked();
    harness.act(|_, _, cx| {
        let launcher = launcher.as_ref().unwrap().read(cx);
        let height = launcher.height.unwrap();
        assert!(
            model::search_is_tight(height),
            "the divider at its 15% limit: {height}"
        );
        assert!(launcher.rows(cx).0.is_empty());
    });
    harness.act(|root, _, cx| {
        let layout = root.tabs.active_mut().unwrap();
        let runner_core::pane_layout::PaneNode::Split(split) = &layout.root else {
            panic!("a split tab");
        };
        let id = split.id.clone();
        assert!(layout.set_split_sizes(&id, [50., 50.]));
        cx.notify();
    });
    harness
        .visual
        .simulate_resize(gpui::size(px(1200.), px(240.)));
    harness.visual.run_until_parked();
    harness.act(|_, _, cx| {
        let launcher = launcher.as_ref().unwrap().read(cx);
        let height = launcher.height.unwrap();
        assert!(height < 95., "a window shrunk after the split: {height}");
        assert!(
            launcher.rows(cx).0.is_empty(),
            "below one row only the search field is left in {height}"
        );
    });
}

#[test]
fn capability_hiding_keeps_pinned_rows_out_of_ranking() {
    let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
    harness.act(|root, window, cx| {
        root.start_chat_modal = None;
        let view = root.render_launcher(None, true, window, cx);
        let access = crate::surfaces::front_page::RemoteAccess::new(FrontPageTarget::Remote {
            computer: "host".into(),
            fingerprint: "fp".into(),
            generation: 1,
        });
        let remote = FrontPageClient::remote(root.core(cx).clone(), access.clone());
        view.update(cx, |launcher, cx| {
            launcher.client = remote;
            launcher.runtimes = vec![agent()];
            launcher.roles = (0..6)
                .map(|index| {
                    let mut role = role();
                    role.id = index.to_string();
                    role.display_name = format!("Role {index}");
                    role
                })
                .collect();
            access.set_features(vec![
                "catalog".into(),
                "startChat".into(),
                "layout".into(),
                "startShell".into(),
                "startMission".into(),
            ]);
            launcher
                .query
                .update(cx, |query, cx| query.reset("Role", cx));
            let (rows, total) = launcher.rows(cx);
            assert_eq!(total, 7);
            assert_eq!(rows.len(), 6);
            assert_eq!(rows[4].choice, Choice::Terminal);
            assert_eq!(rows[5].choice, Choice::Mission);
            access.set_features(vec!["catalog".into(), "startChat".into()]);
            assert!(launcher.rows(cx).0.is_empty());
            access.set_features(vec!["layout".into(), "startShell".into()]);
            let rows = launcher.rows(cx).0;
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].choice, Choice::Terminal);
        });
    });
}
