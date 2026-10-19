use super::*;
use runner_core::protocol::status::{Activity, HumanInteraction, TurnOutcome};

fn wait(reason: WaitReason, id: &str) -> AgentStatus {
    AgentStatus {
        lifecycle: Lifecycle::Running,
        observation: runner_core::protocol::status::AgentObservation {
            interactions: vec![HumanInteraction {
                id: id.into(),
                reason,
                owners: Vec::new(),
                since: 1,
            }],
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn status_edges_baseline_new_sessions_repeats_and_priority() {
    let empty = BTreeMap::new();
    for reason in [
        WaitReason::Approval,
        WaitReason::Answer,
        WaitReason::Unknown,
    ] {
        let next = BTreeMap::from([("chat".into(), wait(reason, "first"))]);
        assert!(detect_status_triggers(None, &next, &BTreeSet::new()).is_empty());
        assert_eq!(
            detect_status_triggers(Some(&empty), &next, &BTreeSet::new()),
            [("chat".into(), Reason::Input(reason))]
        );
        assert!(detect_status_triggers(Some(&next), &next, &BTreeSet::new()).is_empty());
        let newer = BTreeMap::from([("chat".into(), wait(reason, "second"))]);
        assert_eq!(
            detect_status_triggers(Some(&next), &newer, &BTreeSet::new()),
            [("chat".into(), Reason::Input(reason))]
        );
    }
    let cases = [
        (
            AgentStatus {
                unread_since: Some(1),
                ..Default::default()
            },
            Reason::Finished,
        ),
        (
            AgentStatus {
                failed_since: Some(1),
                unread_since: Some(1),
                ..Default::default()
            },
            Reason::Failed,
        ),
        (
            AgentStatus {
                lifecycle: Lifecycle::Error,
                exit_code: Some(2),
                failed_since: Some(1),
                unread_since: Some(1),
                ..Default::default()
            },
            Reason::Error(Some(2)),
        ),
    ];
    for (status, reason) in cases {
        let next = BTreeMap::from([("chat".into(), status)]);
        assert_eq!(
            detect_status_triggers(Some(&empty), &next, &BTreeSet::new()),
            [("chat".into(), reason)]
        );
        assert!(detect_status_triggers(Some(&next), &next, &BTreeSet::new()).is_empty());
    }
    let mut priority = wait(WaitReason::Approval, "new");
    priority.lifecycle = Lifecycle::Error;
    priority.failed_since = Some(1);
    priority.unread_since = Some(1);
    assert_eq!(
        detect_status_triggers(
            Some(&empty),
            &BTreeMap::from([("chat".into(), priority)]),
            &BTreeSet::new()
        ),
        [("chat".into(), Reason::Input(WaitReason::Approval))]
    );
}

#[test]
fn content_matches_each_direct_chat_trigger() {
    for (reason, text) in [
        (Reason::Input(WaitReason::Approval), "Approval needed"),
        (Reason::Input(WaitReason::Answer), "Answer needed"),
        (Reason::Input(WaitReason::Unknown), "Needs you"),
        (Reason::Finished, "Finished"),
        (Reason::Failed, "Response failed"),
        (Reason::Error(Some(9)), "Error · exit code 9"),
        (Reason::Error(None), "Error"),
    ] {
        let content = notification_content("chat", "Fix the test".into(), Some("runner"), reason);
        assert_eq!(content.tag.as_ref(), "chat");
        assert_eq!(content.title.as_ref(), "Fix the test");
        assert_eq!(content.body.as_ref(), format!("{text} · runner"));
        assert!(content.actions.is_empty());
        assert_eq!(
            notification_content("chat", "Title".into(), None, reason)
                .body
                .as_ref(),
            text
        );
    }
}

#[test]
fn switches_and_visibility_include_every_pane_of_the_active_tab() {
    let mut settings = NotificationSettings::default();
    settings.chats.input = false;
    settings.chats.finished = false;
    settings.chats.failed = false;
    for reason in [
        Reason::Input(WaitReason::Approval),
        Reason::Input(WaitReason::Answer),
        Reason::Input(WaitReason::Unknown),
        Reason::Finished,
        Reason::Failed,
        Reason::Error(None),
    ] {
        assert!(!reason.enabled(&settings, false));
    }
    let visible = Visible {
        chats: vec!["focused".into(), "unfocused".into()],
        ..Default::default()
    };
    assert!(target_visible(&Target::Chat("unfocused".into()), &visible));
    assert!(!target_visible(
        &Target::Chat("another-tab".into()),
        &visible
    ));
    assert!(!target_visible(
        &Target::Chat("focused".into()),
        &Visible::default()
    ));
}

#[test]
fn every_withdrawal_rule() {
    let settings = NotificationSettings::default();
    for (reason, status) in [
        (
            Reason::Input(WaitReason::Approval),
            wait(WaitReason::Approval, "wait"),
        ),
        (
            Reason::Finished,
            AgentStatus {
                unread_since: Some(1),
                ..Default::default()
            },
        ),
        (
            Reason::Failed,
            AgentStatus {
                failed_since: Some(1),
                ..Default::default()
            },
        ),
        (
            Reason::Error(None),
            AgentStatus {
                lifecycle: Lifecycle::Error,
                ..Default::default()
            },
        ),
    ] {
        let posted = BTreeMap::from([(
            "chat".into(),
            Posted {
                target: Target::Chat("chat".into()),
                reason,
            },
        )]);
        let statuses = BTreeMap::from([("chat".into(), status)]);
        assert!(stale_tags(
            &posted,
            &statuses,
            &BTreeMap::new(),
            &settings,
            &Visible::default(),
            |_| true
        )
        .is_empty());
        assert_eq!(
            stale_tags(
                &posted,
                &BTreeMap::new(),
                &BTreeMap::new(),
                &settings,
                &Visible::default(),
                |_| true
            ),
            ["chat"]
        );
        assert_eq!(
            stale_tags(
                &posted,
                &BTreeMap::from([("chat".into(), AgentStatus::default())]),
                &BTreeMap::new(),
                &settings,
                &Visible::default(),
                |_| true
            ),
            ["chat"]
        );
        assert_eq!(
            stale_tags(
                &posted,
                &statuses,
                &BTreeMap::new(),
                &settings,
                &Visible::default(),
                |_| { false }
            ),
            ["chat"]
        );
        assert_eq!(
            stale_tags(
                &posted,
                &statuses,
                &BTreeMap::new(),
                &settings,
                &Visible {
                    chats: vec!["chat".into()],
                    ..Default::default()
                },
                |_| true
            ),
            ["chat"]
        );
        let off = NotificationSettings {
            chats: crate::app_settings::ChatNotifications {
                input: false,
                finished: false,
                failed: false,
            },
            ..Default::default()
        };
        assert_eq!(
            stale_tags(
                &posted,
                &statuses,
                &BTreeMap::new(),
                &off,
                &Visible::default(),
                |_| true
            ),
            ["chat"]
        );
    }
}

#[cfg(unix)]
#[test]
fn controller_posts_once_replaces_clears_archives_switches_and_quits() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store)
    });
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        install(cx);
    });
    assert!(cx.shown_system_notifications().is_empty());
    let change = |cx: &mut gpui::TestAppContext, state: AgentStatus| {
        store.update(cx, |store, cx| {
            store
                .session_statuses
                .insert("direct-agent".into(), state.clone());
            store.session_statuses.insert("busy-shell".into(), state);
            store.revisions.activity += 1;
            cx.notify();
        });
        cx.update(|_| ());
    };
    change(&mut cx, wait(WaitReason::Approval, "first"));
    assert_eq!(cx.shown_system_notifications().len(), 1);
    change(&mut cx, wait(WaitReason::Approval, "first"));
    assert_eq!(cx.shown_system_notifications().len(), 1);
    change(&mut cx, wait(WaitReason::Answer, "second"));
    assert_eq!(cx.shown_system_notifications().len(), 2);
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    change(&mut cx, AgentStatus::default());
    cx.update(|_| ());
    assert!(cx.delivered_system_notifications().is_empty());
    change(&mut cx, wait(WaitReason::Unknown, "third"));
    store.update(&mut cx, |store, cx| {
        store.settings.notifications.chats.input = false;
        store.revisions.settings += 1;
        cx.notify();
    });
    cx.update(|_| ());
    assert!(cx.delivered_system_notifications().is_empty());
    change(&mut cx, wait(WaitReason::Unknown, "fourth"));
    assert_eq!(cx.shown_system_notifications().len(), 3);
    store.update(&mut cx, |store, cx| {
        store.settings.notifications.chats.input = true;
        store.revisions.settings += 1;
        cx.notify();
    });
    change(&mut cx, wait(WaitReason::Unknown, "fifth"));
    cx.update(dismiss_all);
    cx.update(|_| ());
    assert!(cx.delivered_system_notifications().is_empty());
    change(&mut cx, wait(WaitReason::Unknown, "sixth"));
    store.update(&mut cx, |store, cx| {
        store.session_details.remove("direct-agent");
        store.revisions.sessions += 1;
        cx.notify();
    });
    cx.update(|_| ());
    assert!(cx.delivered_system_notifications().is_empty());
}

