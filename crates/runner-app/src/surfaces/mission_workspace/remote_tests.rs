use super::*;
use crate::surfaces::mission_workspace::composer_model::ComposerPost;
use crate::surfaces::remote_client::{
    tests::{self as remote_test, Fake},
    RemoteItem,
};
use gpui::{TestAppContext, VisualTestContext};
use runner_core::companion::{FeedEntry, Overview};
use std::sync::Arc;

fn overview() -> Overview {
    serde_json::from_value(serde_json::json!({"projects":[],"tabs":[{"id":"remote-tab","project":null,"name":"Remote tab","pinned":false,"panes":{"session":null},"shells":[]}],"missions":[{"id":"remote-mission","project":null,"title":"Host mission","crew":"Duo","state":"running","pinned":false,"members":[{"handle":"coder","session":"member","lead":true}],"asks":[{"question":"q1","from":"coder","prompt":"Same prompt","choices":["yes","no"]},{"question":"q2","from":"coder","prompt":"Same prompt","choices":["yes","no"]}],"shells":[]}],"sessions":{"member":{"kind":"missionSlot","title":"Host member","runtime":"codex","cwd":"C:\\host\\work","status":"working","unread":false}}})).unwrap()
}

fn setup() -> (
    tempfile::TempDir,
    TestAppContext,
    gpui::WindowHandle<NativeRoot>,
    Entity<AppStore>,
    Arc<Fake>,
) {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
    remote_test::enable_missions(&fake);
    fake.set_overview(overview());
    let window = remote_test::add_window(&mut cx, &store, temp.path(), "remote-mission-tests");
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
    (temp, cx, window, store, fake)
}

fn workspace(
    window: gpui::WindowHandle<NativeRoot>,
    cx: &TestAppContext,
) -> Entity<MissionWorkspace> {
    window
        .read_with(cx, |root, _| {
            root.remote_client.remote_mission_workspace().unwrap()
        })
        .unwrap()
}

fn entry(id: &str, kind: serde_json::Value) -> FeedEntry {
    let mut value = serde_json::json!({"id":id,"ts":"2026-10-24T05:00:00Z","from":"coder","to":[]});
    value
        .as_object_mut()
        .unwrap()
        .extend(kind.as_object().unwrap().clone());
    serde_json::from_value(value).unwrap()
}

#[test]
fn mission_drawer_island_stacks_below_mission_with_full_height_rail() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, _, fake) = setup();
    let workspace = workspace(window, &cx);
    let mut data = overview();
    let mut shell = data.sessions["member"].clone();
    shell.kind = runner_core::companion::SessionKind::Shell;
    shell.runtime = None;
    data.sessions.insert("shell".into(), shell);
    data.missions[0].shells = vec!["shell".into()];
    fake.set_overview(data);
    cx.run_until_parked();
    workspace.update(&mut cx, |workspace, cx| {
        workspace.layout.drawer.set_open(true);
        cx.notify();
    });
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(gpui::size(px(1600.), px(1000.)));
    for zoom in [1., 1.3] {
        for collapsed in [false, true] {
            for rail_open in [true, false] {
                window
                    .update(&mut visual, |root, window, cx| {
                        root.set_zoom(zoom, window, cx);
                        root.set_sidebar_collapsed(collapsed, false, cx);
                        root.sidebar_visibility = SidebarVisibilityTransition::new(!collapsed);
                        workspace.update(cx, |workspace, cx| {
                            workspace.set_rail_open(rail_open, cx);
                            workspace.rail_visibility = SidebarVisibilityTransition::new(rail_open);
                            cx.notify();
                        });
                        cx.notify();
                    })
                    .unwrap();
                visual.run_until_parked();
                let area = visual.debug_bounds("APP_CONTENT_COLUMN").unwrap();
                let mission = visual.debug_bounds("MISSION_ISLAND").unwrap();
                let drawer = visual.debug_bounds("MISSION_TERMINAL_DRAWER").unwrap();
                assert_eq!(mission.left(), area.left());
                assert_eq!(mission.top(), area.top());
                let drawer_island = visual.debug_bounds("MISSION_DRAWER_ISLAND").unwrap();
                let row_gutter = visual.debug_bounds("MISSION_DRAWER_GUTTER").unwrap();
                let drawer_header = visual.debug_bounds("DRAWER_HEADER").unwrap();
                assert_eq!(drawer_island.bottom(), area.bottom());
                assert!(
                    (drawer_island.top()
                        - mission.bottom()
                        - px(runner_app::ui::work_card::ISLAND_GAP * zoom))
                    .abs()
                        < px(0.5)
                );
                assert_eq!(row_gutter.top(), mission.bottom());
                assert_eq!(row_gutter.bottom(), drawer_island.top());
                assert!((drawer_island.size.height - px(280. * zoom)).abs() < px(0.5));
                assert!((drawer_header.size.height - px(40. * zoom)).abs() < px(0.5));
                assert!(
                    drawer.left() > drawer_island.left() && drawer.right() < drawer_island.right()
                );
                assert!(drawer.bottom() < drawer_island.bottom());
                for fraction in [0.01, 0.5, 0.99] {
                    assert!(row_gutter.contains(&gpui::point(
                        row_gutter.center().x,
                        row_gutter.top() + row_gutter.size.height * fraction
                    )));
                }
                visual.update(|window, cx| {
                    workspace.update(cx, |workspace, cx| workspace.drawer_focus.focus(window, cx))
                });
                visual.run_until_parked();
                let focus_border = theme::with_alpha(
                    theme::text(),
                    if theme::active_variant().is_light() {
                        0.22
                    } else {
                        0.2
                    },
                );
                crate::theme_snapshot::assert_border(
                    &mut visual,
                    "MISSION_DRAWER_ISLAND",
                    focus_border,
                );
                crate::theme_snapshot::assert_border(
                    &mut visual,
                    "MISSION_ISLAND",
                    theme::border(),
                );
                visual.update(|window, cx| {
                    workspace.update(cx, |workspace, cx| workspace.root_focus.focus(window, cx))
                });
                visual.run_until_parked();
                crate::theme_snapshot::assert_border(&mut visual, "MISSION_ISLAND", focus_border);
                crate::theme_snapshot::assert_border(
                    &mut visual,
                    "MISSION_DRAWER_ISLAND",
                    theme::border(),
                );
                if collapsed {
                    assert!(
                        (area.left() - px(runner_app::ui::work_card::ISLAND_MARGIN * zoom)).abs()
                            < px(0.5)
                    );
                } else {
                    let sidebar = visual.debug_bounds("APP_SIDEBAR").unwrap();
                    let gutter = visual.debug_bounds("SIDEBAR_RESIZE_GUTTER").unwrap();
                    assert!(
                        (area.left()
                            - sidebar.right()
                            - px(runner_app::ui::work_card::ISLAND_GAP * zoom))
                        .abs()
                            < px(0.5)
                    );
                    assert!((gutter.center().x - area.left()).abs() <= px(0.5));
                    assert!((gutter.size.width - px(6. * zoom)).abs() < px(0.5));
                }
                if rail_open {
                    let rail = visual.debug_bounds("MISSION_RAIL").unwrap();
                    let gutter = visual.debug_bounds("MISSION_RAIL_GUTTER").unwrap();
                    assert!(
                        (rail.left()
                            - mission.right()
                            - px(runner_app::ui::work_card::ISLAND_GAP * zoom))
                        .abs()
                            < px(0.5)
                    );
                    assert_eq!(rail.right(), area.right());
                    assert_eq!((rail.top(), rail.bottom()), (area.top(), area.bottom()));
                    assert!((gutter.center().x - rail.left()).abs() < px(0.5));
                    assert!((gutter.size.width - px(6. * zoom)).abs() < px(0.5));
                } else {
                    assert_eq!(mission.right(), area.right());
                    assert!(visual.debug_bounds("MISSION_RAIL_GUTTER").is_none());
                }
            }
        }
        for (height, expected_rows) in if zoom == 1. {
            [(120., 4), (280., 11), (600., 29)]
        } else {
            [(120., 4), (280., 11), (600., 28)]
        } {
            workspace.update(&mut visual, |workspace, cx| {
                workspace.layout.drawer.set_height(height);
                cx.notify();
            });
            visual.run_until_parked();
            let island = visual.debug_bounds("MISSION_DRAWER_ISLAND").unwrap();
            assert!((island.size.height - px(height * zoom)).abs() < px(0.5));
            let rows = visual.update(|window, cx| {
                workspace
                    .read(cx)
                    .estimated_mission_drawer_terminal_size_for_height(height, window, cx)
                    .1
            });
            assert_eq!(rows, expected_rows, "drawer height={height}, zoom={zoom}");
        }
        workspace.update(&mut visual, |workspace, cx| {
            workspace.layout.drawer.set_height(280.);
            cx.notify();
        });
    }
    workspace.update(&mut visual, |workspace, cx| {
        workspace.layout.drawer.set_open(false);
        cx.notify();
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("MISSION_DRAWER_ISLAND").is_none());
    let area = visual.debug_bounds("APP_CONTENT_COLUMN").unwrap();
    assert_eq!(
        visual.debug_bounds("MISSION_ISLAND").unwrap().bottom(),
        area.bottom()
    );
    workspace.update(&mut visual, |workspace, cx| {
        workspace.layout.drawer.set_open(true);
        cx.notify();
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("MISSION_DRAWER_ISLAND").is_some());
    assert_eq!(
        workspace.read_with(&visual, |workspace, _| workspace.layout.drawer.height()),
        280.
    );
}

#[test]
fn remote_mission_uses_shared_sections_without_local_rows_or_private_metadata() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, store, fake) = setup();
    let workspace = workspace(window, &cx);
    workspace.read_with(&cx, |workspace, cx| {
        assert!(workspace.mission.is_none());
        assert!(workspace.sessions.is_empty());
        assert!(workspace.roster.is_empty());
        assert!(workspace.attached.is_empty());
        assert_eq!(workspace.mission_view().unwrap().title, "Host mission");
        assert_eq!(workspace.members()[0].session.id, "member");
        assert_eq!(workspace.mission_composer_roster()[0].handle, "coder");
        assert!(workspace.settings(cx).last_mission_terminal_ids.is_empty());
        assert_eq!(
            workspace.composer_input.read(cx).placeholder(),
            "Message everyone — @handle to address one"
        );
    });
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    for selector in [
        "MISSION_HEADER",
        "MISSION_TABS",
        "MISSION_RAIL",
        "MISSION_FEED",
        "MISSION_RAIL_ADD_ROLE",
        "MISSION_CARD_REMOVE member",
    ] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
    for selector in ["MISSION_CARD_KEY member", "MISSION_META_STARTED"] {
        assert!(visual.debug_bounds(selector).is_none(), "{selector}");
    }
    assert_eq!(fake.request_count("follow"), 1);
    assert_eq!(fake.request_count("feed"), 1);
    let local_calls = fake.local_call_count();
    window
        .update(&mut cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.select_mission_session("member", window, cx)
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.local_call_count(), local_calls);
    store.read_with(&cx, |store, _| {
        assert!(!store
            .settings
            .last_mission_terminal_ids
            .contains_key("remote-mission"))
    });
    let mut pushed = overview();
    pushed.missions[0].members[0].session = "replacement".into();
    let old = pushed.sessions.remove("member").unwrap();
    pushed.sessions.insert("replacement".into(), old);
    fake.set_overview(pushed);
    cx.run_until_parked();
    workspace.read_with(&cx, |workspace, _| {
        assert_eq!(
            workspace.active_tab,
            MissionTab::Session("replacement".into())
        );
        assert_eq!(workspace.open_tabs, ["replacement"]);
        assert_eq!(workspace.members()[0].handle, "coder");
    });
    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Tab("remote-tab".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(&cx, |workspace, _| {
        assert_eq!(
            workspace.active_tab,
            MissionTab::Session("replacement".into())
        )
    });
}

#[test]
fn selecting_a_crashed_remote_member_acknowledges_it_without_acknowledging_the_feed() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
    fake.set_features(&[
        "overview",
        "feed",
        "follow",
        "unfollow",
        "markViewed",
        "resume",
        "restart",
    ]);
    let mut data = overview();
    data.sessions.get_mut("member").unwrap().status.status = runner_core::companion::Status::Error;
    fake.set_overview(data);
    cx.run_until_parked();
    let window = remote_test::add_window(&mut cx, &store, temp.path(), "acknowledge-remote-member");
    window.update(&mut cx, remote_test::choose_host).unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();
    let workspace = workspace(window, &cx);
    workspace.read_with(&cx, |workspace, _| {
        assert_eq!(workspace.active_tab, MissionTab::Feed);
    });
    assert_eq!(fake.request_count("markViewed"), 0);

    window
        .update(&mut cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.select_mission_session("member", window, cx)
            })
        })
        .unwrap();
    cx.run_until_parked();
    let requests = remote_test::action_trace(&fake, "markViewed");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["session"], "member");

    fake.emit(
        "remote/session-status",
        serde_json::json!({"computer":"host","status":{"session":"member","status":"error","unread":false,"errorAcknowledged":true}}),
    );
    cx.run_until_parked();
    window
        .update(&mut cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.select_mission_feed(window, cx)
            })
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.select_mission_session("member", window, cx)
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.request_count("markViewed"), 1);
    assert_eq!(fake.attach_count(), 0);
    assert_eq!(fake.request_count("resume"), 0);
    assert_eq!(fake.request_count("restart"), 0);
}

