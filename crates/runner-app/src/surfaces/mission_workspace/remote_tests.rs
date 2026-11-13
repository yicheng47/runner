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
                            workspace.update_app_settings(cx, false, |settings| {
                                settings.mission_rail_open = rail_open;
                                true
                            });
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
                assert!((drawer_island.top() - mission.bottom() - px(6. * zoom)).abs() < px(0.5));
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
                    assert!((area.left() - px(8. * zoom)).abs() < px(0.5));
                } else {
                    let sidebar = visual.debug_bounds("APP_SIDEBAR").unwrap();
                    let gutter = visual.debug_bounds("SIDEBAR_RESIZE_GUTTER").unwrap();
                    assert!((area.left() - sidebar.right() - px(6. * zoom)).abs() < px(0.5));
                    assert!((gutter.center().x - area.left()).abs() < px(0.5));
                    assert!((gutter.size.width - px(6. * zoom)).abs() < px(0.5));
                }
                if rail_open {
                    let rail = visual.debug_bounds("MISSION_RAIL").unwrap();
                    let gutter = visual.debug_bounds("MISSION_RAIL_GUTTER").unwrap();
                    assert!((rail.left() - mission.right() - px(6. * zoom)).abs() < px(0.5));
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
    ] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
    for selector in [
        "MISSION_CARD_KEY member",
        "MISSION_RAIL_ADD_ROLE",
        "MISSION_META_STARTED",
    ] {
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
            assert!(root.terminal_close_confirm.is_none());
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
            workspace.update_app_settings(cx, false, |settings| {
                settings.mission_rail_width = 333.;
                settings.mission_rail_open = false;
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
        assert!(!workspace.settings(cx).mission_rail_open);
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
                    assert!(root.launcher_search_focused(window, cx))
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
fn remote_zero_role_mission_does_not_flash_no_roles_or_no_events_before_feed_loads() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for with_events in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
        remote_test::enable_missions(&fake);
        let mut data = overview();
        data.missions[0].members.clear();
        data.missions[0].asks.clear();
        data.sessions.clear();
        fake.set_overview(data.clone());
        let window = remote_test::add_window(&mut cx, &store, temp.path(), "remote-empty-roles");
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
        assert!(visual.debug_bounds("MISSION_RAIL_ADD_ROLE").is_none());
        workspace.read_with(&visual, |workspace, _| {
            assert_eq!(workspace.feed_item_count(), 1);
            assert_eq!(workspace.feed_empty_panel_min_height.get(), px(0.));
        });

        visual.run_until_parked();
        feed.read_with(&visual, |feed, _| {
            assert!(!feed.loading);
            assert!(feed.history_loaded);
        });
        assert!(visual.debug_bounds("MISSION_NO_ROLES").is_none());
        assert!(visual.debug_bounds("MISSION_RAIL_NO_ROLES").is_some());
        assert!(visual.debug_bounds("MISSION_RAIL_ADD_ROLE").is_none());
        workspace.read_with(&visual, |workspace, _| {
            assert_eq!(workspace.feed_empty_panel_min_height.get(), px(0.));
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
            assert!(visual.debug_bounds("MISSION_NO_ROLES").is_none());
            assert!(visual.debug_bounds("MISSION_RAIL_NO_ROLES").is_some());
            assert!(visual.debug_bounds("MISSION_RAIL_ADD_ROLE").is_none());
            workspace.read_with(&visual, |workspace, _| {
                assert_eq!(workspace.feed_empty_panel_min_height.get(), px(0.));
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