#[cfg(unix)]
#[test]
fn click_activates_the_window_tab_and_pane_and_view_changes_defer() {
    use gpui::{SystemNotificationResponse, VisualTestContext};
    use runner_app::pane_layout::{PaneLayout, TabSet};
    use runner_core::protocol::node::{NodeRow, NodeType};

    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store)
    });
    let rows = ["direct-agent", "busy-shell"].map(|id| NodeRow {
        id: if id == "direct-agent" {
            "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        } else {
            "01ARZ3NDEKTSV4RRFFQ69G5FAW"
        }
        .into(),
        parent_id: None,
        position: 0,
        node_type: NodeType::Tab,
        name: None,
        ref_id: None,
        layout: Some(PaneLayout::single(Some(id), &[]).serialize().unwrap()),
        pinned_position: None,
        last_completed_at: None,
        last_viewed_at: None,
        created_at: String::new(),
    });
    let client = store.read_with(&cx, |store, _| store.client.clone());
    for row in &rows {
        client
            .node_tab_upsert(runner_core::protocol::node::NodeTabUpsertInput {
                id: row.id.clone(),
                parent_id: None,
                name: String::new(),
                layout: row.layout.clone().unwrap(),
            })
            .unwrap();
    }
    store.update(&mut cx, |store, _| store.nodes = rows.to_vec());
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        install(cx);
    });
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "main".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.tabs = TabSet::from_rows(&rows);
        root.tabs.activate(&rows[1].id);
        root.route = AppRoute::Chat;
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.update(|window, _| window.activate_window());
    cx.run_until_parked();
    visual.deactivate_window();
    cx.run_until_parked();
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store
                .session_statuses
                .insert("direct-agent".into(), wait(WaitReason::Approval, "new"));
            store.revisions.activity += 1;
            cx.notify();
        })
    });
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    cx.simulate_system_notification_response(SystemNotificationResponse {
        tag: "direct-agent".into(),
        action_id: None,
    });
    cx.run_until_parked();
    host.read_with(&cx, |root, _| {
        assert_eq!(root.route, AppRoute::Chat);
        assert_eq!(root.tabs.active_tab_id(), Some(rows[0].id.as_str()));
        assert_eq!(
            root.tabs.active().unwrap().focused_session_id(),
            Some("direct-agent")
        );
    })
    .unwrap();
    cx.update(|_| ());
    assert!(cx.delivered_system_notifications().is_empty());
    host.update(&mut visual, |root, _, cx| root.report_current_subjects(cx))
        .unwrap();
    cx.update(|_| ());
    assert!(cx.delivered_system_notifications().is_empty());
    host.update(&mut visual, |root, _, _| {
        root.tabs.activate(&rows[1].id);
    })
    .unwrap();
    let secondary = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "secondary".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.tabs = TabSet::from_rows(&rows);
        root.tabs.activate(&rows[0].id);
        root.route = AppRoute::Chat;
        root
    });
    visual.update(|window, _| window.activate_window());
    cx.run_until_parked();
    visual.deactivate_window();
    cx.run_until_parked();
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.session_statuses.insert(
                "direct-agent".into(),
                wait(WaitReason::Approval, "second-window"),
            );
            store.revisions.activity += 1;
            cx.notify();
        })
    });
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    cx.simulate_system_notification_response(SystemNotificationResponse {
        tag: "direct-agent".into(),
        action_id: None,
    });
    cx.run_until_parked();
    secondary
        .read_with(&cx, |root, _| {
            assert_eq!(root.tabs.active_tab_id(), Some(rows[0].id.as_str()))
        })
        .unwrap();
    host.read_with(&cx, |root, _| {
        assert_eq!(root.tabs.active_tab_id(), Some(rows[1].id.as_str()))
    })
    .unwrap();
    assert_eq!(
        cx.read(|cx| cx.active_window().unwrap().window_id()),
        secondary.window_id()
    );
}