#[test]
fn remote_feed_is_shared_pages_opaque_cursors_and_releases_only_after_last_window() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (temp, mut cx, first, store, fake) = setup();
    let ws1 = workspace(first, &cx);
    let feed = ws1.read_with(&cx, |workspace, _| {
        workspace
            .presentation
            .remote_for_test()
            .unwrap()
            .feed
            .clone()
            .unwrap()
    });
    fake.queue_result("feed", Ok(serde_json::json!({"entries":[entry("b",serde_json::json!({"kind":"message","text":"B"})),entry("a",serde_json::json!({"kind":"message","text":"A"}))],"before":"opaque-before"})));
    feed.update(&mut cx, |feed, cx| feed.refresh(cx));
    cx.run_until_parked();
    let follows = fake.request_count("follow");
    let second = remote_test::add_window(&mut cx, &store, temp.path(), "mission-second");
    second
        .update(&mut cx, |root, window, cx| {
            remote_test::choose_host(root, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    second
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();
    let ws2 = workspace(second, &cx);
    assert_eq!(
        ws2.read_with(&cx, |workspace, _| workspace
            .presentation
            .remote_for_test()
            .unwrap()
            .feed
            .clone()
            .unwrap()),
        feed
    );
    assert_eq!(fake.request_count("follow"), follows);
    let oversized = entry(
        "oversized",
        serde_json::json!({"kind":"signal","signal":"feedEntryTooLarge","payload":{"message":"This feed entry is too large to send. View it on the host."}}),
    );
    let unknown = entry(
        "unknown",
        serde_json::json!({"kind":"futureKind","content":"retained"}),
    );
    fake.emit("remote/feed", serde_json::json!({"computer":"other","feed":{"mission":"remote-mission","entries":[oversized.clone()]}}));
    fake.emit("remote/feed", serde_json::json!({"computer":"host","feed":{"mission":"remote-mission","entries":[oversized.clone(),oversized,unknown]}}));
    cx.run_until_parked();
    feed.read_with(&cx, |feed, _| {
        assert_eq!(
            feed.entries
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "oversized", "unknown"]
        )
    });
    fake.queue_result(
        "feed",
        Err(runner_core::protocol::ClientError::msg(
            "Older page refused",
        )),
    );
    feed.update(&mut cx, |feed, cx| feed.older(cx));
    cx.run_until_parked();
    feed.read_with(&cx, |feed, _| {
        assert!(feed.error.is_some());
        assert_eq!(feed.before.as_deref(), Some("opaque-before"));
    });
    fake.queue_result("feed", Ok(serde_json::json!({"entries":[entry("older",serde_json::json!({"kind":"message","text":"Older"}))],"before":null})));
    feed.update(&mut cx, |feed, cx| {
        feed.retry(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        remote_test::action_trace(&fake, "feed").last().unwrap()["before"],
        "opaque-before"
    );
    feed.read_with(&cx, |feed, _| {
        assert_eq!(feed.entries.first().unwrap().id, "older")
    });
    ws1.read_with(&cx, |workspace, _| {
        assert!(workspace.events.iter().any(|event| event.id == "oversized"));
        assert!(workspace.events.iter().any(|event| event.id == "unknown"));
    });
    drop(feed);
    first
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Tab("remote-tab".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.request_count("unfollow"), 0);
    second
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Tab("remote-tab".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.request_count("unfollow"), 1);
}

#[test]
fn remote_answers_use_question_ids_and_posts_preserve_failed_drafts_without_synthetic_events() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, _, fake) = setup();
    let workspace = workspace(window, &cx);
    workspace.update(&mut cx, |workspace, cx| {
        workspace.answer_mission_question("q2".into(), "yes".into(), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        remote_test::action_trace(&fake, "answer")[0]["question"],
        "q2"
    );
    workspace.read_with(&cx, |workspace, _| {
        assert!(workspace.resolved_asks.is_empty());
        assert!(workspace.events.is_empty());
        assert_eq!(workspace.pending_ask_choices.get("q2").unwrap(), "yes");
    });
    let mut pushed = overview();
    pushed.missions[0].asks.retain(|ask| ask.question != "q2");
    fake.set_overview(pushed);
    cx.run_until_parked();
    workspace.update(&mut cx, |workspace, cx| {
        workspace.answer_mission_question("q2".into(), "no".into(), cx)
    });
    cx.run_until_parked();
    assert_eq!(fake.request_count("answer"), 1);
    fake.emit("remote/feed",serde_json::json!({"computer":"host","feed":{"mission":"remote-mission","entries":[entry("response",serde_json::json!({"kind":"answer","question":"q2","text":"yes"}))]}}));
    cx.run_until_parked();
    workspace.read_with(&cx, |workspace, _| {
        assert_eq!(workspace.resolved_asks.get("q2").unwrap(), "yes")
    });
    fake.queue_result(
        "post",
        Err(runner_core::protocol::ClientError::msg("Post refused")),
    );
    window
        .update(&mut cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace
                    .composer_input
                    .update(cx, |input, cx| input.reset("Keep draft", cx));
                workspace.post_mission_composer(
                    ComposerPost {
                        text: "@coder Keep draft".into(),
                        to: Default::default(),
                    },
                    window,
                    cx,
                );
            })
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(&cx, |workspace, cx| {
        assert_eq!(workspace.composer_input.read(cx).text(), "Keep draft");
        assert!(workspace.error.is_some());
        assert_eq!(workspace.events.len(), 1);
    });
    assert_eq!(
        remote_test::action_trace(&fake, "post")[0]["to"],
        serde_json::json!(["coder"])
    );
    window
        .update(&mut cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.post_mission_composer(
                    ComposerPost {
                        text: "Keep draft".into(),
                        to: runner_core::Recipients::new(vec!["coder".into()]),
                    },
                    window,
                    cx,
                )
            })
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(&cx, |workspace, cx| {
        assert_eq!(workspace.composer_input.read(cx).text(), "");
        assert_eq!(workspace.events.len(), 1);
    });
}

#[test]
fn lifecycle_shortcuts_ignore_a_focused_mission_drawer_shell() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for status in [
        runner_core::companion::Status::Working,
        runner_core::companion::Status::Stopped,
    ] {
        let (_temp, mut cx, window, _, fake) = setup();
        cx.update(|cx| {
            crate::keymap::install_bindings(
                cx,
                &crate::keymap::KeymapOverrides::new(),
                crate::keymap::TerminalCtrlKeys::RunnerFirst,
                false,
            )
        });
        let workspace = workspace(window, &cx);
        let mut data = overview();
        data.sessions.get_mut("member").unwrap().status.status = status;
        let mut shell = data.sessions["member"].clone();
        shell.kind = runner_core::companion::SessionKind::Shell;
        shell.status.status = runner_core::companion::Status::Working;
        shell.runtime = None;
        data.sessions.insert("shell".into(), shell);
        data.missions[0].shells = vec!["shell".into()];
        fake.set_overview(data);
        cx.run_until_parked();
        window
            .update(&mut cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    workspace.select_mission_session("member", window, cx);
                    workspace.layout.drawer.set_open(true);
                    cx.notify();
                })
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.run_until_parked();
        window
            .update(&mut visual, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    workspace.drawer_focus.focus(window, cx);
                    assert!(workspace.drawer_focused(window, cx));
                    assert_eq!(workspace.active_tab, MissionTab::Session("member".into()));
                })
            })
            .unwrap();
        visual.simulate_keystrokes(&crate::keymap::platform_default("shift-cmd-x"));
        visual.simulate_keystrokes(&crate::keymap::platform_default("shift-cmd-r"));
        visual.run_until_parked();
        assert_eq!(fake.request_count("stop"), 0);
        assert_eq!(fake.request_count("resume"), 0);
    }
}

#[test]
fn remote_mission_shell_retry_keeps_successful_start_and_waits_for_drawer_push() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, _, fake) = setup();
    let workspace = workspace(window, &cx);
    fake.queue_result(
        "layout",
        Err(runner_core::protocol::ClientError::msg("Placement refused")),
    );
    window
        .update(&mut cx, |root, window, cx| root.new_terminal(window, cx))
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(&cx, |workspace, _| {
        assert!(workspace.layout.drawer.shells().is_empty());
        assert!(workspace.error.is_some());
        assert_eq!(
            workspace
                .presentation
                .remote_for_test()
                .unwrap()
                .shell_started
                .lock()
                .unwrap()
                .as_deref(),
            Some("started-session")
        );
    });
    let start = &remote_test::action_trace(&fake, "startShell")[0];
    assert_eq!(start["cwd"], "C:\\host\\work");
    assert!(start["project"].is_null());
    assert!(start["cols"].as_u64().unwrap() >= 20);
    assert!(start["rows"].as_u64().unwrap() >= 8);
    window
        .update(&mut cx, |root, window, cx| root.new_terminal(window, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.request_count("startShell"), 1);
    assert_eq!(fake.request_count("layout"), 2);
    let layout = &remote_test::action_trace(&fake, "layout")[1];
    assert_eq!(layout["target"]["kind"], "mission");
    assert_eq!(layout["shells"], serde_json::json!(["started-session"]));
    assert!(layout["panes"]["session"].is_null());
    workspace.read_with(&cx, |workspace, _| {
        assert!(workspace.layout.drawer.shells().is_empty())
    });
    let mut pushed = overview();
    pushed.missions[0].shells.push("started-session".into());
    let mut shell = pushed.sessions["member"].clone();
    shell.kind = runner_core::companion::SessionKind::Shell;
    pushed.sessions.insert("started-session".into(), shell);
    fake.set_overview(pushed);
    cx.run_until_parked();
    workspace.read_with(&cx, |workspace, _| {
        assert_eq!(workspace.layout.drawer.shells(), &["started-session"]);
        assert!(workspace.layout.drawer.open());
        assert!(workspace
            .presentation
            .remote_for_test()
            .unwrap()
            .drawer_expected
            .is_none());
    });
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.request_close_terminal_drawer_shell("started-session", window, cx)
        })
    });
    window
        .update(&mut visual, |root, _, _| {
            assert!(root.panes.close_confirm.is_none());
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(fake.request_count("close"), 1);
    workspace.read_with(&visual, |workspace, _| {
        assert_eq!(workspace.layout.drawer.shells(), &["started-session"]);
        assert!(workspace
            .presentation
            .remote_for_test()
            .unwrap()
            .drawer_expected
            .is_some());
    });
    fake.set_overview(overview());
    visual.run_until_parked();
    workspace.read_with(&visual, |workspace, _| {
        assert!(workspace.layout.drawer.shells().is_empty());
        assert!(!workspace.layout.drawer.open());
        assert!(workspace
            .presentation
            .remote_for_test()
            .unwrap()
            .drawer_expected
            .is_none());
    });
}

#[test]
fn mission_lifecycle_waits_for_pushes_and_navigation_discards_late_errors() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for push_before_result in [true, false] {
        let (_temp, mut cx, window, _, fake) = setup();
        let workspace = workspace(window, &cx);
        let mut stopped = overview();
        stopped.sessions.get_mut("member").unwrap().status.status =
            runner_core::companion::Status::Stopped;
        let pushed = stopped.clone();
        window
            .update(&mut cx, |root, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    workspace.open_stop_all_confirm(window, cx);
                    assert!(workspace.stop_all_confirm);
                    workspace.stop_all_confirm = false;
                    workspace.stop_open_mission(window, cx);
                    assert_eq!(
                        workspace.members()[0].session.status,
                        SessionStatus::Running
                    );
                });
                if push_before_result {
                    let clients = root.app_store.read(cx).remote_clients.clone();
                    clients.update(cx, |clients, cx| {
                        clients.overviews.insert("host".into(), pushed);
                        cx.notify();
                    });
                    root.sync_remote_state(cx);
                }
            })
            .unwrap();
        cx.run_until_parked();
        if !push_before_result {
            workspace.read_with(&cx, |workspace, _| {
                assert_eq!(
                    workspace.members()[0].session.status,
                    SessionStatus::Running
                )
            });
            fake.set_overview(stopped);
            cx.run_until_parked();
        }
        workspace.read_with(&cx, |workspace, _| {
            assert_eq!(
                workspace.members()[0].session.status,
                SessionStatus::Stopped
            )
        });
        window
            .update(&mut cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    workspace.resume_open_mission(window, cx)
                })
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(fake.request_count("resumeMission"), 1);
        workspace.read_with(&cx, |workspace, _| {
            assert_eq!(
                workspace.members()[0].session.status,
                SessionStatus::Stopped
            )
        });
        fake.set_overview(overview());
        cx.run_until_parked();
        fake.queue_result(
            "stop",
            Err(runner_core::protocol::ClientError::msg(
                "Old target failure",
            )),
        );
        window
            .update(&mut cx, |root, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    workspace.act_on_slot("member", SessionControlKind::Stop, window, cx)
                });
                root.select_remote_item(RemoteItem::Tab("remote-tab".into()), cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(fake.request_count("stop"), 1);
        workspace.read_with(&cx, |workspace, _| {
            assert!(!workspace
                .error
                .as_ref()
                .is_some_and(|error| error.resolve().contains("Old target failure")))
        });
    }
}

#[test]
fn reconnect_refreshes_shared_feed_and_preserves_older_history_without_replaying_posts() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, _, fake) = setup();
    let workspace = workspace(window, &cx);
    let feed = workspace.read_with(&cx, |workspace, _| {
        workspace
            .presentation
            .remote_for_test()
            .unwrap()
            .feed
            .clone()
            .unwrap()
    });
    fake.queue_result("feed", Ok(serde_json::json!({"entries":[entry("older",serde_json::json!({"kind":"message","text":"Older"}))],"before":"old-cursor"})));
    feed.update(&mut cx, |feed, cx| feed.refresh(cx));
    cx.run_until_parked();
    let before = fake.request_count("follow");
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"disconnected","error":"closed"}),
    );
    cx.run_until_parked();
    fake.queue_result("feed", Ok(serde_json::json!({"entries":[entry("new",serde_json::json!({"kind":"message","text":"New"}))],"before":"new-cursor"})));
    fake.queue_result("feed", Ok(serde_json::json!({"entries":[entry("gap",serde_json::json!({"kind":"message","text":"While disconnected"})),entry("older",serde_json::json!({"kind":"message","text":"Older"}))],"before":"overlap-cursor"})));
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"connected"}),
    );
    cx.run_until_parked();
    feed.read_with(&cx, |feed, _| {
        assert_eq!(
            feed.entries
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["older", "gap", "new"]
        );
        assert_eq!(feed.before.as_deref(), Some("old-cursor"));
    });
    assert_eq!(
        remote_test::action_trace(&fake, "feed").last().unwrap()["before"],
        "new-cursor"
    );
    assert_eq!(fake.request_count("follow"), before + 1);
    assert_eq!(fake.request_count("post"), 0);
    assert_eq!(fake.request_count("answer"), 0);
    window
        .update(&mut cx, |root, window, cx| {
            root.return_to_this_computer(window, cx)
        })
        .unwrap();
    drop(feed);
    cx.run_until_parked();
    workspace.read_with(&cx, |workspace, _| {
        assert!(!workspace.active);
        assert!(workspace
            .presentation
            .remote_for_test()
            .unwrap()
            .feed
            .is_none());
    });
}

#[test]
fn absent_mission_capabilities_hide_controls_and_refuse_dispatch_at_the_boundary() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, store, fake) = setup();
    let workspace = workspace(window, &cx);
    let target = workspace.read_with(&cx, |workspace, _| workspace.client.target.clone());
    let access = crate::surfaces::front_page::RemoteAccess::new(target);
    access.set_features(vec!["feed".into(), "follow".into(), "unfollow".into()]);
    let client = store.read_with(&cx, |store, _| store.client.clone());
    workspace.update(&mut cx, |workspace, cx| {
        workspace.client = crate::surfaces::front_page::FrontPageClient::remote(client, access);
        cx.notify();
    });
    window
        .update(&mut cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.request_slot_action("member", SessionControlKind::Restart, window, cx);
                assert!(workspace.restart_confirm.is_none());
                workspace.open_stop_all_confirm(window, cx);
                assert!(!workspace.stop_all_confirm);
                workspace.resume_open_mission(window, cx);
                workspace.archive_open_mission(window, cx);
                workspace.add_terminal_drawer_shell(window, cx);
                workspace.answer_mission_question("q1".into(), "yes".into(), cx);
                workspace.post_mission_composer(
                    ComposerPost {
                        text: "Message".into(),
                        to: Default::default(),
                    },
                    window,
                    cx,
                );
            })
        })
        .unwrap();
    cx.run_until_parked();
    for kind in [
        "restart",
        "stopMission",
        "resumeMission",
        "archive",
        "startShell",
        "layout",
        "answer",
        "post",
    ] {
        assert_eq!(fake.request_count(kind), 0, "{kind}");
    }
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    assert!(visual
        .debug_bounds("MISSION_CARD_CONTROLS member")
        .is_none());
}

