use super::*;
use crate::surfaces::mission_composer::ComposerPost;
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
        .read_with(cx, |root, _| root.remote_mission_workspace().unwrap())
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
        assert!(workspace.members()[0].lead);
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
fn remote_feed_is_shared_pages_opaque_cursors_and_releases_only_after_last_window() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (temp, mut cx, first, store, fake) = setup();
    let ws1 = workspace(first, &cx);
    let feed = ws1.read_with(&cx, |workspace, _| {
        workspace.remote.as_ref().unwrap().feed.clone().unwrap()
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
            .remote
            .as_ref()
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
                workspace.remote_post(
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
                workspace.remote_post(
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
                .remote
                .as_ref()
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
        assert!(workspace.remote.as_ref().unwrap().drawer_expected.is_none());
    });
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.request_close_terminal_drawer_shell("started-session", window, cx)
        })
    });
    window
        .update(&mut visual, |root, window, cx| {
            assert!(root.terminal_close_confirm.is_some());
            root.confirm_terminal_close(window, cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(fake.request_count("close"), 1);
    workspace.read_with(&visual, |workspace, _| {
        assert_eq!(workspace.layout.drawer.shells(), &["started-session"]);
        assert!(workspace.remote.as_ref().unwrap().drawer_expected.is_some());
    });
    fake.set_overview(overview());
    visual.run_until_parked();
    workspace.read_with(&visual, |workspace, _| {
        assert!(workspace.layout.drawer.shells().is_empty());
        assert!(!workspace.layout.drawer.open());
        assert!(workspace.remote.as_ref().unwrap().drawer_expected.is_none());
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
        workspace.remote.as_ref().unwrap().feed.clone().unwrap()
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
        assert!(workspace.remote.as_ref().unwrap().feed.is_none());
    });
}

#[test]
fn absent_mission_capabilities_hide_controls_and_refuse_dispatch_at_the_boundary() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, mut cx, window, store, fake) = setup();
    let workspace = workspace(window, &cx);
    let target = workspace.read_with(&cx, |workspace, _| {
        workspace.remote.as_ref().unwrap().client.target.clone()
    });
    let access = crate::surfaces::front_page::RemoteAccess::new(target);
    access.set_features(vec!["feed".into(), "follow".into(), "unfollow".into()]);
    let client = store.read_with(&cx, |store, _| store.client.clone());
    workspace.update(&mut cx, |workspace, cx| {
        workspace.remote.as_mut().unwrap().client =
            crate::surfaces::front_page::FrontPageClient::remote(client, access);
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
                workspace.remote_post(
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
                workspace.remote_post(
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
            let focus = window
                .read_with(&cx, |root, _| root.root_focus.clone())
                .unwrap();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            visual.update(|window, _| assert!(focus.is_focused(window)));
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
            workspace.remote.as_ref().unwrap().feed.clone().unwrap()
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
            workspace.remote.as_ref().unwrap().feed.clone().unwrap()
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
fn existing_mission_shells_remain_navigable_with_independently_gated_exited_actions() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let mut widths = Vec::new();
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
        let actions = visual.debug_bounds("SESSION_ENDED_ACTIONS").unwrap();
        widths.push(f32::from(actions.size.width));
        if features.is_empty() {
            assert_eq!(actions.size.width, px(0.));
        } else if features.len() == 1 {
            assert!(actions.size.width > px(0.));
            visual.simulate_click(actions.center(), Default::default());
            visual.run_until_parked();
            assert_eq!(fake.request_count(features[0]), 1);
            let hidden = if features[0] == "resume" {
                "close"
            } else {
                "resume"
            };
            assert_eq!(fake.request_count(hidden), 0);
        }
        assert_eq!(fake.request_count("startShell"), 0);
        assert_eq!(fake.request_count("layout"), 0);
    }
    assert!(widths[3] > widths[1]);
    assert!(widths[3] > widths[2]);
}