#[test]
fn mission_idle_edges_switches_content_visibility_and_withdrawal() {
    let slots = BTreeSet::from(["slot".into()]);
    let mut working = AgentStatus::default();
    working.observation.activity = Activity::Working;
    let mut finished = working.clone();
    finished.observation.activity = Activity::Idle;
    finished.observation.outcome = Some(TurnOutcome::Completed);
    finished.unread_since = Some(1);
    assert!(detect_status_triggers(
        Some(&BTreeMap::from([("slot".into(), working)])),
        &BTreeMap::from([("slot".into(), finished)]),
        &slots
    )
    .is_empty());
    let busy = BTreeMap::from([("mission".into(), Some(MissionActivityState::Busy))]);
    let idle = BTreeMap::from([("mission".into(), Some(MissionActivityState::Idle))]);
    assert_eq!(detect_mission_idle(&busy, &idle), ["mission"]);
    assert!(detect_mission_idle(&idle, &idle).is_empty());
    assert!(detect_mission_idle(&BTreeMap::new(), &idle).is_empty());
    assert!(detect_mission_idle(&BTreeMap::from([("mission".into(), None)]), &idle).is_empty());
    assert!(detect_mission_idle(&busy, &busy).is_empty());
    assert!(detect_mission_idle(&busy, &BTreeMap::from([("mission".into(), None)])).is_empty());
    let mut settings = NotificationSettings::default();
    for reason in [
        Reason::Input(WaitReason::Approval),
        Reason::Failed,
        Reason::Error(None),
        Reason::Question,
        Reason::MissionIdle,
        Reason::Finished,
    ] {
        assert!(!reason.enabled(&settings, true));
    }
    settings.missions.idle = true;
    assert!(Reason::MissionIdle.enabled(&settings, true));
    assert!(!Reason::Finished.enabled(&settings, true));
    settings.missions.input = false;
    settings.missions.failed = false;
    settings.missions.question = false;
    settings.missions.idle = false;
    for reason in [
        Reason::Input(WaitReason::Answer),
        Reason::Input(WaitReason::Unknown),
        Reason::Failed,
        Reason::Error(None),
        Reason::Question,
        Reason::MissionIdle,
    ] {
        assert!(!reason.enabled(&settings, true));
    }
    for reason in [
        Reason::Input(WaitReason::Approval),
        Reason::Input(WaitReason::Answer),
        Reason::Input(WaitReason::Unknown),
        Reason::Failed,
        Reason::Error(Some(3)),
    ] {
        let content = mission_content("slot", "coder", "Ship Runner", reason);
        assert_eq!(content.title.as_ref(), "@coder · Ship Runner");
        assert_eq!(content.body.as_ref(), reason.label());
        assert!(content.actions.is_empty());
    }
    let content = mission_idle_content("mission", "Ship Runner");
    assert_eq!(content.tag.as_ref(), "mission:mission:idle");
    assert_eq!(content.title.as_ref(), "Ship Runner");
    assert_eq!(content.body.as_ref(), "Every crew member is idle");
    assert!(content.actions.is_empty());
    assert_eq!(
        question_content("question:q", Some("lead"), "Ship Runner", "Ship now?")
            .title
            .as_ref(),
        "@lead asked you · Ship Runner"
    );
    assert_eq!(
        question_content("question:q", None, "Ship Runner", "Ship now?")
            .title
            .as_ref(),
        "A crew member asked you · Ship Runner"
    );
    assert_eq!(
        question_content("question:q", None, "Ship Runner", "Ship now?")
            .body
            .as_ref(),
        "Ship now?"
    );
    let target = Target::MissionIdle {
        mission: "mission".into(),
    };
    let visible = Visible {
        app_active: true,
        mission: Some(("mission".into(), MissionTab::Session("slot".into()))),
        ..Default::default()
    };
    assert!(target_visible(&target, &visible));
    let posted = BTreeMap::from([(
        "mission:mission:idle".into(),
        Posted {
            target,
            reason: Reason::MissionIdle,
        },
    )]);
    settings.missions.idle = true;
    assert!(stale_tags(
        &posted,
        &BTreeMap::new(),
        &idle,
        &settings,
        &Visible::default(),
        |_| true
    )
    .is_empty());
    for missions in [
        busy,
        BTreeMap::new(),
        BTreeMap::from([("mission".into(), None)]),
    ] {
        assert_eq!(
            stale_tags(
                &posted,
                &BTreeMap::new(),
                &missions,
                &settings,
                &Visible::default(),
                |_| true
            ),
            ["mission:mission:idle"]
        );
    }
    assert_eq!(
        stale_tags(
            &posted,
            &BTreeMap::new(),
            &idle,
            &settings,
            &visible,
            |_| true
        ),
        ["mission:mission:idle"]
    );
    assert_eq!(
        stale_tags(
            &posted,
            &BTreeMap::new(),
            &idle,
            &settings,
            &Visible::default(),
            |_| false
        ),
        ["mission:mission:idle"]
    );
    settings.missions.idle = false;
    assert_eq!(
        stale_tags(
            &posted,
            &BTreeMap::new(),
            &idle,
            &settings,
            &Visible::default(),
            |_| true
        ),
        ["mission:mission:idle"]
    );
}