#[test]
fn stopped_host_missions_keep_the_feed_read_only_and_hide_slot_controls() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, _, fake) = setup();
    let workspace = workspace(window, &cx);
    let mut stopped = overview();
    stopped.missions[0].state = runner_core::companion::MissionState::Stopped;
    stopped.sessions.get_mut("member").unwrap().status.status =
        runner_core::companion::Status::Stopped;
    fake.set_overview(stopped);
    cx.run_until_parked();
    window
        .update(&mut cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.act_on_slot("member", SessionControlKind::Resume, window, cx)
            })
        })
        .unwrap();
    window
        .update(&mut cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.stop_open_mission(window, cx);
                workspace.resume_open_mission(window, cx);
                workspace.post_mission_composer(
                    ComposerPost {
                        text: "Stale composer".into(),
                        to: Default::default(),
                    },
                    window,
                    cx,
                );
            })
        })
        .unwrap();
    cx.run_until_parked();
    for kind in ["stopMission", "resumeMission", "post"] {
        assert_eq!(fake.request_count(kind), 0, "{kind}");
    }
    assert_eq!(fake.request_count("resume"), 0);
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    assert!(visual
        .debug_bounds("MISSION_CARD_CONTROLS member")
        .is_none());
    assert!(visual.debug_bounds("MISSION_FEED").is_some());
}

#[test]
fn pushed_roster_and_shell_order_preserve_client_tab_order_focus_and_drawer_presentation() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, cx, window, _, fake) = setup();
    let workspace = workspace(window, &cx);
    let mut pushed = overview();
    pushed.missions[0]
        .members
        .push(runner_core::companion::Member {
            handle: "second".into(),
            session: "second-id".into(),
            lead: false,
        });
    let session = pushed.sessions["member"].clone();
    pushed.sessions.insert("second-id".into(), session.clone());
    for id in ["shell-a", "shell-b"] {
        let mut shell = session.clone();
        shell.kind = runner_core::companion::SessionKind::Shell;
        pushed.sessions.insert(id.into(), shell);
    }
    pushed.missions[0].shells = vec!["shell-a".into(), "shell-b".into()];
    fake.set_overview(pushed.clone());
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.close_mission_session_tab("member", window, cx);
            workspace.select_mission_session("member", window, cx);
            workspace.toggle_terminal_drawer(window, cx);
            workspace.activate_terminal_drawer_shell("shell-a", window, cx);
            workspace.layout.drawer.set_height(219.);
        })
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |workspace, _| {
        assert_eq!(workspace.open_tabs, ["second-id", "member"])
    });
    pushed.missions[0].shells = vec!["shell-b".into(), "shell-a".into()];
    fake.set_overview(pushed);
    visual.run_until_parked();
    workspace.read_with(&visual, |workspace, _| {
        assert_eq!(workspace.open_tabs, ["second-id", "member"]);
        assert_eq!(workspace.layout.drawer.shells(), &["shell-b", "shell-a"]);
        assert_eq!(workspace.layout.drawer.active_shell(), Some("shell-a"));
        assert!(workspace.layout.drawer.open());
        assert_eq!(workspace.layout.drawer.height(), 219.);
    });
    assert_eq!(fake.request_count("layout"), 0);
    window
        .update(&mut visual, |root, _, cx| {
            root.select_remote_item(RemoteItem::Tab("remote-tab".into()), cx)
        })
        .unwrap();
    visual.run_until_parked();
    window
        .update(&mut visual, |root, _, cx| {
            root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
        })
        .unwrap();
    visual.run_until_parked();
    workspace.read_with(&visual, |workspace, _| {
        assert_eq!(workspace.open_tabs, ["second-id", "member"])
    });
}

#[test]
fn deferred_remote_focus_does_not_steal_rename_focus_and_global_zoom_keeps_target_geometry() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, cx, window, store, _fake) = setup();
    let workspace = workspace(window, &cx);
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.select_mission_session("member", window, cx)
        })
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.focus_active_mission_terminal(window, cx);
            workspace.configure_mission_action_menu(cx);
            workspace.handle_mission_menu_action(1, window, cx);
        })
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        let workspace = workspace.read(cx);
        assert!(workspace
            .rename_modal
            .as_ref()
            .unwrap()
            .input
            .read(cx)
            .focus_handle()
            .is_focused(window));
    });
    let local_width = store.read_with(&visual, |store, _| store.settings.mission_rail_width);
    visual.update(|_, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.set_remote_sidebar_width(341., cx);
            workspace.set_rail_open(false, cx);
            workspace.update_app_settings(cx, false, |settings| {
                settings.mission_rail_width = 333.;
                true
            });
        })
    });
    store.update(&mut visual, |store, cx| {
        store.update_settings(
            |settings| {
                settings.app_zoom = 1.2;
                true
            },
            false,
            cx,
        );
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |workspace, cx| {
        assert_eq!(workspace.settings(cx).app_zoom, 1.2);
        assert_eq!(workspace.settings(cx).sidebar_width, 341.);
        assert_eq!(workspace.settings(cx).mission_rail_width, 333.);
        assert!(!workspace.rail_is_open(cx));
    });
    store.read_with(&visual, |store, _| {
        assert_eq!(store.settings.mission_rail_width, local_width)
    });
}

#[test]
fn pending_mission_rename_is_dismissed_on_connection_generation_change_without_replay() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, store, fake) = setup();
    let workspace = workspace(window, &cx);
    window
        .update(&mut cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.configure_mission_action_menu(cx);
                workspace.handle_mission_menu_action(1, window, cx);
                workspace
                    .rename_modal
                    .as_ref()
                    .unwrap()
                    .input
                    .update(cx, |input, cx| input.reset("Renamed host mission", cx));
                workspace.submit_mission_rename(window, cx);
            });
        })
        .unwrap();
    for _ in 0..1000 {
        if fake.request_count("rename") == 1 {
            break;
        }
        assert!(cx.executor().tick());
    }
    assert_eq!(fake.request_count("rename"), 1);
    let generation = workspace.read_with(&cx, |workspace, _| {
        assert!(workspace.rename_modal.as_ref().unwrap().submitting);
        workspace.generation
    });
    window
        .update(&mut cx, |root, _, cx| {
            let clients = store.read(cx).remote_clients.clone();
            clients.update(cx, |clients, cx| {
                clients.connections.insert(
                    "host".into(),
                    crate::surfaces::remote_client::model::Connection::Reconnecting,
                );
                clients.invalidate_capabilities("host");
                cx.notify();
            });
            root.sync_remote_state(cx);
        })
        .unwrap();
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"connected"}),
    );
    cx.run_until_parked();
    workspace.update(&mut cx, |workspace, cx| {
        assert!(workspace.generation > generation);
        assert!(workspace.rename_modal.is_none());
        assert!(workspace.render_mission_overlays(cx).is_empty());
        assert!(workspace.error.is_none());
    });
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.update(|window, cx| assert!(workspace.read(cx).root_focus.is_focused(window)));
    assert_eq!(fake.request_count("rename"), 1);
}

#[test]
fn mission_dialogs_do_not_survive_navigation_or_host_removal() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for removed in [false, true] {
        for dialog in ["rename", "stop", "restart"] {
            let (_temp, mut cx, window, store, fake) = setup();
            let workspace = workspace(window, &cx);
            window
                .update(&mut cx, |_, window, cx| {
                    workspace.update(cx, |workspace, cx| {
                        match dialog {
                            "rename" => {
                                workspace.configure_mission_action_menu(cx);
                                workspace.handle_mission_menu_action(1, window, cx);
                            }
                            "stop" => workspace.open_stop_all_confirm(window, cx),
                            _ => workspace.request_slot_action(
                                "member",
                                SessionControlKind::Restart,
                                window,
                                cx,
                            ),
                        }
                        assert!(
                            workspace.rename_modal.is_some()
                                || workspace.stop_all_confirm
                                || workspace.restart_confirm.is_some()
                        );
                    });
                })
                .unwrap();
            cx.run_until_parked();
            window
                .update(&mut cx, |root, _, cx| {
                    if removed {
                        let mut pushed = overview();
                        pushed.missions.clear();
                        store
                            .read(cx)
                            .remote_clients
                            .clone()
                            .update(cx, |clients, cx| {
                                clients.overviews.insert("host".into(), pushed);
                                cx.notify();
                            });
                        root.sync_remote_state(cx);
                    } else {
                        root.select_remote_item(RemoteItem::Tab("remote-tab".into()), cx);
                    }
                })
                .unwrap();
            cx.run_until_parked();
            workspace.update(&mut cx, |workspace, cx| {
                assert!(!workspace.active);
                assert!(workspace.rename_modal.is_none());
                assert!(!workspace.stop_all_confirm);
                assert!(workspace.restart_confirm.is_none());
                assert!(workspace.render_mission_overlays(cx).is_empty());
            });
            window
                .update(&mut cx, |root, window, cx| {
                    assert!(root.launchers.any_search_focused(window, cx))
                })
                .unwrap();
            for kind in ["rename", "stopMission", "restart"] {
                assert_eq!(fake.request_count(kind), 0);
            }
        }
    }
}

#[test]
fn failed_feed_catch_up_retries_to_the_contiguous_boundary_despite_tail_appends() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for append_during_refresh in [true, false] {
        let (_temp, mut cx, window, _, fake) = setup();
        let workspace = workspace(window, &cx);
        let feed = workspace.read_with(&cx, |workspace, _| {
            workspace
                .presentation
                .remote_for_test()
                .unwrap()
                .feed
                .clone()
                .unwrap()
        });
        let message = |id: &str| entry(id, serde_json::json!({"kind":"message","text":id}));
        fake.queue_result(
            "feed",
            Ok(serde_json::json!({"entries":[message("a")],"before":null})),
        );
        feed.update(&mut cx, |feed, cx| feed.refresh(cx));
        cx.run_until_parked();
        fake.queue_result(
            "feed",
            Ok(serde_json::json!({"entries":[message("c")],"before":"failed-gap"})),
        );
        fake.queue_result(
            "feed",
            Err(runner_core::protocol::ClientError::msg(
                "Intermediate page failed",
            )),
        );
        feed.update(&mut cx, |feed, cx| feed.refresh(cx));
        if !append_during_refresh {
            cx.run_until_parked();
        }
        fake.emit("remote/feed", serde_json::json!({"computer":"host","feed":{"mission":"remote-mission","entries":[message("d")]}}));
        cx.run_until_parked();
        feed.read_with(&cx, |feed, _| {
            assert!(feed.error.is_some());
            assert_eq!(
                feed.entries
                    .iter()
                    .map(|entry| entry.id.as_str())
                    .collect::<Vec<_>>(),
                ["a", "d"]
            );
        });
        fake.queue_result(
            "feed",
            Ok(serde_json::json!({"entries":[message("d"),message("c")],"before":"retry-gap"})),
        );
        fake.queue_result(
            "feed",
            Ok(serde_json::json!({"entries":[message("b"),message("a")],"before":null})),
        );
        feed.update(&mut cx, |feed, cx| feed.retry(cx));
        cx.run_until_parked();
        feed.read_with(&cx, |feed, _| {
            assert!(feed.error.is_none());
            assert_eq!(
                feed.entries
                    .iter()
                    .map(|entry| entry.id.as_str())
                    .collect::<Vec<_>>(),
                ["a", "b", "c", "d"]
            );
            assert!(feed.before.is_none());
        });
        assert_eq!(
            remote_test::action_trace(&fake, "feed").last().unwrap()["before"],
            "retry-gap"
        );
        assert_eq!(fake.request_count("post"), 0);
        assert_eq!(fake.request_count("answer"), 0);
    }
}

#[test]
fn older_pagination_cannot_clear_a_failed_catch_up_and_recovers_after_the_gap_is_bridged() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for older_fails in [false, true] {
        let (_temp, mut cx, window, _, fake) = setup();
        let workspace = workspace(window, &cx);
        let feed = workspace.read_with(&cx, |workspace, _| {
            workspace
                .presentation
                .remote_for_test()
                .unwrap()
                .feed
                .clone()
                .unwrap()
        });
        let message = |id: &str| entry(id, serde_json::json!({"kind":"message","text":id}));
        fake.queue_result(
            "feed",
            Ok(serde_json::json!({"entries":[message("a")],"before":"old-cursor"})),
        );
        feed.update(&mut cx, |feed, cx| feed.refresh(cx));
        cx.run_until_parked();
        fake.queue_result(
            "feed",
            Ok(serde_json::json!({"entries":[message("c")],"before":"failed-gap"})),
        );
        fake.queue_result(
            "feed",
            Err(runner_core::protocol::ClientError::msg(
                "Catch-up page failed",
            )),
        );
        feed.update(&mut cx, |feed, cx| feed.refresh(cx));
        fake.emit("remote/feed", serde_json::json!({"computer":"host","feed":{"mission":"remote-mission","entries":[message("d")]}}));
        cx.run_until_parked();
        let requests = fake.request_count("feed");
        let error = feed.read_with(&cx, |feed, _| {
            assert!(!feed.can_load_older());
            feed.error.clone().unwrap()
        });
        feed.update(&mut cx, |feed, cx| feed.older(cx));
        cx.run_until_parked();
        assert_eq!(fake.request_count("feed"), requests);
        feed.read_with(&cx, |feed, _| {
            assert_eq!(feed.error.as_deref(), Some(error.as_str()));
            assert_eq!(feed.before.as_deref(), Some("old-cursor"));
        });
        fake.queue_result(
            "feed",
            Ok(serde_json::json!({"entries":[message("d"),message("c")],"before":"retry-gap"})),
        );
        fake.queue_result("feed", Ok(serde_json::json!({"entries":[message("b"),message("a")],"before":"overlap-cursor"})));
        fake.queue_result(
            "feed",
            if older_fails {
                Err(runner_core::protocol::ClientError::msg("Older page failed"))
            } else {
                Ok(serde_json::json!({"entries":[message("z")],"before":null}))
            },
        );
        feed.update(&mut cx, |feed, cx| feed.retry(cx));
        cx.run_until_parked();
        assert_eq!(fake.request_count("feed"), requests + 2);
        assert_eq!(
            remote_test::action_trace(&fake, "feed").last().unwrap()["before"],
            "retry-gap"
        );
        feed.read_with(&cx, |feed, _| {
            assert!(feed.error.is_none());
            assert!(feed.can_load_older());
            assert_eq!(feed.before.as_deref(), Some("old-cursor"));
            assert_eq!(
                feed.entries
                    .iter()
                    .map(|entry| entry.id.as_str())
                    .collect::<Vec<_>>(),
                ["a", "b", "c", "d"]
            );
        });
        feed.update(&mut cx, |feed, cx| feed.older(cx));
        cx.run_until_parked();
        assert_eq!(
            remote_test::action_trace(&fake, "feed").last().unwrap()["before"],
            "old-cursor"
        );
        if older_fails {
            feed.read_with(&cx, |feed, _| {
                assert!(feed.error.is_some());
                assert_eq!(feed.before.as_deref(), Some("old-cursor"));
            });
            fake.queue_result(
                "feed",
                Ok(serde_json::json!({"entries":[message("z")],"before":null})),
            );
            feed.update(&mut cx, |feed, cx| feed.retry(cx));
            cx.run_until_parked();
            assert_eq!(
                remote_test::action_trace(&fake, "feed").last().unwrap()["before"],
                "old-cursor"
            );
        }
        feed.read_with(&cx, |feed, _| {
            assert!(feed.error.is_none());
            assert!(!feed.can_load_older());
            assert!(feed.before.is_none());
            assert_eq!(
                feed.entries
                    .iter()
                    .map(|entry| entry.id.as_str())
                    .collect::<Vec<_>>(),
                ["z", "a", "b", "c", "d"]
            );
        });
        assert_eq!(fake.request_count("post"), 0);
        assert_eq!(fake.request_count("answer"), 0);
    }
}

#[test]
fn existing_mission_shells_have_no_exited_lifecycle_actions() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for features in [
        vec![],
        vec!["resume"],
        vec!["close"],
        vec!["resume", "close"],
    ] {
        let (_temp, mut cx, window, store, fake) = setup();
        let workspace = workspace(window, &cx);
        fake.set_features(&features);
        window
            .update(&mut cx, |root, _, cx| {
                store
                    .read(cx)
                    .remote_clients
                    .clone()
                    .update(cx, |clients, cx| {
                        clients.invalidate_capabilities("host");
                        cx.notify();
                    });
                root.sync_remote_state(cx);
            })
            .unwrap();
        cx.run_until_parked();
        let mut pushed = overview();
        let mut shell = pushed.sessions["member"].clone();
        shell.kind = runner_core::companion::SessionKind::Shell;
        shell.status.status = runner_core::companion::Status::Stopped;
        pushed.sessions.insert("shell".into(), shell);
        pushed.missions[0].shells = vec!["shell".into()];
        fake.set_overview(pushed);
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        let header = visual.debug_bounds("MISSION_HEADER_ROW").unwrap();
        visual.simulate_click(
            gpui::point(header.right() - px(20.), header.center().y),
            Default::default(),
        );
        visual.run_until_parked();
        workspace.read_with(&visual, |workspace, _| {
            assert!(workspace.layout.drawer.open())
        });
        assert!(visual.debug_bounds("SESSION_ENDED_ACTIONS").is_none());
        for action in ["resume", "close", "startShell", "layout"] {
            assert_eq!(fake.request_count(action), 0);
        }
    }
}

#[test]
fn remote_mission_panes_use_the_host_palette() {
    let (_temp, cx, window, _, fake) = setup();
    let mut data = overview();
    data.terminal_palette = Some(
        runner_terminal::terminal::palette_to_wire(runner_terminal::palette::ROSE_PINE_DAWN).into(),
    );
    fake.set_overview(data);
    cx.run_until_parked();
    let palette =
        workspace(window, &cx).read_with(&cx, |workspace, cx| workspace.terminal_style(cx).palette);
    assert_eq!(palette, runner_terminal::palette::ROSE_PINE_DAWN);
}

#[test]
fn the_remote_header_reads_and_toggles_a_linked_missions_group_pin() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, _store, fake) = setup();
    let mut data = overview();
    data.tabs[0].panes = runner_core::companion::Pane::Leaf {
        session: Some("chat".into()),
    };
    data.tabs[0].pinned = true;
    data.missions[0].linked_chat = Some(runner_core::companion::LinkedChat {
        session_id: "chat".into(),
        title: "Chat".into(),
        archived: false,
        reports: true,
    });
    let chat = data.sessions["member"].clone();
    data.sessions.insert("chat".into(), chat);
    fake.set_overview(data.clone());
    cx.run_until_parked();
    let workspace = workspace(window, &cx);
    let pinned = |cx: &mut TestAppContext| {
        workspace.read_with(cx, |workspace, cx| {
            workspace.mission_pinned(&workspace.mission_view().unwrap(), cx)
        })
    };
    assert!(pinned(&mut cx), "the group's tab is pinned");

    window
        .update(&mut cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.toggle_pin_for_test(window, cx)
            })
        })
        .unwrap();
    for _ in 0..1000 {
        if fake.request_count("pin") == 1 {
            break;
        }
        assert!(cx.executor().tick());
    }
    let pins = remote_test::action_trace(&fake, "pin");
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0]["pinned"], false, "the header offers Unpin");

    data.tabs[0].pinned = false;
    fake.set_overview(data);
    cx.run_until_parked();
    assert!(!pinned(&mut cx), "the header follows the host's group pin");
}

#[test]
fn clicking_the_remote_linked_chat_opens_its_tab() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, cx, window, _store, fake) = setup();
    let mut data = overview();
    data.tabs[0].panes = runner_core::companion::Pane::Leaf {
        session: Some("chat".into()),
    };
    data.missions[0].linked_chat = Some(runner_core::companion::LinkedChat {
        session_id: "chat".into(),
        title: "Chat".into(),
        archived: false,
        reports: true,
    });
    let chat = data.sessions["member"].clone();
    data.sessions.insert("chat".into(), chat);
    fake.set_overview(data);
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    let chip = visual
        .debug_bounds("MISSION_LINKED_CHAT")
        .expect("the header shows the linked chat");
    visual.simulate_click(chip.center(), Default::default());
    visual.run_until_parked();

    window
        .read_with(&visual, |root, _| {
            assert!(
                root.remote_client.active_item() == Some(&RemoteItem::Tab("remote-tab".into())),
                "the linked chat's tab is selected"
            );
        })
        .unwrap();
}

#[test]
fn remote_zero_role_mission_does_not_flash_no_roles_or_no_events_before_feed_loads() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for capable in [true, false] {
        for with_events in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
            remote_test::enable_missions(&fake);
            if !capable {
                fake.without_features(&["addMissionRole"]);
            }
            let mut data = overview();
            data.missions[0].members.clear();
            data.missions[0].asks.clear();
            data.sessions.clear();
            fake.set_overview(data.clone());
            let window =
                remote_test::add_window(&mut cx, &store, temp.path(), "remote-empty-roles");
            window
                .update(&mut cx, |root, window, cx| {
                    remote_test::choose_host(root, window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            if with_events {
                fake.queue_result(
                    "feed",
                    Ok(serde_json::json!({
                        "entries": [entry("e1", serde_json::json!({"kind":"message","text":"Mission started"}))],
                        "before": null
                    })),
                );
            }
            window
                .update(&mut cx, |root, _, cx| {
                    root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
                })
                .unwrap();
            let workspace = workspace(window, &cx);
            let feed = workspace.read_with(&cx, |workspace, _| {
                assert!(!workspace.loading);
                assert_eq!(workspace.feed_item_count(), 1);
                assert_eq!(workspace.feed_empty_panel_min_height.get(), px(0.));
                workspace
                    .presentation
                    .remote_for_test()
                    .unwrap()
                    .feed
                    .clone()
                    .unwrap()
            });
            feed.read_with(&cx, |feed, _| {
                assert!(feed.loading);
                assert!(!feed.history_loaded);
            });
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            visual.update(|window, _| window.refresh());
            feed.read_with(&visual, |feed, _| {
                assert!(feed.loading);
                assert!(!feed.history_loaded);
            });
            assert!(visual.debug_bounds("MISSION_FEED").is_some());
            assert!(visual.debug_bounds("MISSION_NO_ROLES").is_none());
            assert!(visual.debug_bounds("MISSION_NO_EVENTS").is_none());
            assert!(visual.debug_bounds("MISSION_RAIL_NO_ROLES").is_some());
            assert_eq!(
                visual.debug_bounds("MISSION_RAIL_ADD_ROLE").is_some(),
                capable
            );
            workspace.read_with(&visual, |workspace, _| {
                assert_eq!(workspace.feed_item_count(), 1);
                assert_eq!(workspace.feed_empty_panel_min_height.get(), px(0.));
            });

            visual.run_until_parked();
            visual.update(|window, _| window.refresh());
            visual.run_until_parked();
            feed.read_with(&visual, |feed, _| {
                assert!(!feed.loading);
                assert!(feed.history_loaded);
            });
            assert_eq!(visual.debug_bounds("MISSION_NO_ROLES").is_some(), capable);
            assert!(visual.debug_bounds("MISSION_RAIL_NO_ROLES").is_some());
            assert_eq!(
                visual.debug_bounds("MISSION_RAIL_ADD_ROLE").is_some(),
                capable
            );
            let initial_no_roles_bounds = visual.debug_bounds("MISSION_NO_ROLES");
            let initial_min_height = workspace.read_with(&visual, |workspace, _| {
                assert_eq!(workspace.feed_item_count(), if capable { 2 } else { 1 });
                if !capable {
                    assert_eq!(workspace.feed_empty_panel_min_height.get(), px(0.));
                }
                workspace.feed_empty_panel_min_height.get()
            });
            let initial_no_events_bounds = if with_events {
                assert!(visual.debug_bounds("MISSION_NO_EVENTS").is_none());
                assert!(visual.debug_bounds("MISSION_MESSAGE_GROUP e1").is_some());
                None
            } else {
                let bounds = visual.debug_bounds("MISSION_NO_EVENTS");
                assert!(bounds.is_some());
                bounds
            };

            for _ in 0..3 {
                fake.set_overview(data.clone());
                visual.run_until_parked();
                visual.update(|window, _| window.refresh());
                assert_eq!(
                    visual.debug_bounds("MISSION_NO_ROLES"),
                    initial_no_roles_bounds
                );
                assert!(visual.debug_bounds("MISSION_RAIL_NO_ROLES").is_some());
                assert_eq!(
                    visual.debug_bounds("MISSION_RAIL_ADD_ROLE").is_some(),
                    capable
                );
                workspace.read_with(&visual, |workspace, _| {
                    assert_eq!(
                        workspace.feed_empty_panel_min_height.get(),
                        initial_min_height
                    );
                });
                if with_events {
                    assert!(visual.debug_bounds("MISSION_NO_EVENTS").is_none());
                    assert!(visual.debug_bounds("MISSION_MESSAGE_GROUP e1").is_some());
                } else {
                    assert_eq!(
                        visual.debug_bounds("MISSION_NO_EVENTS"),
                        initial_no_events_bounds
                    );
                }
            }
        }
    }
}

#[test]
fn remote_zero_role_mission_hides_no_events_when_initial_feed_load_fails() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
    remote_test::enable_missions(&fake);
    let mut data = overview();
    data.missions[0].members.clear();
    data.missions[0].asks.clear();
    data.sessions.clear();
    fake.set_overview(data);
    let window = remote_test::add_window(&mut cx, &store, temp.path(), "remote-feed-error");
    window
        .update(&mut cx, |root, window, cx| {
            remote_test::choose_host(root, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    fake.queue_result(
        "feed",
        Err(runner_core::protocol::ClientError::msg("Feed unavailable")),
    );
    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.run_until_parked();
    let workspace = workspace(window, &visual);
    workspace.read_with(&visual, |workspace, cx| {
        let feed = workspace
            .presentation
            .remote_for_test()
            .unwrap()
            .feed
            .as_ref()
            .unwrap()
            .read(cx);
        assert!(!feed.loading);
        assert!(!feed.history_loaded);
        assert!(feed.error.is_some());
    });
    assert!(visual.debug_bounds("MISSION_FEED").is_some());
    assert!(visual.debug_bounds("MISSION_NO_ROLES").is_none());
    assert!(visual.debug_bounds("MISSION_NO_EVENTS").is_none());
}

fn push_member(
    data: &mut Overview,
    mission_idx: usize,
    handle: &str,
    session_id: &str,
    status: runner_core::companion::Status,
) {
    data.missions[mission_idx]
        .members
        .push(runner_core::companion::Member {
            handle: handle.into(),
            session: session_id.into(),
            lead: false,
        });
    let mut session = data.sessions["member"].clone();
    session.status.status = status;
    data.sessions.insert(session_id.into(), session);
}

#[test]
fn member_added_while_viewing_another_tab_or_mission_gets_a_tab_on_return() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, _, fake) = setup();
    let workspace = workspace(window, &cx);
    let mut data = overview();
    let mut second_mission = data.missions[0].clone();
    second_mission.id = "second-mission".into();
    second_mission.title = "Second mission".into();
    second_mission.members = vec![runner_core::companion::Member {
        handle: "lead".into(),
        session: "other-member".into(),
        lead: true,
    }];
    second_mission.asks.clear();
    data.missions.push(second_mission);
    data.sessions
        .insert("other-member".into(), data.sessions["member"].clone());
    fake.set_overview(data.clone());
    cx.run_until_parked();

    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Tab("remote-tab".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();

    push_member(
        &mut data,
        0,
        "extra-writer",
        "extra",
        runner_core::companion::Status::Working,
    );
    fake.set_overview(data.clone());
    cx.run_until_parked();

    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    workspace.read_with(&visual, |workspace, _| {
        assert_eq!(workspace.open_tabs, ["member", "extra"]);
        let members: Vec<String> = workspace
            .members()
            .into_iter()
            .map(|member| member.handle)
            .collect();
        assert_eq!(members, ["coder", "extra-writer"]);
        let roster: Vec<String> = workspace
            .mission_composer_roster()
            .into_iter()
            .map(|entry| entry.handle)
            .collect();
        assert_eq!(roster, ["coder", "extra-writer"]);
    });
    for selector in [
        "MISSION_TAB member",
        "MISSION_TAB extra",
        "MISSION_CARD member",
        "MISSION_CARD extra",
    ] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }

    window
        .update(&mut visual, |root, _, cx| {
            root.select_remote_item(RemoteItem::Mission("second-mission".into()), cx)
        })
        .unwrap();
    visual.run_until_parked();

    push_member(
        &mut data,
        0,
        "third",
        "third-id",
        runner_core::companion::Status::Working,
    );
    fake.set_overview(data);
    visual.run_until_parked();

    window
        .update(&mut visual, |root, _, cx| {
            root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
        })
        .unwrap();
    visual.run_until_parked();

    workspace.read_with(&visual, |workspace, _| {
        assert_eq!(workspace.open_tabs, ["member", "extra", "third-id"]);
        let roster: Vec<String> = workspace
            .mission_composer_roster()
            .into_iter()
            .map(|entry| entry.handle)
            .collect();
        assert_eq!(roster, ["coder", "extra-writer", "third"]);
    });
    for selector in ["MISSION_TAB third-id", "MISSION_CARD third-id"] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
}

#[test]
fn member_added_while_mission_is_stopped_gets_tab_when_reopened() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, _, fake) = setup();
    let workspace = workspace(window, &cx);

    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Tab("remote-tab".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();

    let mut stopped = overview();
    stopped.missions[0].state = runner_core::companion::MissionState::Stopped;
    stopped.sessions.get_mut("member").unwrap().status.status =
        runner_core::companion::Status::Stopped;
    push_member(
        &mut stopped,
        0,
        "extra-writer",
        "extra",
        runner_core::companion::Status::Stopped,
    );
    fake.set_overview(stopped);
    cx.run_until_parked();

    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    workspace.read_with(&visual, |workspace, _| {
        assert_eq!(workspace.open_tabs, ["member", "extra"]);
        let roster: Vec<String> = workspace
            .mission_composer_roster()
            .into_iter()
            .map(|entry| entry.handle)
            .collect();
        assert_eq!(roster, ["coder", "extra-writer"]);
    });
    for selector in [
        "MISSION_TAB member",
        "MISSION_TAB extra",
        "MISSION_CARD member",
        "MISSION_CARD extra",
    ] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
}