#[cfg(unix)]
#[test]
fn controller_handles_slots_and_live_question_events() {
    use runner_core::protocol::model::{EventKind, SignalType};
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store);
        store.settings.notifications.missions = crate::app_settings::MissionNotifications {
            question: true,
            input: true,
            idle: true,
            failed: true,
        };
    });
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        install(cx);
    });
    let change = |cx: &mut gpui::TestAppContext, state: AgentStatus| {
        cx.update(|cx| {
            store.update(cx, |store, cx| {
                store
                    .session_statuses
                    .insert("mission-agent".into(), state.clone());
                store.session_statuses.insert("mission-shell".into(), state);
                store.revisions.activity += 1;
                cx.notify();
            })
        })
    };
    change(&mut cx, wait(WaitReason::Approval, "slot-wait"));
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    assert_eq!(
        cx.delivered_system_notifications()[0].title.as_ref(),
        "@agent · Test"
    );
    let working = AgentStatus {
        observation: runner_core::protocol::status::AgentObservation {
            activity: Activity::Working,
            ..Default::default()
        },
        ..Default::default()
    };
    let finished = AgentStatus {
        observation: runner_core::protocol::status::AgentObservation {
            activity: Activity::Idle,
            outcome: Some(TurnOutcome::Completed),
            ..Default::default()
        },
        ..Default::default()
    };
    change(&mut cx, working.clone());
    assert!(cx.delivered_system_notifications().is_empty());
    change(&mut cx, finished.clone());
    assert_eq!(cx.shown_system_notifications().len(), 1);
    change(&mut cx, working.clone());
    change(&mut cx, finished);
    assert!(cx.delivered_system_notifications().is_empty());
    change(&mut cx, working);
    let events = store.read_with(&cx, |store, _| store.test_core.events.clone());
    let event = |id: &str, kind: &str, payload| Event {
        id: id.into(),
        ts: chrono::Utc::now(),
        crew_id: Some("test-crew".into()),
        mission_id: "test-mission".into(),
        kind: EventKind::Signal,
        from: "lead".into(),
        to: Default::default(),
        signal_type: Some(SignalType::new(kind)),
        payload,
    };
    let emit = |cx: &gpui::TestAppContext, event: Event| {
        events.emit(
            "event/appended",
            &serde_json::json!({"mission_id":"test-mission", "event":event, "next_offset":1}),
        );
        cx.run_until_parked();
    };
    emit(
        &cx,
        event("ask", "ask_human", serde_json::json!({"prompt":"Ship?"})),
    );
    emit(
        &cx,
        event(
            "question",
            "human_question",
            serde_json::json!({"triggered_by":"ask", "prompt":"Ship?"}),
        ),
    );
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    assert_eq!(
        cx.delivered_system_notifications()[0].title.as_ref(),
        "@lead asked you · Test"
    );
    assert_eq!(
        cx.delivered_system_notifications()[0].tag.as_ref(),
        "question:question"
    );
    emit(
        &cx,
        event(
            "answer",
            "human_response",
            serde_json::json!({"question_id":"question"}),
        ),
    );
    assert!(cx.delivered_system_notifications().is_empty());
    emit(
        &cx,
        event(
            "fallback",
            "human_question",
            serde_json::json!({"triggered_by":"old-ask", "prompt":"Another?"}),
        ),
    );
    assert_eq!(
        cx.delivered_system_notifications()[0].title.as_ref(),
        "A crew member asked you · Test"
    );
    cx.update(dismiss_all);
    assert!(cx.delivered_system_notifications().is_empty());
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.settings.notifications.missions.question = false;
            store.revisions.settings += 1;
            cx.notify();
        })
    });
    emit(
        &cx,
        event(
            "off",
            "human_question",
            serde_json::json!({"prompt":"Muted?"}),
        ),
    );
    assert!(cx.delivered_system_notifications().is_empty());
}