#[test]
fn closed_tab_stays_closed_while_added_member_gets_tab_and_presentation_holds() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, cx, window, _, fake) = setup();
    let workspace = workspace(window, &cx);
    let mut pushed = overview();
    push_member(
        &mut pushed,
        0,
        "second",
        "second-id",
        runner_core::companion::Status::Working,
    );
    let mut shell = pushed.sessions["member"].clone();
    shell.kind = runner_core::companion::SessionKind::Shell;
    for id in ["shell-a", "shell-b"] {
        pushed.sessions.insert(id.into(), shell.clone());
    }
    pushed.missions[0].shells = vec!["shell-a".into(), "shell-b".into()];
    fake.set_overview(pushed.clone());
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.select_mission_session("second-id", window, cx);
            workspace.close_mission_session_tab("member", window, cx);
            workspace.toggle_terminal_drawer(window, cx);
            workspace.activate_terminal_drawer_shell("shell-a", window, cx);
            workspace.layout.drawer.set_height(219.);
        })
    });
    visual.run_until_parked();

    window
        .update(&mut visual, |root, _, cx| {
            root.select_remote_item(RemoteItem::Tab("remote-tab".into()), cx)
        })
        .unwrap();
    visual.run_until_parked();

    push_member(
        &mut pushed,
        0,
        "extra-writer",
        "extra",
        runner_core::companion::Status::Working,
    );
    fake.set_overview(pushed.clone());
    visual.run_until_parked();

    window
        .update(&mut visual, |root, _, cx| {
            root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
        })
        .unwrap();
    visual.run_until_parked();

    workspace.read_with(&visual, |workspace, _| {
        assert_eq!(workspace.open_tabs, ["second-id", "extra"]);
        assert_eq!(
            workspace.active_tab,
            MissionTab::Session("second-id".into())
        );
        assert_eq!(
            workspace.presentation.selected_handle().map(String::as_str),
            Some("second")
        );
        assert!(workspace.layout.drawer.open());
        assert_eq!(workspace.layout.drawer.active_shell(), Some("shell-a"));
        assert_eq!(workspace.layout.drawer.height(), 219.);
        let roster: Vec<String> = workspace
            .mission_composer_roster()
            .into_iter()
            .map(|entry| entry.handle)
            .collect();
        assert_eq!(roster, ["coder", "second", "extra-writer"]);
    });
    assert!(visual.debug_bounds("MISSION_TAB member").is_none());
    for selector in [
        "MISSION_TAB second-id",
        "MISSION_TAB extra",
        "MISSION_CARD member",
        "MISSION_CARD second-id",
        "MISSION_CARD extra",
    ] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }

    pushed.missions[0].state = runner_core::companion::MissionState::Stopped;
    for member in &mut pushed.missions[0].members {
        pushed
            .sessions
            .get_mut(&member.session)
            .unwrap()
            .status
            .status = runner_core::companion::Status::Stopped;
    }
    fake.set_overview(pushed.clone());
    visual.run_until_parked();

    pushed.missions[0].state = runner_core::companion::MissionState::Running;
    for (member, next_id) in pushed.missions[0].members.iter_mut().zip([
        "coder-resumed",
        "second-resumed",
        "extra-resumed",
    ]) {
        let mut session = pushed.sessions.remove(&member.session).unwrap();
        session.status.status = runner_core::companion::Status::Working;
        member.session = next_id.into();
        pushed.sessions.insert(next_id.into(), session);
    }
    fake.set_overview(pushed);
    visual.run_until_parked();

    workspace.read_with(&visual, |workspace, _| {
        assert_eq!(workspace.open_tabs, ["second-resumed", "extra-resumed"]);
        assert_eq!(
            workspace.active_tab,
            MissionTab::Session("second-resumed".into())
        );
        assert_eq!(
            workspace.presentation.selected_handle().map(String::as_str),
            Some("second")
        );
        assert!(workspace.layout.drawer.open());
        assert_eq!(workspace.layout.drawer.active_shell(), Some("shell-a"));
        assert_eq!(workspace.layout.drawer.height(), 219.);
    });
}

#[test]
fn member_removed_while_away_loses_tab_and_active_tab_falls_back() {
    let (_temp, mut cx, window, _, fake) = setup();
    let workspace = workspace(window, &cx);
    let mut data = overview();
    push_member(
        &mut data,
        0,
        "second",
        "second-id",
        runner_core::companion::Status::Working,
    );
    push_member(
        &mut data,
        0,
        "third",
        "third-id",
        runner_core::companion::Status::Working,
    );
    fake.set_overview(data.clone());
    cx.run_until_parked();

    window
        .update(&mut cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.select_mission_session("second-id", window, cx);
                workspace.close_mission_session_tab("third-id", window, cx);
            })
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Tab("remote-tab".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();

    data.missions[0]
        .members
        .retain(|member| member.handle != "second");
    data.sessions.remove("second-id");
    fake.set_overview(data.clone());
    cx.run_until_parked();

    push_member(
        &mut data,
        0,
        "extra-writer",
        "extra",
        runner_core::companion::Status::Working,
    );
    fake.set_overview(data);
    cx.run_until_parked();

    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(&cx, |workspace, _| {
        assert_eq!(workspace.open_tabs, ["member", "extra"]);
        assert_eq!(workspace.active_tab, MissionTab::Feed);
        assert_eq!(workspace.presentation.selected_handle(), None);
    });
}

#[test]
fn saved_presentation_survives_same_host_reconnect_and_receives_added_members() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, cx, window, store, fake) = setup();
    let workspace = workspace(window, &cx);
    let mut pushed = overview();
    push_member(
        &mut pushed,
        0,
        "second",
        "second-id",
        runner_core::companion::Status::Working,
    );
    let mut shell = pushed.sessions["member"].clone();
    shell.kind = runner_core::companion::SessionKind::Shell;
    pushed.sessions.insert("shell-a".into(), shell);
    pushed.missions[0].shells = vec!["shell-a".into()];
    fake.set_overview(pushed.clone());
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.select_mission_session("second-id", window, cx);
            workspace.close_mission_session_tab("member", window, cx);
            workspace.toggle_terminal_drawer(window, cx);
            workspace.activate_terminal_drawer_shell("shell-a", window, cx);
            workspace.layout.drawer.set_height(219.);
            workspace
                .rail_surfaces
                .select(crate::surfaces::side_panel::PanelViewId::Files);
        })
    });
    visual.run_until_parked();

    let (target_before, generation_before, rail_before) = workspace.read_with(&visual, |w, _| {
        (
            w.client.target.clone(),
            w.generation,
            w.rail_surfaces.selection,
        )
    });

    window
        .update(&mut visual, |root, _, cx| {
            root.select_remote_item(RemoteItem::Tab("remote-tab".into()), cx)
        })
        .unwrap();
    visual.run_until_parked();

    window
        .update(&mut visual, |root, _, cx| {
            let clients = store.read(cx).remote_clients.clone();
            clients.update(cx, |clients, cx| {
                clients.connections.insert(
                    "host".into(),
                    crate::surfaces::remote_client::model::Connection::Reconnecting,
                );
                clients.invalidate_capabilities("host");
                cx.notify();
            });
            root.sync_remote_state(cx);
        })
        .unwrap();
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer":"host","state":"connected"}),
    );
    visual.run_until_parked();

    push_member(
        &mut pushed,
        0,
        "extra-writer",
        "extra",
        runner_core::companion::Status::Working,
    );
    fake.set_overview(pushed);
    visual.run_until_parked();

    window
        .update(&mut visual, |root, _, cx| {
            root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
        })
        .unwrap();
    visual.run_until_parked();

    workspace.read_with(&visual, |workspace, _| {
        assert_ne!(workspace.client.target, target_before);
        assert!(workspace.generation > generation_before);
        assert_eq!(workspace.open_tabs, ["second-id", "extra"]);
        assert_eq!(
            workspace.active_tab,
            MissionTab::Session("second-id".into())
        );
        assert_eq!(
            workspace.presentation.selected_handle().map(String::as_str),
            Some("second")
        );
        assert!(workspace.layout.drawer.open());
        assert_eq!(workspace.layout.drawer.active_shell(), Some("shell-a"));
        assert_eq!(workspace.layout.drawer.height(), 219.);
        assert_eq!(workspace.rail_surfaces.selection, rail_before);
    });
}

#[test]
fn remote_mission_rail_branch_row_tracks_online_checkout_and_hides_offline() {
    use runner_core::companion::{Checkout, CheckoutKind};

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, cx, window, _store, fake) = setup();
    let workspace = workspace(window, &cx);
    let mut pushed = overview();
    pushed.missions[0].cwd = Some(r"C:\remote-host\repo\.worktrees\feat-rail".into());
    pushed.missions[0].checkout = Some(Checkout::Git {
        top: r"C:\remote-host\repo\.worktrees\feat-rail".into(),
        main_checkout: r"C:\remote-host\repo".into(),
        kind: CheckoutKind::Linked,
        managed: true,
        branch: Some("feat/rail".into()),
        commit: Some("abc1234".into()),
        missing: false,
        missing_branch: None,
        candidate_branches: Vec::new(),
        auto_base: Some("origin/main".into()),
    });
    fake.set_overview(pushed.clone());
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_mission_info_popover(window, cx);
            cx.notify();
        });
    });
    visual.run_until_parked();

    assert!(
        visual.debug_bounds("MISSION_META_BRANCH").is_some(),
        "remote linked worktree mission should show Info branch row while online"
    );

    // Dropping remote connection must hide the Info branch row instead of showing stale branch info.
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer": "host", "state": "reconnecting"}),
    );
    visual.run_until_parked();
    assert!(
        visual.debug_bounds("MISSION_META_BRANCH").is_none(),
        "offline remote mission must hide Info branch row"
    );

    // Reconnecting restores it.
    fake.emit(
        "remote/connection",
        serde_json::json!({"computer": "host", "state": "connected"}),
    );
    fake.emit(
        "remote/overview",
        serde_json::json!({"computer": "host", "overview": pushed}),
    );
    visual.run_until_parked();
    assert!(
        visual.debug_bounds("MISSION_META_BRANCH").is_some(),
        "reconnected remote mission should restore Info branch row"
    );
}