#[cfg(unix)]
#[test]
fn questions_wait_for_mission_metadata_and_responses_cancel_pending_posts() {
    use runner_core::protocol::model::{EventKind, SignalType};
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store);
        store.settings.notifications.missions = crate::app_settings::MissionNotifications {
            question: true,
            input: true,
            idle: true,
            failed: true,
        };
    });
    let missions = store.read_with(&cx, |store, _| store.missions.clone());
    store.update(&mut cx, |store, _| store.missions.clear());
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        install(cx);
    });
    let event = |id: &str, kind: &str, payload| Event {
        id: id.into(),
        ts: chrono::Utc::now(),
        crew_id: Some("test-crew".into()),
        mission_id: "test-mission".into(),
        kind: EventKind::Signal,
        from: "lead".into(),
        to: Default::default(),
        signal_type: Some(SignalType::new(kind)),
        payload,
    };
    cx.update(|cx| {
        let controller = cx.global::<GlobalNotifications>().0.clone();
        controller.update(cx, |this, cx| {
            this.event(
                event(
                    "cancelled",
                    "human_question",
                    serde_json::json!({"prompt":"Cancelled?"}),
                ),
                cx,
            );
            this.event(
                event(
                    "response",
                    "human_response",
                    serde_json::json!({"question_id":"cancelled"}),
                ),
                cx,
            );
            this.event(
                event(
                    "pending",
                    "human_question",
                    serde_json::json!({"prompt":"Still open?"}),
                ),
                cx,
            );
        });
    });
    assert!(cx.delivered_system_notifications().is_empty());
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.missions = missions;
            store.revisions.missions += 1;
            cx.notify();
        })
    });
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    assert_eq!(
        cx.delivered_system_notifications()[0].tag.as_ref(),
        "question:pending"
    );
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.settings.notifications.missions.question = false;
            store.revisions.settings += 1;
            cx.notify();
        })
    });
    assert!(cx.delivered_system_notifications().is_empty());
}

#[cfg(unix)]
#[test]
fn controller_accepts_an_empty_loaded_store_as_the_baseline() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    assert!(store.read_with(&cx, |store, _| store.session_statuses.is_empty()));
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        install(cx);
    });
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            store.session_statuses.insert(
                "direct-agent".into(),
                wait(WaitReason::Approval, "first-session"),
            );
            store.revisions.activity += 1;
            cx.notify();
        })
    });
    assert_eq!(cx.shown_system_notifications().len(), 1);
    assert_eq!(
        cx.shown_system_notifications()[0].tag.as_ref(),
        "direct-agent"
    );
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.revisions.activity += 1;
            cx.notify();
        })
    });
    assert_eq!(cx.shown_system_notifications().len(), 1);
}

#[cfg(unix)]
#[test]
fn foreground_routes_suppress_all_kinds_without_background_replay() {
    use gpui::VisualTestContext;
    use runner_core::protocol::model::{EventKind, SignalType};

    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store);
        store.settings.notifications.missions = crate::app_settings::MissionNotifications {
            question: true,
            input: true,
            idle: true,
            failed: true,
        };
    });
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        install(cx);
    });
    let host = cx.add_window(|window, cx| {
        NativeRoot::new(
            "main".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    let secondary = cx.add_window(|window, cx| {
        NativeRoot::new(
            "secondary".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    let mut visual = VisualTestContext::from_window(secondary.into(), &cx);
    visual.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let change = |cx: &mut gpui::TestAppContext, state: AgentStatus, activity| {
        cx.update(|cx| {
            store.update(cx, |store, cx| {
                store
                    .session_statuses
                    .insert("direct-agent".into(), state.clone());
                store.session_statuses.insert("mission-agent".into(), state);
                store.missions[0].activity = Some(activity);
                store.revisions.activity += 1;
                cx.notify();
            })
        });
    };
    let question = |cx: &mut gpui::TestAppContext, id: &str| {
        cx.update(|cx| {
            let controller = cx.global::<GlobalNotifications>().0.clone();
            controller.update(cx, |this, cx| {
                this.event(
                    Event {
                        id: id.into(),
                        ts: chrono::Utc::now(),
                        crew_id: Some("test-crew".into()),
                        mission_id: "test-mission".into(),
                        kind: EventKind::Signal,
                        from: "lead".into(),
                        to: Default::default(),
                        signal_type: Some(SignalType::new("human_question")),
                        payload: serde_json::json!({"prompt":"Continue?"}),
                    },
                    cx,
                )
            });
        });
    };
    for (index, route) in [
        AppRoute::Chat,
        AppRoute::Settings,
        AppRoute::Mission("other".into()),
    ]
    .into_iter()
    .enumerate()
    {
        secondary
            .update(&mut visual, |root, _, _| root.route = route)
            .unwrap();
        change(&mut cx, AgentStatus::default(), MissionActivityState::Busy);
        change(
            &mut cx,
            wait(WaitReason::Approval, &format!("foreground-{index}")),
            MissionActivityState::Idle,
        );
        question(&mut cx, &format!("foreground-question-{index}"));
        change(
            &mut cx,
            AgentStatus {
                lifecycle: Lifecycle::Error,
                ..Default::default()
            },
            MissionActivityState::Idle,
        );
        change(
            &mut cx,
            AgentStatus {
                failed_since: Some(1),
                ..Default::default()
            },
            MissionActivityState::Idle,
        );
        change(
            &mut cx,
            AgentStatus {
                unread_since: Some(1),
                ..Default::default()
            },
            MissionActivityState::Idle,
        );
    }
    assert!(cx.shown_system_notifications().is_empty());
    assert_eq!(
        cx.read(|cx| cx.active_window().unwrap().window_id()),
        secondary.window_id()
    );
    host.read_with(&cx, |root, _| assert_eq!(root.route, AppRoute::Chat))
        .unwrap();
    visual.deactivate_window();
    cx.run_until_parked();
    cx.update(view_changed);
    assert!(cx.shown_system_notifications().is_empty());
    change(&mut cx, AgentStatus::default(), MissionActivityState::Busy);
    change(
        &mut cx,
        wait(WaitReason::Answer, "fresh-background"),
        MissionActivityState::Idle,
    );
    question(&mut cx, "fresh-background-question");
    let tags = cx
        .delivered_system_notifications()
        .into_iter()
        .map(|n| n.tag.to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        tags,
        BTreeSet::from([
            "direct-agent".into(),
            "mission-agent".into(),
            "mission:test-mission:idle".into(),
            "question:fresh-background-question".into(),
        ])
    );
}

#[cfg(unix)]
#[test]
fn chat_finished_defaults_on_and_reenabling_does_not_replay() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store)
    });
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        install(cx);
    });
    let change = |cx: &mut gpui::TestAppContext, unread| {
        cx.update(|cx| {
            store.update(cx, |store, cx| {
                store.session_statuses.insert(
                    "direct-agent".into(),
                    AgentStatus {
                        unread_since: unread,
                        ..Default::default()
                    },
                );
                store.revisions.activity += 1;
                cx.notify();
            })
        });
    };
    change(&mut cx, Some(1));
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    assert_eq!(
        cx.delivered_system_notifications()[0].body.as_ref(),
        "Finished"
    );
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.settings.notifications.chats.finished = false;
            store.revisions.settings += 1;
            cx.notify();
        })
    });
    assert!(cx.delivered_system_notifications().is_empty());
    change(&mut cx, None);
    change(&mut cx, Some(2));
    assert_eq!(cx.shown_system_notifications().len(), 1);
    assert!(cx.delivered_system_notifications().is_empty());
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.settings.notifications.chats.finished = true;
            store.revisions.settings += 1;
            cx.notify();
        })
    });
    change(&mut cx, Some(2));
    assert_eq!(cx.shown_system_notifications().len(), 1);
    assert!(cx.delivered_system_notifications().is_empty());
    change(&mut cx, None);
    change(&mut cx, Some(3));
    assert_eq!(cx.shown_system_notifications().len(), 2);
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    assert_eq!(
        cx.delivered_system_notifications()[0].body.as_ref(),
        "Finished"
    );
}

#[cfg(unix)]
#[test]
fn mission_switches_are_independent_and_reenabling_never_replays() {
    use crate::app_settings::MissionNotifications;
    use runner_core::protocol::model::{EventKind, SignalType};

    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store);
        store.missions[0].activity = Some(MissionActivityState::Idle);
    });
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        install(cx);
    });
    let change = |cx: &mut gpui::TestAppContext, state: AgentStatus, activity| {
        cx.update(|cx| {
            store.update(cx, |store, cx| {
                store.session_statuses.insert("mission-agent".into(), state);
                store.missions[0].activity = activity;
                store.revisions.activity += 1;
                cx.notify();
            })
        });
    };
    let settings = |cx: &mut gpui::TestAppContext, choices| {
        cx.update(|cx| {
            store.update(cx, |store, cx| {
                store.settings.notifications.missions = choices;
                store.revisions.settings += 1;
                cx.notify();
            })
        });
    };
    let question = |cx: &mut gpui::TestAppContext, id: &str| {
        cx.update(|cx| {
            let controller = cx.global::<GlobalNotifications>().0.clone();
            controller.update(cx, |this, cx| {
                this.event(
                    Event {
                        id: id.into(),
                        ts: chrono::Utc::now(),
                        crew_id: Some("test-crew".into()),
                        mission_id: "test-mission".into(),
                        kind: EventKind::Signal,
                        from: "lead".into(),
                        to: Default::default(),
                        signal_type: Some(SignalType::new("human_question")),
                        payload: serde_json::json!({"prompt":"Ship?"}),
                    },
                    cx,
                )
            });
        });
    };
    change(
        &mut cx,
        AgentStatus::default(),
        Some(MissionActivityState::Busy),
    );
    change(
        &mut cx,
        wait(WaitReason::Approval, "muted"),
        Some(MissionActivityState::Idle),
    );
    question(&mut cx, "muted");
    change(
        &mut cx,
        AgentStatus {
            lifecycle: Lifecycle::Error,
            ..Default::default()
        },
        Some(MissionActivityState::Idle),
    );
    change(
        &mut cx,
        AgentStatus {
            failed_since: Some(1),
            ..Default::default()
        },
        Some(MissionActivityState::Idle),
    );
    assert!(cx.shown_system_notifications().is_empty());
    for kind in ["input", "question", "idle", "failed"] {
        change(
            &mut cx,
            AgentStatus::default(),
            Some(MissionActivityState::Busy),
        );
        let choices = MissionNotifications {
            question: kind == "question",
            input: kind == "input",
            idle: kind == "idle",
            failed: kind == "failed",
        };
        settings(&mut cx, choices.clone());
        let before = cx.shown_system_notifications().len();
        let state = if kind == "failed" {
            AgentStatus {
                lifecycle: Lifecycle::Error,
                ..Default::default()
            }
        } else {
            wait(WaitReason::Answer, kind)
        };
        change(&mut cx, state.clone(), Some(MissionActivityState::Idle));
        question(&mut cx, kind);
        assert_eq!(cx.shown_system_notifications().len(), before + 1);
        let notification = &cx.delivered_system_notifications()[0];
        let expected_tag = match kind {
            "question" => "question:question",
            "idle" => "mission:test-mission:idle",
            _ => "mission-agent",
        };
        assert_eq!(notification.tag.as_ref(), expected_tag);
        settings(&mut cx, MissionNotifications::default());
        assert!(cx.delivered_system_notifications().is_empty());
        question(&mut cx, &format!("muted-{kind}"));
        settings(&mut cx, choices);
        change(&mut cx, state, Some(MissionActivityState::Idle));
        assert_eq!(cx.shown_system_notifications().len(), before + 1);
        assert!(cx.delivered_system_notifications().is_empty());
        settings(&mut cx, MissionNotifications::default());
    }
    settings(
        &mut cx,
        MissionNotifications {
            failed: true,
            ..Default::default()
        },
    );
    change(
        &mut cx,
        AgentStatus::default(),
        Some(MissionActivityState::Busy),
    );
    change(
        &mut cx,
        AgentStatus {
            failed_since: Some(2),
            ..Default::default()
        },
        Some(MissionActivityState::Busy),
    );
    assert_eq!(
        cx.delivered_system_notifications()[0].body.as_ref(),
        "Response failed"
    );
    settings(
        &mut cx,
        MissionNotifications {
            idle: true,
            ..Default::default()
        },
    );
    assert!(cx.delivered_system_notifications().is_empty());
    change(&mut cx, AgentStatus::default(), None);
    change(
        &mut cx,
        AgentStatus::default(),
        Some(MissionActivityState::Idle),
    );
    assert!(cx.delivered_system_notifications().is_empty());
    change(
        &mut cx,
        AgentStatus::default(),
        Some(MissionActivityState::Busy),
    );
    change(
        &mut cx,
        AgentStatus::default(),
        Some(MissionActivityState::Idle),
    );
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    let before = cx.shown_system_notifications().len();
    change(
        &mut cx,
        AgentStatus::default(),
        Some(MissionActivityState::Idle),
    );
    assert_eq!(cx.shown_system_notifications().len(), before);
    change(
        &mut cx,
        AgentStatus::default(),
        Some(MissionActivityState::Busy),
    );
    assert!(cx.delivered_system_notifications().is_empty());
    settings(
        &mut cx,
        MissionNotifications {
            input: true,
            question: true,
            idle: true,
            failed: false,
        },
    );
    change(
        &mut cx,
        wait(WaitReason::Approval, "three-kinds"),
        Some(MissionActivityState::Idle),
    );
    question(&mut cx, "three-kinds");
    assert_eq!(cx.delivered_system_notifications().len(), 3);
    settings(
        &mut cx,
        MissionNotifications {
            question: true,
            idle: true,
            ..Default::default()
        },
    );
    assert_eq!(cx.delivered_system_notifications().len(), 2);
    settings(
        &mut cx,
        MissionNotifications {
            idle: true,
            ..Default::default()
        },
    );
    assert_eq!(
        cx.delivered_system_notifications()[0].tag.as_ref(),
        "mission:test-mission:idle"
    );
    settings(&mut cx, MissionNotifications::default());
    assert!(cx.delivered_system_notifications().is_empty());
}