fn wait_remote_mission(
    visual: &mut VisualTestContext,
    what: &str,
    mut ready: impl FnMut(&mut VisualTestContext) -> bool,
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

fn overview_with_shells(shells: &[&str]) -> Overview {
    let mut data = overview();
    for &id in shells {
        let mut shell = data.sessions["member"].clone();
        shell.kind = runner_core::companion::SessionKind::Shell;
        shell.runtime = None;
        shell.title = id.into();
        data.sessions.insert(id.into(), shell);
    }
    data.missions[0].shells = shells.iter().map(|id| (*id).to_owned()).collect();
    data
}

#[test]
fn remote_mission_drawer_cmd_w_closes_shells_and_restores_focus() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for on_session_tab in [false, true] {
        for push_before_result in [false, true] {
            let (_temp, cx, window, _store, fake) = setup();
            let workspace = workspace(window, &cx);
            fake.set_overview(overview_with_shells(&["shell-1", "shell-2", "shell-3"]));
            cx.run_until_parked();

            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            visual.simulate_resize(gpui::size(px(1600.), px(900.)));

            // Focus non-final middle shell shell-2 so closing it must select predecessor shell-1
            // (matching TerminalDrawer::remove) rather than jumping to shell-3.
            let focus_root = window
                .update(&mut visual, |_, window, cx| {
                    workspace.update(cx, |ws, cx| {
                        if on_session_tab {
                            ws.select_mission_session("member", window, cx);
                        } else {
                            ws.select_mission_feed(window, cx);
                        }
                        ws.layout.drawer.set_open(true);
                        ws.activate_terminal_drawer_shell("shell-2", window, cx);
                        ws.root_focus.clone()
                    })
                })
                .unwrap();

            wait_remote_mission(
                &mut visual,
                "shell-2 attached and focused before first Cmd+W",
                |v| {
                    let shell_focused = window
                        .read_with(v, |root, cx| {
                            root.remote_client
                                .remote_terminal_interactive("shell-2", cx)
                                .then(|| root.remote_client.remote_terminal_focus("shell-2"))
                                .flatten()
                        })
                        .unwrap()
                        .is_some_and(|focus| v.update(|w, _| focus.is_focused(w)));
                    let member_ready = !on_session_tab
                        || window
                            .read_with(v, |root, cx| {
                                root.remote_client.remote_terminal_interactive("member", cx)
                            })
                            .unwrap();
                    let drawer_flags = v.update(|w, cx| {
                        workspace.read_with(cx, |ws, cx| {
                            ws.drawer_focused(w, cx) && ws.drawer_was_focused
                        })
                    });
                    shell_focused && member_ready && drawer_flags
                },
            );

            // First Cmd+W closes non-final active shell-2 -> moves focus to predecessor shell-1.
            let after_first = overview_with_shells(&["shell-1", "shell-3"]);
            if push_before_result {
                window
                    .update(&mut visual, |root, window, cx| {
                        crate::surfaces::windowing::close_tab(root, window, cx);
                        let clients = root.app_store.read(cx).remote_clients.clone();
                        clients.update(cx, |clients, cx| {
                            clients.overviews.insert("host".into(), after_first.clone());
                            cx.notify();
                        });
                        root.sync_remote_state(cx);
                    })
                    .unwrap();
            } else {
                window
                    .update(&mut visual, crate::surfaces::windowing::close_tab)
                    .unwrap();
                wait_remote_mission(
                    &mut visual,
                    "first close RPC reply applied on UI with drawer_expected set",
                    |v| {
                        fake.request_count("close") == 1
                            && workspace.read_with(v, |ws, _| {
                                ws.presentation
                                    .remote_for_test()
                                    .unwrap()
                                    .drawer_expected
                                    .is_some()
                            })
                    },
                );
                workspace.read_with(&visual, |ws, _| {
                    assert_eq!(
                        ws.layout.drawer.shells(),
                        &["shell-1", "shell-2", "shell-3"]
                    );
                });
                // A second Cmd+W before the host overview arrives must not fire a duplicate close.
                window
                    .update(&mut visual, crate::surfaces::windowing::close_tab)
                    .unwrap();
                visual.run_until_parked();
                assert_eq!(fake.request_count("close"), 1);
                fake.set_overview(after_first);
            }

            wait_remote_mission(
                &mut visual,
                "shell-2 closed and predecessor shell-1 attached and focused",
                |v| {
                    let close_sent = fake.request_count("close") == 1;
                    let drawer_ok = workspace.read_with(v, |ws, _| {
                        ws.layout.drawer.open()
                            && ws.layout.drawer.shells() == ["shell-1", "shell-3"]
                            && ws.layout.drawer.active_shell() == Some("shell-1")
                            && ws.drawer_was_focused
                            && ws
                                .presentation
                                .remote_for_test()
                                .unwrap()
                                .drawer_expected
                                .is_none()
                    });
                    let shell_1_focused = window
                        .read_with(v, |root, cx| {
                            root.remote_client
                                .remote_terminal_interactive("shell-1", cx)
                                .then(|| root.remote_client.remote_terminal_focus("shell-1"))
                                .flatten()
                        })
                        .unwrap()
                        .is_some_and(|focus| v.update(|w, _| focus.is_focused(w)));
                    close_sent && drawer_ok && shell_1_focused
                },
            );
            assert_eq!(
                remote_test::action_trace(&fake, "close")[0]["session"],
                "shell-2"
            );

            fake.input.lock().unwrap().clear();
            visual.simulate_keystrokes("x");
            visual.run_until_parked();
            assert_eq!(
                fake.input.lock().unwrap().as_slice(),
                &[("remote:host:shell-1".into(), b"x".to_vec())]
            );

            // Second Cmd+W closes index-0 shell-1 -> moves focus to remaining shell-3.
            let after_second = overview_with_shells(&["shell-3"]);
            if push_before_result {
                window
                    .update(&mut visual, |root, window, cx| {
                        crate::surfaces::windowing::close_tab(root, window, cx);
                        let clients = root.app_store.read(cx).remote_clients.clone();
                        clients.update(cx, |clients, cx| {
                            clients
                                .overviews
                                .insert("host".into(), after_second.clone());
                            cx.notify();
                        });
                        root.sync_remote_state(cx);
                    })
                    .unwrap();
            } else {
                window
                    .update(&mut visual, crate::surfaces::windowing::close_tab)
                    .unwrap();
                wait_remote_mission(
                    &mut visual,
                    "second close RPC reply applied on UI with drawer_expected set",
                    |v| {
                        fake.request_count("close") == 2
                            && workspace.read_with(v, |ws, _| {
                                ws.presentation
                                    .remote_for_test()
                                    .unwrap()
                                    .drawer_expected
                                    .is_some()
                            })
                    },
                );
                fake.set_overview(after_second);
            }

            wait_remote_mission(
                &mut visual,
                "shell-1 closed and shell-3 attached and focused",
                |v| {
                    let close_sent = fake.request_count("close") == 2;
                    let drawer_ok = workspace.read_with(v, |ws, _| {
                        ws.layout.drawer.open()
                            && ws.layout.drawer.shells() == ["shell-3"]
                            && ws.layout.drawer.active_shell() == Some("shell-3")
                            && ws.drawer_was_focused
                            && ws
                                .presentation
                                .remote_for_test()
                                .unwrap()
                                .drawer_expected
                                .is_none()
                    });
                    let shell_3_focused = window
                        .read_with(v, |root, cx| {
                            root.remote_client
                                .remote_terminal_interactive("shell-3", cx)
                                .then(|| root.remote_client.remote_terminal_focus("shell-3"))
                                .flatten()
                        })
                        .unwrap()
                        .is_some_and(|focus| v.update(|w, _| focus.is_focused(w)));
                    close_sent && drawer_ok && shell_3_focused
                },
            );
            assert_eq!(
                remote_test::action_trace(&fake, "close")[1]["session"],
                "shell-1"
            );

            fake.input.lock().unwrap().clear();
            visual.simulate_keystrokes("y");
            visual.run_until_parked();
            assert_eq!(
                fake.input.lock().unwrap().as_slice(),
                &[("remote:host:shell-3".into(), b"y".to_vec())]
            );

            // Third Cmd+W closes last shell-3, hides drawer, clears drawer_was_focused, and
            // returns focus to the remote mission (member terminal or Feed root_focus).
            let after_third = overview_with_shells(&[]);
            if push_before_result {
                window
                    .update(&mut visual, |root, window, cx| {
                        crate::surfaces::windowing::close_tab(root, window, cx);
                        let clients = root.app_store.read(cx).remote_clients.clone();
                        clients.update(cx, |clients, cx| {
                            clients.overviews.insert("host".into(), after_third.clone());
                            cx.notify();
                        });
                        root.sync_remote_state(cx);
                    })
                    .unwrap();
            } else {
                window
                    .update(&mut visual, crate::surfaces::windowing::close_tab)
                    .unwrap();
                wait_remote_mission(
                    &mut visual,
                    "third close RPC reply applied on UI with drawer_expected set",
                    |v| {
                        fake.request_count("close") == 3
                            && workspace.read_with(v, |ws, _| {
                                ws.presentation
                                    .remote_for_test()
                                    .unwrap()
                                    .drawer_expected
                                    .is_some()
                            })
                    },
                );
                fake.set_overview(after_third);
            }

            wait_remote_mission(
                &mut visual,
                "last shell closed, drawer hidden, focus restored to remote mission",
                |v| {
                    let close_sent = fake.request_count("close") == 3;
                    let drawer_ok = workspace.read_with(v, |ws, _| {
                        !ws.layout.drawer.open()
                            && ws.layout.drawer.shells().is_empty()
                            && !ws.drawer_was_focused
                            && ws
                                .presentation
                                .remote_for_test()
                                .unwrap()
                                .drawer_expected
                                .is_none()
                    });
                    let focus_ok = if on_session_tab {
                        window
                            .read_with(v, |root, _| {
                                root.remote_client.remote_terminal_focus("member")
                            })
                            .unwrap()
                            .is_some_and(|focus| v.update(|w, _| focus.is_focused(w)))
                    } else {
                        v.update(|w, _| focus_root.is_focused(w))
                    };
                    close_sent && drawer_ok && focus_ok
                },
            );
            assert_eq!(
                remote_test::action_trace(&fake, "close")[2]["session"],
                "shell-3"
            );

            if on_session_tab {
                fake.input.lock().unwrap().clear();
                visual.simulate_keystrokes("m");
                visual.run_until_parked();
                assert_eq!(
                    fake.input.lock().unwrap().as_slice(),
                    &[("remote:host:member".into(), b"m".to_vec())]
                );
            }

            window
                .update(&mut visual, |_, window, _| window.remove_window())
                .unwrap();
        }
    }
}

#[test]
fn remote_mission_drawer_cmd_w_respects_close_capability_empty_fallback_and_non_regressions() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();

    // 1. Host without "close" capability: Cmd+W in focused drawer shell sends no RPC and leaves
    //    the drawer open with its shell intact.
    {
        let (_temp, mut cx, window, store, fake) = setup();
        let workspace = workspace(window, &cx);
        fake.without_features(&["close"]);
        window
            .update(&mut cx, |root, _, cx| {
                store
                    .read(cx)
                    .remote_clients
                    .clone()
                    .update(cx, |clients, cx| {
                        clients.invalidate_capabilities("host");
                        cx.notify();
                    });
                root.sync_remote_state(cx);
            })
            .unwrap();
        fake.set_overview(overview_with_shells(&["shell-1"]));
        cx.run_until_parked();

        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(gpui::size(px(1600.), px(900.)));
        window
            .update(&mut visual, |_, window, cx| {
                workspace.update(cx, |ws, cx| {
                    ws.layout.drawer.set_open(true);
                    ws.activate_terminal_drawer_shell("shell-1", window, cx);
                })
            })
            .unwrap();
        wait_remote_mission(
            &mut visual,
            "shell-1 focused without close capability",
            |v| {
                window
                    .read_with(v, |root, _| {
                        root.remote_client.remote_terminal_focus("shell-1")
                    })
                    .unwrap()
                    .is_some_and(|focus| v.update(|w, _| focus.is_focused(w)))
            },
        );

        window
            .update(&mut visual, crate::surfaces::windowing::close_tab)
            .unwrap();
        visual.run_until_parked();
        assert_eq!(fake.request_count("close"), 0);
        workspace.read_with(&visual, |ws, _| {
            assert!(ws.layout.drawer.open());
            assert_eq!(ws.layout.drawer.shells(), &["shell-1"]);
            assert!(ws
                .presentation
                .remote_for_test()
                .unwrap()
                .drawer_expected
                .is_none());
        });
        window
            .update(&mut visual, |_, window, _| window.remove_window())
            .unwrap();
    }

    // 2. Empty open & focused drawer (active_shell() == None) falls back safely to hiding.
    {
        let (_temp, cx, window, _store, fake) = setup();
        let workspace = workspace(window, &cx);
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(gpui::size(px(1600.), px(900.)));
        visual.update(|window, _| window.activate_window());
        let focus_root = window
            .update(&mut visual, |_, window, cx| {
                workspace.update(cx, |ws, cx| {
                    ws.select_mission_feed(window, cx);
                    ws.layout.drawer.set_open(true);
                    ws.drawer_focus.focus(window, cx);
                    ws.root_focus.clone()
                })
            })
            .unwrap();
        wait_remote_mission(&mut visual, "empty remote drawer focused", |v| {
            v.update(|w, cx| {
                workspace.read_with(cx, |ws, cx| {
                    ws.drawer_focused(w, cx)
                        && ws.drawer_was_focused
                        && ws.layout.drawer.active_shell().is_none()
                })
            })
        });

        window
            .update(&mut visual, crate::surfaces::windowing::close_tab)
            .unwrap();
        wait_remote_mission(&mut visual, "empty remote drawer hidden", |v| {
            workspace.read_with(v, |ws, _| {
                !ws.layout.drawer.open() && !ws.drawer_was_focused
            }) && v.update(|w, _| focus_root.is_focused(w))
        });
        assert_eq!(fake.request_count("close"), 0);
        window
            .update(&mut visual, |_, window, _| window.remove_window())
            .unwrap();
    }

    // 3. Non-drawer Cmd+W still closes the active remote mission session tab, and hide/toggle keep
    //    both shells intact.
    {
        let (_temp, cx, window, _store, fake) = setup();
        let workspace = workspace(window, &cx);
        fake.set_overview(overview_with_shells(&["shell-1", "shell-2"]));
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(gpui::size(px(1600.), px(900.)));
        window
            .update(&mut visual, |_, window, cx| {
                workspace.update(cx, |ws, cx| {
                    ws.layout.drawer.set_open(true);
                    ws.select_mission_session("member", window, cx);
                })
            })
            .unwrap();
        wait_remote_mission(&mut visual, "member tab focused outside drawer", |v| {
            window
                .read_with(v, |root, _| {
                    root.remote_client.remote_terminal_focus("member")
                })
                .unwrap()
                .is_some_and(|focus| v.update(|w, _| focus.is_focused(w)))
        });

        window
            .update(&mut visual, crate::surfaces::windowing::close_tab)
            .unwrap();
        wait_remote_mission(&mut visual, "member tab closed to Feed", |v| {
            workspace.read_with(v, |ws, _| ws.active_tab == MissionTab::Feed)
        });
        assert_eq!(fake.request_count("close"), 0);
        workspace.read_with(&visual, |ws, _| {
            assert!(ws.layout.drawer.open());
            assert_eq!(ws.layout.drawer.shells(), &["shell-1", "shell-2"]);
        });

        // Hide button and toggle leave shells intact.
        window
            .update(&mut visual, |_, window, cx| {
                workspace.update(cx, |ws, cx| ws.hide_terminal_drawer(window, cx))
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(fake.request_count("close"), 0);
        workspace.read_with(&visual, |ws, _| {
            assert!(!ws.layout.drawer.open());
            assert_eq!(ws.layout.drawer.shells(), &["shell-1", "shell-2"]);
            assert!(!ws.drawer_was_focused);
        });
        visual.dispatch_action(crate::ToggleTerminalDrawer);
        visual.run_until_parked();
        workspace.read_with(&visual, |ws, _| {
            assert!(ws.layout.drawer.open());
            assert_eq!(ws.layout.drawer.shells(), &["shell-1", "shell-2"]);
        });
        assert_eq!(fake.request_count("close"), 0);

        // Dispatching ToggleSessionInfo (⌘I / Ctrl+Shift+I) on a remote mission opens and closes
        // the Mission Title ▾ Info popover cleanly.
        window
            .update(&mut visual, |root, window, cx| {
                root.root_focus.focus(window, cx);
            })
            .unwrap();
        visual.run_until_parked();
        visual.dispatch_action(crate::ToggleSessionInfo);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while visual.debug_bounds("MISSION_META_ID").is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for remote mission S2 Info popover to open"
            );
            visual.run_until_parked();
        }
        visual.dispatch_action(crate::ToggleSessionInfo);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while visual.debug_bounds("MISSION_META_ID").is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for remote mission S2 Info popover to close"
            );
            visual.run_until_parked();
        }

        window
            .update(&mut visual, |_, window, _| window.remove_window())
            .unwrap();
    }
}

#[test]
fn remote_mission_rail_shows_runtime_line_handles_older_hosts_and_hides_session_key() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, cx, window, _store, fake) = setup();
    let workspace = workspace(window, &cx);
    let data: Overview = serde_json::from_value(serde_json::json!({
        "projects": [],
        "tabs": [],
        "missions": [{
            "id": "remote-mission",
            "project": null,
            "title": "Host mission",
            "crew": "Mixed",
            "state": "running",
            "pinned": false,
            "members": [
                {"handle": "reviewer", "session": "reviewer", "lead": false},
                {"handle": "legacy", "session": "legacy", "lead": false},
                {"handle": "shell", "session": "shell", "lead": false}
            ],
            "asks": [],
            "shells": []
        }],
        "sessions": {
            "reviewer": {
                "kind": "agent",
                "title": "Reviewer",
                "runtime": "codex",
                "model": "gpt-6-astra",
                "effort": "xhigh",
                "cwd": "/tmp/work",
                "status": "working",
                "unread": false
            },
            "legacy": {
                "kind": "agent",
                "title": "Legacy",
                "runtime": "claude-code",
                "cwd": "/tmp/work",
                "status": "idle",
                "unread": false
            },
            "shell": {
                "kind": "shell",
                "title": "Shell",
                "runtime": null,
                "cwd": "/tmp/work",
                "status": "idle",
                "unread": false
            }
        }
    }))
    .unwrap();
    fake.set_overview(data);
    cx.run_until_parked();

    workspace.read_with(&cx, |ws, _| {
        let members = ws.members();
        assert_eq!(
            rail::session_runtime_line(&members[0]).as_deref(),
            Some("Codex · gpt-6-astra · Extra high")
        );
        assert_eq!(
            rail::session_runtime_line(&members[1]).as_deref(),
            Some("Claude Code")
        );
        assert_eq!(rail::session_runtime_line(&members[2]), None);
    });

    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(gpui::size(px(1200.), px(800.)));
    visual.run_until_parked();

    for id in ["reviewer", "legacy"] {
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
        assert!(status.left() >= avatar.right(), "{id}");
        assert!(status.top() >= handle.bottom(), "{id}");
        assert!(runtime.top() >= status.bottom(), "{id}");
        assert!(runtime.right() <= card.right(), "{id}");
        assert_eq!(mark.size, gpui::size(px(12.), px(12.)), "{id}");
        assert!(
            visual
                .debug_bounds(format!("MISSION_CARD_KEY {id}").leak())
                .is_none(),
            "remote card must keep session_key hidden: {id}"
        );
    }
    assert!(visual.debug_bounds("MISSION_CARD shell").is_some());
    assert!(visual.debug_bounds("MISSION_CARD_STATUS shell").is_none());
    assert!(visual.debug_bounds("MISSION_CARD_RUNTIME shell").is_none());

    window
        .update(&mut visual, |_, window, _| window.remove_window())
        .unwrap();
}