#[cfg(unix)]
#[test]
fn joined_mission_slot_uses_current_handle_and_counts_in_idle_projection() {
    use runner_core::{EventDraft, EventLog, SignalType};

    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store)
    });
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    let log = EventLog::open(&runner_core::event_log::mission_dir(
        &core.app_data_dir,
        Some("test-crew"),
        "test-mission",
    ))
    .unwrap();
    for handle in ["agent", "shell"] {
        log.append(EventDraft::signal(
            Some("test-crew"),
            "test-mission",
            handle,
            SignalType::new("session_status"),
            serde_json::json!({"state":"idle"}),
        ))
        .unwrap();
    }
    store.update(&mut cx, |store, _| {
        store.settings.notifications.missions.input = true;
        store.settings.notifications.missions.idle = true;
        store.missions = store.client.mission_list_summary_impl(None).unwrap();
        assert_eq!(store.missions[0].activity, Some(MissionActivityState::Idle));
    });
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        install(cx);
    });
    assert!(cx.shown_system_notifications().is_empty());
    core.db.get().unwrap().execute_batch(
        "INSERT INTO roles (id, handle, display_name, runtime, command, created_at, updated_at)
         VALUES ('joined-role', 'buddy', 'Buddy', 'codex', 'codex', '2026-10-13T00:00:00Z', '2026-10-13T00:00:00Z');
         INSERT INTO slots (id, crew_id, role_id, slot_handle, position, lead, added_at)
         VALUES ('joined-slot', 'test-crew', 'joined-role', 'buddy', 3, 0, '2026-10-13T00:00:00Z');
         INSERT INTO sessions (id, role_id, slot_id, mission_id, status, agent_runtime, agent_command)
         VALUES ('joined-agent', 'joined-role', 'joined-slot', 'test-mission', 'running', 'codex', 'codex');"
    ).unwrap();
    {
        let conn = core.db.get().unwrap();
        let mission = runner_daemon::ops::mission::get(&conn, "test-mission").unwrap();
        let mut roster =
            runner_daemon::ops::mission::roster::load(&conn, &core.app_data_dir, &mission)
                .unwrap()
                .file;
        let joined = runner_daemon::repo::slot::get(&conn, "joined-slot")
            .unwrap()
            .unwrap();
        roster
            .slots
            .push(runner_core::mission_roster::RosterSlot::from(&joined));
        runner_daemon::ops::mission::roster::write(
            &runner_core::event_log::mission_dir(
                &core.app_data_dir,
                Some("test-crew"),
                "test-mission",
            ),
            &roster,
        )
        .unwrap();
    }
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.missions = store.client.mission_list_summary_impl(None).unwrap();
            assert_eq!(store.missions[0].activity, Some(MissionActivityState::Busy));
            store.session_statuses.insert(
                "joined-agent".into(),
                wait(WaitReason::Approval, "joined-wait"),
            );
            store.revisions.missions += 1;
            cx.notify();
        })
    });
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    assert_eq!(
        cx.delivered_system_notifications()[0].tag.as_ref(),
        "joined-agent"
    );
    assert_eq!(
        cx.delivered_system_notifications()[0].title.as_ref(),
        "@buddy · Test"
    );
    log.append(EventDraft::signal(
        Some("test-crew"),
        "test-mission",
        "buddy",
        SignalType::new("session_status"),
        serde_json::json!({"state":"idle"}),
    ))
    .unwrap();
    let refresh = |cx: &mut gpui::TestAppContext| {
        cx.update(|cx| {
            store.update(cx, |store, cx| {
                store.missions = store.client.mission_list_summary_impl(None).unwrap();
                assert_eq!(store.missions[0].activity, Some(MissionActivityState::Idle));
                store.session_statuses.insert(
                    "joined-agent".into(),
                    AgentStatus {
                        unread_since: Some(1),
                        ..Default::default()
                    },
                );
                store.revisions.missions += 1;
                cx.notify();
            })
        });
    };
    refresh(&mut cx);
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    assert_eq!(
        cx.delivered_system_notifications()[0].tag.as_ref(),
        "mission:test-mission:idle"
    );
    assert_eq!(cx.shown_system_notifications().len(), 2);
    refresh(&mut cx);
    assert_eq!(cx.shown_system_notifications().len(), 2);
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn pairing_notification_and_toast_open_remote_access_once_per_event() {
    use crate::app_store::RemotePairing;
    use gpui::{SystemNotificationResponse, VisualTestContext};

    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        install(cx);
    });
    let window = cx.add_window(|window, cx| {
        NativeRoot::new(
            "pairing-test".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let controller = cx.update(|cx| cx.global::<GlobalNotifications>().0.clone());
    let pairing = RemotePairing {
        name: "MacBook Air".into(),
        fingerprint: "device-fingerprint".into(),
    };
    cx.run_until_parked();
    controller.update(&mut cx, |controller, _| {
        controller.permission = Permission::Allowed
    });
    store.update(&mut cx, |_, cx| cx.emit(pairing.clone()));
    cx.run_until_parked();
    let notices = cx.shown_system_notifications();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].title.as_ref(), "MacBook Air paired");
    assert_eq!(
        notices[0].body.as_ref(),
        "MacBook Air can now reach this computer."
    );
    cx.simulate_system_notification_response(SystemNotificationResponse {
        tag: notices[0].tag.clone(),
        action_id: None,
    });
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| {
            assert_eq!(root.route, AppRoute::Settings);
            assert_eq!(
                root.settings_page.pane,
                crate::surfaces::SettingsPane::Remote
            );
        })
        .unwrap();
    controller.update(&mut cx, |controller, _| {
        controller.permission = Permission::Denied
    });
    window
        .update(&mut cx, |root, _, _| root.route = AppRoute::Chat)
        .unwrap();
    store.update(&mut cx, |_, cx| cx.emit(pairing.clone()));
    cx.run_until_parked();
    assert_eq!(cx.shown_system_notifications().len(), 1);
    window
        .read_with(&cx, |root, _| {
            let toast = root.toasts.active().unwrap();
            assert_eq!(toast.message, "MacBook Air can now reach this computer.");
            assert!(toast.open_remote && toast.bottom);
        })
        .unwrap();
    let bounds = visual.debug_bounds("REMOTE_TOAST_ACTION").unwrap();
    visual.simulate_click(bounds.center(), gpui::Modifiers::default());
    window
        .read_with(&cx, |root, _| {
            assert_eq!(root.route, AppRoute::Settings);
            assert_eq!(
                root.settings_page.pane,
                crate::surfaces::SettingsPane::Remote
            );
            assert!(root.toasts.active().is_none());
        })
        .unwrap();
    store.update(&mut cx, |_, cx| cx.emit(pairing));
    cx.run_until_parked();
    assert_eq!(cx.shown_system_notifications().len(), 1);
    controller.update(&mut cx, |controller, _| {
        controller.permission = Permission::Allowed
    });
    store.update(&mut cx, |_, cx| {
        cx.emit(RemotePairing {
            name: "MacBook Air".into(),
            fingerprint: "device-fingerprint".into(),
        })
    });
    cx.run_until_parked();
    assert_eq!(
        cx.shown_system_notifications().len(),
        2,
        "pairing again after unknownDevice is a new notification"
    );
}

#[cfg(debug_assertions)]
#[test]
fn pairing_copy_is_localized_in_both_languages() {
    use runner_app::i18n::{testing::with_locale, Locale};
    let pairing = crate::app_store::RemotePairing {
        name: "MacBook Air".into(),
        fingerprint: "fingerprint".into(),
    };
    with_locale(Locale::English, || {
        assert_eq!(
            pairing_content(&pairing).title.as_ref(),
            "MacBook Air paired"
        )
    });
    with_locale(Locale::SimplifiedChinese, || {
        let notification = pairing_content(&pairing);
        assert_eq!(notification.title.as_ref(), "MacBook Air 已配对");
        assert_eq!(
            notification.body.as_ref(),
            "MacBook Air 现在可以访问这台电脑。"
        );
    });
}

#[cfg(target_os = "linux")]
#[test]
fn linux_pairing_uses_desktop_delivery_and_an_actionable_toast_with_unknown_permission() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    cx.update(|cx| {
        cx.set_app_identity("com.wycstudios.runner-dev", "Runner");
        install(cx);
    });
    let window = cx.add_window(|window, cx| {
        NativeRoot::new(
            "pairing-linux".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| permission(cx)), Permission::Unknown);
    store.update(&mut cx, |_, cx| {
        cx.emit(crate::app_store::RemotePairing {
            name: "Linux host".into(),
            fingerprint: "fixture-fingerprint".into(),
        })
    });
    cx.run_until_parked();
    assert_eq!(cx.shown_system_notifications().len(), 1);
    window
        .read_with(&cx, |root, _| {
            let toast = root.toasts.active().unwrap();
            assert_eq!(toast.message, "Linux host can now reach this computer.");
            assert!(toast.open_remote && toast.bottom);
        })
        .unwrap();
    let bounds = visual.debug_bounds("REMOTE_TOAST_ACTION").unwrap();
    visual.simulate_click(bounds.center(), gpui::Modifiers::default());
    window
        .read_with(&visual, |root, _| {
            assert_eq!(root.route, AppRoute::Settings);
            assert_eq!(
                root.settings_page.pane,
                crate::surfaces::SettingsPane::Remote
            );
            assert!(root.toasts.active().is_none());
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(cx.shown_system_notifications().len(), 1);
}