fn rich_host_catalog() -> serde_json::Value {
    serde_json::json!({
        "roles": [
            {
                "id": "role-coder",
                "name": "Host Coder",
                "handle": "coder",
                "runtime": "codex",
                "model": "gpt-5.4",
                "effort": "high",
                "speed": null
            },
            {
                "id": "role-reviewer",
                "name": "Host Reviewer",
                "handle": "reviewer",
                "runtime": "claude-code",
                "model": "claude-opus-4-6",
                "effort": "medium",
                "speed": null
            }
        ],
        "runtimes": [
            {
                "name": "codex",
                "label": "Host Codex",
                "available": true,
                "nativeFork": true,
                "defaultModel": "gpt-5.4",
                "defaultEffort": "medium",
                "models": [
                    {"value": "gpt-5.4", "label": "gpt-5.4"},
                    {"value": "gpt-5.4-mini", "label": "gpt-5.4-mini"}
                ],
                "efforts": [
                    {"value": "low", "label": "Low"},
                    {"value": "medium", "label": "Medium"},
                    {"value": "high", "label": "High"}
                ],
                "speeds": ["standard", "fast"]
            },
            {
                "name": "claude-code",
                "label": "Host Claude",
                "available": true,
                "nativeFork": true,
                "defaultModel": "claude-opus-4-6",
                "defaultEffort": "medium",
                "models": [
                    {"value": "claude-opus-4-6", "label": "Claude Opus 4.6"}
                ],
                "efforts": [
                    {"value": "low", "label": "Low"},
                    {"value": "medium", "label": "Medium"},
                    {"value": "high", "label": "High"}
                ],
                "speeds": []
            }
        ],
        "accounts": [{"id": "host-account", "name": "Host account"}],
        "activeAccount": "host-account",
        "crews": [],
        "projects": [],
        "recentFolders": []
    })
}

fn wait_until(
    visual: &mut VisualTestContext,
    mut pred: impl FnMut(&mut VisualTestContext) -> bool,
    what: &str,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        visual.run_until_parked();
        if pred(visual) {
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
fn older_host_capability_gating_and_stopped_mission_hide_remote_role_controls() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for (omit, expect_add, expect_remove) in [
        (&["addMissionRole"][..], false, true),
        (&["catalog"][..], false, true),
        (&["removeMissionRole"][..], true, false),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
        remote_test::enable_missions(&fake);
        fake.without_features(omit);
        fake.set_overview(overview());
        let window = remote_test::add_window(&mut cx, &store, temp.path(), "remote-role-caps");
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
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.run_until_parked();
        let ws = workspace(window, &visual);
        assert_eq!(
            visual.debug_bounds("MISSION_RAIL_ADD_ROLE").is_some(),
            expect_add,
            "omit={omit:?}"
        );
        assert_eq!(
            visual.debug_bounds("MISSION_CARD_REMOVE member").is_some(),
            expect_remove,
            "omit={omit:?}"
        );
        window
            .update(&mut visual, |_, window, cx| {
                assert_eq!(
                    ws.read(cx).add_role_context(window, cx).is_some(),
                    expect_add,
                    "omit={omit:?}"
                );
                if !expect_remove {
                    ws.update(cx, |ws, cx| ws.request_remove_role("member", window, cx));
                    assert!(ws.read(cx).remove_confirm.is_none());
                }
            })
            .unwrap();
        // Stopped remote mission hides all role mutation controls even when host advertises them.
        let mut stopped = overview();
        stopped.missions[0].state = runner_core::companion::MissionState::Stopped;
        fake.set_overview(stopped);
        visual.run_until_parked();
        assert!(visual.debug_bounds("MISSION_RAIL_ADD_ROLE").is_none());
        assert!(visual.debug_bounds("MISSION_CARD_REMOVE member").is_none());
    }
}

#[test]
fn remote_add_role_modal_loads_catalog_validates_removed_handles_and_submits_with_overrides() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
    remote_test::enable_missions(&fake);
    let mut data = overview();
    data.missions[0].removed_handles = vec!["coder-2".into()];
    fake.set_overview(data.clone());
    let window = remote_test::add_window(&mut cx, &store, temp.path(), "remote-add-role");
    window
        .update(&mut cx, |root, window, cx| {
            remote_test::choose_host(root, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    fake.queue_result(
        "feed",
        Ok(serde_json::json!({
            "entries": [{
                "id": "e-rem-3",
                "ts": "2026-10-24T05:01:00Z",
                "from": "human",
                "to": [],
                "kind": "signal",
                "signal": "role_removed",
                "payload": {"handle": "coder-3"}
            }],
            "before": null
        })),
    );
    window
        .update(&mut cx, |root, _, cx| {
            root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx)
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.run_until_parked();
    let ws = workspace(window, &visual);
    fake.queue_result("catalog", Ok(rich_host_catalog()));
    let local_calls_before = fake.local_call_count();

    window
        .update(&mut visual, |root, window, cx| {
            let context = ws.read(cx).add_role_context(window, cx).unwrap();
            root.open_add_role_modal(context, window, cx);
        })
        .unwrap();
    // Before background catalog completes, modal renders in loading state with controls disabled.
    visual.update(|window, _| window.refresh());
    window
        .read_with(&visual, |root, cx| {
            assert!(root.create_modal_open());
            assert!(root.add_role_loading_catalog_for_test());
            assert_eq!(
                root.add_role_placeholder_for_test(cx).as_deref(),
                Some("Loading…")
            );
            assert_eq!(
                root.add_role_controls_disabled_for_test(cx),
                Some((true, true, true))
            );
            assert!(!root.add_role_can_submit_for_test(cx));
        })
        .unwrap();
    assert!(visual.debug_bounds("ADD_ROLE_FORM").is_some());
    assert!(visual.debug_bounds("ADD_ROLE_EMPTY_ROLES").is_none());

    wait_until(
        &mut visual,
        |visual| {
            window
                .read_with(visual, |root, _| !root.add_role_loading_catalog_for_test())
                .unwrap()
        },
        "remote catalog load",
    );
    assert_eq!(
        fake.local_call_count(),
        local_calls_before,
        "remote Add Role modal must never call local daemon runtime_refresh_models"
    );
    window
        .read_with(&visual, |root, cx| {
            assert_eq!(
                root.add_role_handle_for_test(cx).as_deref(),
                Some("coder-4"),
                "must skip active coder, Overview.removedHandles coder-2, and loaded role_removed coder-3"
            );
            assert_eq!(
                root.add_role_controls_disabled_for_test(cx),
                Some((false, false, false))
            );
            assert!(root.add_role_can_submit_for_test(cx));
        })
        .unwrap();

    for taken in ["coder", "coder-2", "coder-3"] {
        window
            .update(&mut visual, |root, _, cx| {
                root.set_add_role_handle_for_test(taken, cx);
                assert!(!root.add_role_can_submit_for_test(cx), "{taken}");
            })
            .unwrap();
    }
    window
        .update(&mut visual, |root, _, cx| {
            root.set_add_role_handle_for_test("coder-4", cx);
            root.set_add_role_overrides_for_test(
                None,
                Some("gpt-5.4-mini"),
                Some("low"),
                Some("fast"),
                cx,
            );
            root.set_add_role_task_for_test("Audit the patch", cx);
            assert!(root.add_role_can_submit_for_test(cx));
        })
        .unwrap();

    // First submit is rejected by the host (e.g. handle taken concurrently).
    fake.queue_result(
        "addMissionRole",
        Err(runner_core::protocol::ClientError::msg(
            "That handle is already in use in this mission.",
        )),
    );
    window
        .update(&mut visual, |root, window, cx| {
            root.submit_add_role_for_test(window, cx);
        })
        .unwrap();
    wait_until(
        &mut visual,
        |visual| {
            window
                .read_with(visual, |root, _| root.add_role_error_for_test().is_some())
                .unwrap()
        },
        "rejected addMissionRole response",
    );
    window
        .read_with(&visual, |root, cx| {
            assert!(root.create_modal_open());
            assert_eq!(
                root.add_role_error_for_test().as_deref(),
                Some("That handle is already in use in this mission.")
            );
            assert_eq!(
                root.add_role_controls_disabled_for_test(cx),
                Some((false, false, false))
            );
        })
        .unwrap();
    assert!(visual.debug_bounds("ADD_ROLE_ERROR").is_some());
    assert_eq!(ws.read_with(&visual, |ws, _| ws.members().len()), 1);

    // Second submit succeeds: modal closes and wire request carries overrides, task and size.
    window
        .update(&mut visual, |root, window, cx| {
            root.set_add_role_handle_for_test("coder-5", cx);
            root.submit_add_role_for_test(window, cx);
        })
        .unwrap();
    wait_until(
        &mut visual,
        |visual| {
            window
                .read_with(visual, |root, _| !root.create_modal_open())
                .unwrap()
        },
        "successful addMissionRole close",
    );
    let requests = remote_test::action_trace(&fake, "addMissionRole");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1]["mission"], "remote-mission");
    assert_eq!(requests[1]["role"], "role-coder");
    assert_eq!(requests[1]["handle"], "coder-5");
    assert_eq!(requests[1]["model"], "gpt-5.4-mini");
    assert_eq!(requests[1]["effort"], "low");
    assert_eq!(requests[1]["speed"], "fast");
    assert_eq!(requests[1]["task"], "Audit the patch");
    assert!(requests[1]["cols"].as_u64().unwrap() >= 20);
    assert!(requests[1]["rows"].as_u64().unwrap() >= 8);

    // Pushed Overview adds the member card, auto-opens its tab and updates composer roster.
    push_member(
        &mut data,
        0,
        "coder-5",
        "added-member",
        runner_core::companion::Status::Working,
    );
    fake.set_overview(data);
    visual.run_until_parked();
    assert!(visual.debug_bounds("MISSION_CARD added-member").is_some());
    assert!(visual.debug_bounds("MISSION_TAB added-member").is_some());
    ws.read_with(&visual, |ws, _| {
        assert!(ws
            .mission_composer_roster()
            .iter()
            .any(|entry| entry.handle == "coder-5"));
    });
}

#[test]
fn remote_add_role_modal_handles_empty_catalog_and_catalog_failure_cleanly() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, cx, window, _, fake) = setup();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let ws = workspace(window, &visual);

    // Empty host catalog shows remote-specific hint and keeps Submit disabled.
    fake.queue_result(
        "catalog",
        Ok(serde_json::json!({
            "roles": [],
            "runtimes": [],
            "accounts": [],
            "activeAccount": "",
            "crews": [],
            "projects": [],
            "recentFolders": []
        })),
    );
    window
        .update(&mut visual, |root, window, cx| {
            let context = ws.read(cx).add_role_context(window, cx).unwrap();
            root.open_add_role_modal(context, window, cx);
        })
        .unwrap();
    wait_until(
        &mut visual,
        |visual| {
            window
                .read_with(visual, |root, _| !root.add_role_loading_catalog_for_test())
                .unwrap()
        },
        "empty catalog load",
    );
    visual.update(|window, _| window.refresh());
    assert!(visual.debug_bounds("ADD_ROLE_EMPTY_ROLES").is_some());
    assert!(visual.debug_bounds("ADD_ROLE_ERROR").is_none());
    window
        .read_with(&visual, |root, cx| {
            assert!(!root.add_role_can_submit_for_test(cx));
        })
        .unwrap();
    // Verify both locales exist and carry the remote host-accurate wording.
    assert_eq!(
        crate::i18n::text_for(
            crate::i18n::Locale::English,
            "ui.no_roles_yet_on_this_computer_create_one_on_the_host_first"
        ),
        "No roles yet on this computer. Create one on the host first."
    );
    assert_eq!(
        crate::i18n::text_for(
            crate::i18n::Locale::SimplifiedChinese,
            "ui.no_roles_yet_on_this_computer_create_one_on_the_host_first"
        ),
        "该电脑暂无角色。请先在主机上创建。"
    );
    window
        .update(&mut visual, |root, window, cx| {
            root.close_add_role_for_test(window, cx);
        })
        .unwrap();
    visual.run_until_parked();

    // Failed catalog load displays notice_banner, hides empty-roles warning, and lets Cancel close.
    fake.queue_result(
        "catalog",
        Err(runner_core::protocol::ClientError::msg(
            "Host catalog unreachable.",
        )),
    );
    window
        .update(&mut visual, |root, window, cx| {
            let context = ws.read(cx).add_role_context(window, cx).unwrap();
            root.open_add_role_modal(context, window, cx);
        })
        .unwrap();
    wait_until(
        &mut visual,
        |visual| {
            window
                .read_with(visual, |root, _| root.add_role_catalog_failed_for_test())
                .unwrap()
        },
        "failed catalog load",
    );
    visual.update(|window, _| window.refresh());
    assert!(visual.debug_bounds("ADD_ROLE_ERROR").is_some());
    assert!(visual.debug_bounds("ADD_ROLE_EMPTY_ROLES").is_none());
    window
        .read_with(&visual, |root, cx| {
            assert_eq!(
                root.add_role_error_for_test().as_deref(),
                Some("Host catalog unreachable.")
            );
            assert_eq!(
                root.add_role_controls_disabled_for_test(cx),
                Some((true, true, true))
            );
            assert!(!root.add_role_can_submit_for_test(cx));
        })
        .unwrap();
    window
        .update(&mut visual, |root, window, cx| {
            root.close_add_role_for_test(window, cx);
        })
        .unwrap();
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| assert!(!root.create_modal_open()))
        .unwrap();
}

#[test]
fn remote_add_role_modal_discards_stale_replies_and_closes_when_mission_stops() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, cx, window, _, fake) = setup();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let ws = workspace(window, &visual);

    struct StaleCatalogTransport(std::sync::atomic::AtomicUsize);
    impl runner_core::protocol::Transport for StaleCatalogTransport {
        fn call(
            &self,
            _: runner_core::protocol::Request,
        ) -> Result<runner_core::protocol::Response, runner_core::protocol::ClientError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(runner_core::protocol::Response::remote_request(Err(
                runner_core::protocol::ClientError::msg("stale catalog error"),
            )))
        }
        fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
            unreachable!()
        }
    }

    // 1. Close and reopen while the first catalog request is still in-flight: give modal #1 its
    // own failing transport so regardless of background task execution order, modal #1 always
    // receives the stale error and modal #2 always receives the fresh catalog cleanly.
    let stale_transport = Arc::new(StaleCatalogTransport(Default::default()));
    let catalog_before = fake.request_count("catalog");
    fake.queue_result("catalog", Ok(rich_host_catalog()));
    window
        .update(&mut visual, |root, window, cx| {
            let mut stale_context = ws.read(cx).add_role_context(window, cx).unwrap();
            let stale_access =
                crate::surfaces::front_page::RemoteAccess::new(stale_context.client.target.clone());
            stale_access.set_features(vec!["catalog".into(), "addMissionRole".into()]);
            stale_context.client = crate::surfaces::front_page::FrontPageClient::remote(
                runner_core::protocol::DaemonClient::new(stale_transport.clone()),
                stale_access,
            );
            root.open_add_role_modal(stale_context, window, cx);
            assert!(root.add_role_loading_catalog_for_test());
            root.close_add_role_for_test(window, cx);
            assert!(!root.create_modal_open());
            let fresh_context = ws.read(cx).add_role_context(window, cx).unwrap();
            root.open_add_role_modal(fresh_context, window, cx);
            assert!(root.add_role_loading_catalog_for_test());
        })
        .unwrap();
    wait_until(
        &mut visual,
        |visual| {
            stale_transport.0.load(std::sync::atomic::Ordering::SeqCst) == 1
                && fake.request_count("catalog") > catalog_before
                && window
                    .read_with(visual, |root, _| !root.add_role_loading_catalog_for_test())
                    .unwrap()
        },
        "both catalog replies settled",
    );
    window
        .read_with(&visual, |root, cx| {
            assert!(root.create_modal_open());
            assert_eq!(root.add_role_error_for_test(), None);
            assert!(!root.add_role_catalog_failed_for_test());
            assert_eq!(
                root.add_role_handle_for_test(cx).as_deref(),
                Some("coder-2")
            );
        })
        .unwrap();

    // 2. Mission stops while an open, non-submitting modal is active: modal closes automatically.
    let mut stopped = overview();
    stopped.missions[0].state = runner_core::companion::MissionState::Stopped;
    fake.set_overview(stopped);
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| {
            assert!(
                !root.create_modal_open(),
                "non-submitting remote Add Role modal must close when mission stops"
            );
        })
        .unwrap();

    // 3. Mission restarts; open modal, then stop mission while catalog is loading -> closes cleanly.
    fake.set_overview(overview());
    visual.run_until_parked();
    fake.queue_result("catalog", Ok(rich_host_catalog()));
    window
        .update(&mut visual, |root, window, cx| {
            let context = ws.read(cx).add_role_context(window, cx).unwrap();
            root.open_add_role_modal(context, window, cx);
            assert!(root.add_role_loading_catalog_for_test());
        })
        .unwrap();
    let mut stopped = overview();
    stopped.missions[0].state = runner_core::companion::MissionState::Stopped;
    fake.set_overview(stopped);
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| assert!(!root.create_modal_open()))
        .unwrap();

    // 4. Mission restarts; submit addMissionRole, then immediately return to this computer while
    // the submit is still in-flight -> stale submit reply is discarded and modal stays closed.
    fake.set_overview(overview());
    visual.run_until_parked();
    fake.queue_result("catalog", Ok(rich_host_catalog()));
    window
        .update(&mut visual, |root, window, cx| {
            let context = ws.read(cx).add_role_context(window, cx).unwrap();
            root.open_add_role_modal(context, window, cx);
        })
        .unwrap();
    wait_until(
        &mut visual,
        |visual| {
            window
                .read_with(visual, |root, _| !root.add_role_loading_catalog_for_test())
                .unwrap()
        },
        "catalog load before submit",
    );
    let submits_before = fake.request_count("addMissionRole");
    fake.queue_result(
        "addMissionRole",
        Err(runner_core::protocol::ClientError::msg(
            "stale submit error",
        )),
    );
    window
        .update(&mut visual, |root, window, cx| {
            root.submit_add_role_for_test(window, cx);
            root.return_to_this_computer(window, cx);
            assert!(!root.create_modal_open());
        })
        .unwrap();
    wait_until(
        &mut visual,
        |_| fake.request_count("addMissionRole") == submits_before + 1,
        "in-flight submit request completion",
    );
    window
        .read_with(&visual, |root, _| {
            assert!(!root.create_modal_open());
            assert_eq!(root.add_role_error_for_test(), None);
        })
        .unwrap();
}

#[test]
fn remote_remove_role_confirms_handles_rejection_dismisses_on_disappear_and_updates_on_overview() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, cx, window, _, fake) = setup();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let ws = workspace(window, &visual);
    let mut data = overview();
    push_member(
        &mut data,
        0,
        "qa",
        "qa-member",
        runner_core::companion::Status::Working,
    );
    fake.set_overview(data.clone());
    visual.run_until_parked();

    // Activate member tab, open remove confirm, cancel -> no request.
    window
        .update(&mut visual, |_, window, cx| {
            ws.update(cx, |ws, cx| {
                ws.select_mission_session("member", window, cx);
                ws.request_remove_role("member", window, cx);
            });
        })
        .unwrap();
    visual.run_until_parked();
    ws.read_with(&visual, |ws, _| {
        assert_eq!(ws.active_tab, MissionTab::Session("member".into()));
        assert_eq!(ws.remove_confirm.as_deref(), Some("member"));
    });
    assert_eq!(fake.request_count("removeMissionRole"), 0);

    // If the member disappears remotely before confirm, dialog dismisses and resets focus.
    let mut without_member = data.clone();
    without_member.missions[0]
        .members
        .retain(|m| m.session != "member");
    without_member.sessions.remove("member");
    fake.set_overview(without_member);
    visual.run_until_parked();
    ws.read_with(&visual, |ws, _| {
        assert!(ws.remove_confirm.is_none());
        assert_eq!(ws.active_tab, MissionTab::Feed);
    });

    // Restore member, confirm removal with host rejection -> error banner, pending cleared, roster intact.
    fake.set_overview(data.clone());
    visual.run_until_parked();
    fake.queue_result(
        "removeMissionRole",
        Err(runner_core::protocol::ClientError::msg(
            "That role is being resumed or restarted; try again once it is up.",
        )),
    );
    window
        .update(&mut visual, |_, window, cx| {
            ws.update(cx, |ws, cx| {
                ws.select_mission_session("member", window, cx);
                ws.request_remove_role("member", window, cx);
                ws.remove_confirm = None;
                ws.remove_role("member", "coder", window, cx);
            });
        })
        .unwrap();
    wait_until(
        &mut visual,
        |visual| ws.read_with(visual, |ws, _| ws.slot_actions.is_empty()),
        "rejected removeMissionRole completion",
    );
    ws.read_with(&visual, |ws, _| {
        assert!(ws
            .error
            .as_ref()
            .is_some_and(|e| e.resolve().contains("resumed or restarted")));
        assert_eq!(ws.members().len(), 2);
    });

    // Successful removal dispatches removeMissionRole; pushed Overview removes card, closes active tab to Feed, and records removedHandles.
    window
        .update(&mut visual, |_, window, cx| {
            ws.update(cx, |ws, cx| {
                ws.error = None;
                ws.remove_role("member", "coder", window, cx);
            });
        })
        .unwrap();
    wait_until(
        &mut visual,
        |visual| {
            fake.request_count("removeMissionRole") == 2
                && ws.read_with(visual, |ws, _| ws.slot_actions.is_empty())
        },
        "successful removeMissionRole completion",
    );
    let requests = remote_test::action_trace(&fake, "removeMissionRole");
    assert_eq!(requests[1]["mission"], "remote-mission");
    assert_eq!(requests[1]["handle"], "coder");

    let mut after_remove = data;
    after_remove.missions[0]
        .members
        .retain(|m| m.session != "member");
    after_remove.missions[0]
        .removed_handles
        .push("coder".into());
    after_remove.sessions.remove("member");
    fake.set_overview(after_remove);
    visual.run_until_parked();
    assert!(visual.debug_bounds("MISSION_CARD member").is_none());
    assert!(visual.debug_bounds("MISSION_TAB member").is_none());
    assert!(visual.debug_bounds("MISSION_CARD qa-member").is_some());
    ws.read_with(&visual, |ws, _| {
        assert_eq!(ws.active_tab, MissionTab::Feed);
        assert!(!ws
            .mission_composer_roster()
            .iter()
            .any(|entry| entry.handle == "coder"));
    });
    window
        .update(&mut visual, |_, window, cx| {
            let ctx = ws.read(cx).add_role_context(window, cx).unwrap();
            assert!(ctx.removed.contains("coder"));
        })
        .unwrap();
}

#[test]
fn remote_add_role_submit_rejection_dismisses_if_mission_stopped_or_vanished_in_flight() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, cx, window, _, fake) = setup();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let ws = workspace(window, &visual);

    // 1. Stopped Overview arrives while submit is in-flight; held rejection dismisses the modal
    // and restores focus to the mission workspace instead of re-enabling Submit over a stopped mission.
    fake.queue_result("catalog", Ok(rich_host_catalog()));
    window
        .update(&mut visual, |root, window, cx| {
            let context = ws.read(cx).add_role_context(window, cx).unwrap();
            root.open_add_role_modal(context, window, cx);
        })
        .unwrap();
    wait_until(
        &mut visual,
        |visual| {
            window
                .read_with(visual, |root, _| !root.add_role_loading_catalog_for_test())
                .unwrap()
        },
        "catalog load for stopped-in-flight case",
    );
    let (gate, handle) = crate::test_support::StagedReplyGate::pair();
    fake.queue_result(
        "addMissionRole",
        Err(runner_core::protocol::ClientError::msg(
            "The mission is not running.",
        )),
    );
    window
        .update(&mut visual, |root, window, cx| {
            root.stage_add_role_submit_gate_for_test(gate);
            root.submit_add_role_for_test(window, cx);
        })
        .unwrap();
    wait_until(&mut visual, |_| handle.is_entered(), "submit reply staged");
    let mut stopped = overview();
    stopped.missions[0].state = runner_core::companion::MissionState::Stopped;
    fake.set_overview(stopped);
    visual.run_until_parked();
    ws.read_with(&visual, |ws, _| {
        assert_eq!(
            ws.mission_view().unwrap().status,
            runner_core::protocol::model::MissionStatus::Completed
        );
    });
    window
        .read_with(&visual, |root, _| {
            assert!(
                root.create_modal_open(),
                "submitting modal waits for its in-flight reply before dismissing"
            );
        })
        .unwrap();
    let applied = handle.release();
    wait_until(
        &mut visual,
        |_| applied.is_applied(),
        "stopped-in-flight rejection applied",
    );
    window
        .update(&mut visual, |root, window, cx| {
            assert!(!root.create_modal_open());
            assert!(
                ws.read(cx).root_focus.is_focused(window)
                    || ws
                        .read(cx)
                        .composer_input
                        .read(cx)
                        .focus_handle()
                        .is_focused(window)
            );
        })
        .unwrap();

    // 2. Mission vanishes from Overview while submit is in-flight (host switches active item to
    // remote-tab); held rejection dismisses the modal and restores focus instead of leaving it over the tab.
    fake.set_overview(overview());
    visual.run_until_parked();
    fake.queue_result("catalog", Ok(rich_host_catalog()));
    window
        .update(&mut visual, |root, window, cx| {
            let context = ws.read(cx).add_role_context(window, cx).unwrap();
            root.open_add_role_modal(context, window, cx);
        })
        .unwrap();
    wait_until(
        &mut visual,
        |visual| {
            window
                .read_with(visual, |root, _| !root.add_role_loading_catalog_for_test())
                .unwrap()
        },
        "catalog load for vanished-in-flight case",
    );
    let (gate, handle) = crate::test_support::StagedReplyGate::pair();
    fake.queue_result(
        "addMissionRole",
        Err(runner_core::protocol::ClientError::msg(
            "No mission has that id.",
        )),
    );
    window
        .update(&mut visual, |root, window, cx| {
            root.stage_add_role_submit_gate_for_test(gate);
            root.submit_add_role_for_test(window, cx);
        })
        .unwrap();
    wait_until(
        &mut visual,
        |_| handle.is_entered(),
        "vanished submit reply staged",
    );
    let mut vanished = overview();
    vanished.missions.clear();
    vanished.sessions.clear();
    fake.set_overview(vanished);
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| {
            assert!(matches!(
                root.remote_client.active_item(),
                Some(RemoteItem::Tab(id)) if id == "remote-tab"
            ));
            assert!(root.create_modal_open());
        })
        .unwrap();
    let applied = handle.release();
    wait_until(
        &mut visual,
        |_| applied.is_applied(),
        "vanished-in-flight rejection applied",
    );
    window
        .update(&mut visual, |root, window, cx| {
            assert!(!root.create_modal_open());
            assert!(window.focused(cx).is_some());
        })
        .unwrap();

    // 3. Same running mission rejects submit (e.g. handle taken) while Overview refreshes:
    // modal stays open with the inline error and re-enabled controls.
    fake.set_overview(overview());
    visual.run_until_parked();
    window
        .update(&mut visual, |root, _, cx| {
            root.select_remote_item(RemoteItem::Mission("remote-mission".into()), cx);
        })
        .unwrap();
    visual.run_until_parked();
    fake.queue_result("catalog", Ok(rich_host_catalog()));
    window
        .update(&mut visual, |root, window, cx| {
            let context = ws.read(cx).add_role_context(window, cx).unwrap();
            root.open_add_role_modal(context, window, cx);
        })
        .unwrap();
    wait_until(
        &mut visual,
        |visual| {
            window
                .read_with(visual, |root, _| !root.add_role_loading_catalog_for_test())
                .unwrap()
        },
        "catalog load for running rejection case",
    );
    let (gate, handle) = crate::test_support::StagedReplyGate::pair();
    fake.queue_result(
        "addMissionRole",
        Err(runner_core::protocol::ClientError::msg(
            "That handle is already in use in this mission.",
        )),
    );
    window
        .update(&mut visual, |root, window, cx| {
            root.stage_add_role_submit_gate_for_test(gate);
            root.submit_add_role_for_test(window, cx);
        })
        .unwrap();
    wait_until(
        &mut visual,
        |_| handle.is_entered(),
        "running rejection reply staged",
    );
    fake.set_overview(overview());
    visual.run_until_parked();
    let applied = handle.release();
    wait_until(
        &mut visual,
        |_| applied.is_applied(),
        "running rejection reply applied",
    );
    window
        .read_with(&visual, |root, cx| {
            assert!(root.create_modal_open());
            assert_eq!(
                root.add_role_error_for_test().as_deref(),
                Some("That handle is already in use in this mission.")
            );
            assert_eq!(
                root.add_role_controls_disabled_for_test(cx),
                Some((false, false, false))
            );
        })
        .unwrap();
}
