//! Linked groups in the sidebar (#748).

use super::link::{
    chat_row_zone, collapsed_summary, most_urgent, picker_entries, ChatChoice, ChatRowZone,
    LinkGroups, MissionShape, PickerEntry,
};
use super::link_picker::recency;
use super::*;
use crate::surfaces::sidebar::logic::DropKind;
use gpui::{size, Context, Render, TestAppContext, VisualTestContext, Window};
use runner_core::protocol::status::{Activity, AgentObservation, AgentStatus, Lifecycle};
use runner_core::protocol::{LinkedChat, MissionPhase};

fn chat(id: &str, runtime: &str) -> DirectSessionEntry {
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
        status: SessionStatus::Running,
        title: Some(format!("{id} title")),
        live_title: None,
        cwd: None,
        started_at: None,
        stopped_at: None,
        resumable: false,
        native_fork: false,
        forkable: false,
        agent_session_key: None,
        pinned: false,
        archived_at: None,
    }
}

fn linked(session: &str) -> LinkedChat {
    LinkedChat {
        session_id: session.into(),
        title: format!("{session} title"),
        archived: false,
        reports: true,
    }
}

fn idle() -> AgentStatus {
    AgentStatus {
        lifecycle: Lifecycle::Running,
        observation: AgentObservation {
            activity: Activity::Idle,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// A store with a tab holding `chats` at the root, then missions `m1`..`m3`
/// below it, each running with one live member. Returns the tab's node id.
fn store_with_group(
    cx: &mut TestAppContext,
    temp: &std::path::Path,
    chats: &[(&str, &str)],
) -> (Entity<AppStore>, String) {
    let store = crate::app_store::test_lifecycle_store(cx, temp);
    let core = store.read_with(cx, |store, _| store.test_core.clone());
    let conn = core.db.get().unwrap();
    conn.execute_batch(
        "INSERT INTO crews (id, name, created_at, updated_at)
         VALUES ('crew', 'Crew', '2026-11-03T00:00:00Z', '2026-11-03T00:00:00Z')",
    )
    .unwrap();
    for (index, mission) in ["m1", "m2", "m3"].into_iter().enumerate() {
        conn.execute_batch(&format!(
            "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('{mission}', 'crew', '{mission}', 'running', '2026-11-03T00:00:0{index}Z')"
        ))
        .unwrap();
        runner_daemon::repo::node::ensure_mission_node(&conn, mission, None).unwrap();
    }
    let ids = chats
        .iter()
        .map(|(id, _)| (*id).to_owned())
        .collect::<Vec<_>>();
    let mut layout = PaneLayout::single(Some(&ids[0]), &ids[..1]);
    for id in &ids[1..] {
        let pane = layout
            .split("p1", runner_core::pane_layout::SplitOrientation::Row)
            .unwrap();
        layout.assign_session(&pane, id).unwrap();
    }
    let tab = runner_daemon::repo::node::create_tab(
        &conn,
        None,
        "Frontman",
        -1,
        &layout.serialize().unwrap(),
    )
    .unwrap();
    drop(conn);
    store.update(cx, |store, cx| {
        store.refresh_nodes(cx).unwrap();
        store.refresh(crate::app_store::StoreRefreshKind::Missions, cx);
    });
    cx.run_until_parked();
    store.update(cx, |store, _| {
        store.sessions = chats
            .iter()
            .map(|(id, runtime)| chat(id, runtime))
            .collect();
        for summary in &mut store.missions {
            summary.mission.status = runner_core::protocol::model::MissionStatus::Running;
            summary.any_session_live = true;
            summary.session_statuses = vec![(format!("{}-member", summary.mission.id), idle())];
        }
    });
    (store, tab.id)
}

fn link(
    store: &Entity<AppStore>,
    cx: &mut TestAppContext,
    mission: &str,
    chat: Option<LinkedChat>,
) {
    store.update(cx, |store, _| {
        let summary = store
            .missions
            .iter_mut()
            .find(|summary| summary.mission.id == mission)
            .unwrap();
        summary.mission.linked_chat = chat;
    });
}

fn mission_node(store: &Entity<AppStore>, cx: &mut TestAppContext, mission: &str) -> String {
    store.read_with(cx, |store, _| {
        store
            .nodes
            .iter()
            .find(|node| node.ref_id.as_deref() == Some(mission))
            .unwrap()
            .id
            .clone()
    })
}

fn groups(sidebar: &Entity<Sidebar>, cx: &mut TestAppContext) -> LinkGroups {
    sidebar.update(cx, |sidebar, cx| {
        let tabs = TabSet::from_rows(&sidebar.app_store.read(cx).nodes);
        LinkGroups::of(&sidebar.resolved_sidebar_rows_for_tabs(&tabs, cx))
    })
}

#[test]
fn a_chat_with_two_missions_heads_one_group_and_an_unlinked_mission_stays_flat() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, tab) = store_with_group(&mut cx, temp.path(), &[("chat", "codex")]);
    link(&store, &mut cx, "m1", Some(linked("chat")));
    link(&store, &mut cx, "m2", Some(linked("chat")));
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));

    let groups = groups(&sidebar, &mut cx);
    let [m1, m2, m3] = ["m1", "m2", "m3"].map(|id| mission_node(&store, &mut cx, id));
    let mut expected = vec![m1.clone(), m2.clone()];
    let position = |id: &str| {
        store.read_with(&cx, |store, _| {
            store
                .nodes
                .iter()
                .find(|node| node.id == id)
                .unwrap()
                .position
        })
    };
    expected.sort_by_key(|id| position(id));
    assert_eq!(groups.missions(&tab), expected, "in their sidebar order");
    assert_eq!(groups.head(&m1), Some(tab.as_str()));
    assert_eq!(
        groups.head(&m3),
        None,
        "a mission without a link is a flat row"
    );
}

#[test]
fn archiving_the_chat_dissolves_the_group_and_a_terminal_cannot_head_one() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, tab) = store_with_group(&mut cx, temp.path(), &[("chat", "codex")]);
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    link(
        &store,
        &mut cx,
        "m1",
        Some(LinkedChat {
            archived: true,
            ..linked("chat")
        }),
    );
    assert!(groups(&sidebar, &mut cx).missions(&tab).is_empty());

    let temp = tempfile::tempdir().unwrap();
    let (store, tab) = store_with_group(&mut cx, temp.path(), &[("shell", "shell")]);
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    link(&store, &mut cx, "m1", Some(linked("shell")));
    assert!(groups(&sidebar, &mut cx).missions(&tab).is_empty());
}

#[test]
fn an_ended_mission_shows_only_its_unread_dot() {
    use crate::surfaces::sidebar::logic::AttentionState;
    assert_eq!(
        super::link::phase_attention(MissionPhase::Ended, true),
        AttentionState::Unread
    );
    assert_eq!(
        super::link::phase_attention(MissionPhase::Ended, false),
        AttentionState::None
    );
    assert_eq!(
        super::link::phase_attention(MissionPhase::Interrupted, true),
        AttentionState::Interrupted
    );
    assert_eq!(
        super::link::phase_attention(MissionPhase::Unknown, false),
        AttentionState::Working
    );
}

#[test]
fn the_collapsed_summary_names_the_most_urgent_state() {
    use MissionPhase::*;
    assert_eq!(
        most_urgent(&[Some(Settled), Some(Working)]),
        Some((Working, 1))
    );
    assert_eq!(
        most_urgent(&[Some(Working), Some(NeedsYou), Some(Failed), Some(NeedsYou)]),
        Some((NeedsYou, 2))
    );
    assert_eq!(
        most_urgent(&[Some(Settled), Some(Failed)]),
        Some((Failed, 1))
    );
    assert_eq!(
        most_urgent(&[Some(Ended), Some(Settled)]),
        Some((Settled, 1))
    );
    assert_eq!(
        most_urgent(&[Some(Working), Some(Interrupted), Some(Settled)]),
        Some((Interrupted, 1))
    );
    assert_eq!(
        most_urgent(&[Some(Interrupted), Some(Failed)]),
        Some((Failed, 1))
    );
    assert_eq!(
        most_urgent(&[Some(Interrupted), Some(NeedsYou)]),
        Some((NeedsYou, 1))
    );
    assert_eq!(
        collapsed_summary(&[Some(Working), Some(Interrupted)]),
        "2 missions · 1 interrupted"
    );
    assert_eq!(most_urgent(&[None]), None);
    assert_eq!(
        collapsed_summary(&[Some(Working), Some(NeedsYou)]),
        "2 missions · 1 needs you"
    );
    assert_eq!(collapsed_summary(&[Some(Settled)]), "1 mission · 1 settled");
    assert_eq!(collapsed_summary(&[None, None]), "2 missions");
}

#[test]
fn the_middle_half_of_a_chat_row_links_and_its_quarters_reorder() {
    for (y, zone) in [
        (100., ChatRowZone::Before),
        (106., ChatRowZone::Before),
        (108., ChatRowZone::Link),
        (116., ChatRowZone::Link),
        (124., ChatRowZone::Link),
        (125., ChatRowZone::After),
        (131., ChatRowZone::After),
    ] {
        assert_eq!(chat_row_zone(y, 100., 32.), zone, "{y}");
    }
}

#[test]
fn the_picker_lists_linked_chats_then_five_recent_and_searches_every_chat() {
    let choice = |id: &str, active_at: i64, missions: usize| ChatChoice {
        session_id: id.into(),
        title: format!("{id} chat"),
        runtime: "codex".into(),
        active_at: Some(active_at),
        missions,
    };
    let chats = [
        choice("frontman", 1, 2),
        choice("old", 2, 0),
        choice("a", 9, 0),
        choice("b", 8, 0),
        choice("c", 7, 0),
        choice("d", 6, 0),
        choice("e", 5, 0),
        choice("busy", 3, 1),
    ];
    let ids = |entries: Vec<PickerEntry>| {
        entries
            .into_iter()
            .map(|entry| match entry {
                PickerEntry::Heading(key) => key.to_owned(),
                PickerEntry::Chat(chat) => chat.session_id,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(picker_entries(&chats, "")),
        [
            "heading.with_missions",
            "busy",
            "frontman",
            "heading.recent",
            "a",
            "b",
            "c",
            "d",
            "e"
        ]
    );
    assert_eq!(
        ids(picker_entries(&chats, "OLD")),
        ["old"],
        "search reaches every chat"
    );
    assert!(picker_entries(&chats, "nothing").is_empty());
}

#[test]
fn the_picker_hint_says_how_long_ago_a_chat_was_active() {
    use chrono::TimeZone;
    let now = chrono::Local
        .with_ymd_and_hms(2026, 11, 3, 15, 0, 0)
        .unwrap();
    let ago = |minutes: i64| Some((now - chrono::Duration::minutes(minutes)).timestamp_millis());
    assert_eq!(recency(ago(0), now), "now");
    assert_eq!(recency(ago(5), now), "5m");
    assert_eq!(recency(ago(120), now), "2h");
    assert_eq!(recency(ago(20 * 60), now), "Yesterday");
    assert_eq!(recency(None, now), "");
}

#[test]
fn a_mission_drags_onto_a_chat_in_its_own_place_that_it_is_not_linked_to() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, tab) = store_with_group(&mut cx, temp.path(), &[("chat", "codex")]);
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    let [m1, m2] = ["m1", "m2"].map(|id| mission_node(&store, &mut cx, id));
    link(&store, &mut cx, "m2", Some(linked("chat")));
    let chats = vec![("chat".to_owned(), "chat title".to_owned())];
    sidebar.update(&mut cx, |sidebar, cx| {
        assert!(sidebar.can_link_drop(&m1, &tab, &chats, cx));
        assert!(
            !sidebar.can_link_drop(&m2, &tab, &chats, cx),
            "already linked there"
        );
        assert!(
            !sidebar.can_link_drop(&tab, &tab, &chats, cx),
            "only a mission links"
        );
    });
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    {
        let conn = core.db.get().unwrap();
        let project =
            runner_daemon::repo::project::create(&conn, "Elsewhere", "/tmp/else").unwrap();
        let project = runner_daemon::repo::node::ensure_project_node(&conn, &project.id).unwrap();
        conn.execute_batch(&format!(
            "UPDATE nodes SET parent_id = '{}' WHERE id = '{m1}'",
            project.id
        ))
        .unwrap();
    }
    store.update(&mut cx, |store, cx| store.refresh_nodes(cx).unwrap());
    sidebar.update(&mut cx, |sidebar, cx| {
        assert!(
            !sidebar.can_link_drop(&m1, &tab, &chats, cx),
            "a mission links only within its own project"
        );
    });
}

#[test]
fn linking_and_unlinking_go_through_the_daemon_and_unlinking_lands_below_the_group() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, tab) = store_with_group(&mut cx, temp.path(), &[("chat", "codex")]);
    let core = store.read_with(&cx, |store, _| store.test_core.clone());
    core.db
        .get()
        .unwrap()
        .execute(
            "INSERT INTO sessions (id, status, agent_runtime, agent_command, started_at)
             VALUES ('chat', 'running', 'codex', 'codex', '2026-11-03T00:00:00Z')",
            [],
        )
        .unwrap();
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    sidebar.update(&mut cx, |sidebar, cx| {
        sidebar
            .front_page
            .link_mission("m3".into(), "chat".into())
            .unwrap();
        sidebar.refresh_store(crate::app_store::StoreRefreshKind::All, cx);
    });
    cx.run_until_parked();
    let linked_to = |cx: &mut TestAppContext| {
        store.read_with(cx, |store, _| {
            store
                .missions
                .iter()
                .find(|summary| summary.mission.id == "m3")
                .and_then(|summary| summary.mission.linked_chat.clone())
                .map(|chat| chat.session_id)
        })
    };
    assert_eq!(linked_to(&mut cx).as_deref(), Some("chat"));
    let m3 = mission_node(&store, &mut cx, "m3");
    assert_eq!(groups(&sidebar, &mut cx).head(&m3), Some(tab.as_str()));

    sidebar.update(&mut cx, |sidebar, cx| {
        sidebar.unlink_mission("m3".into(), cx)
    });
    cx.run_until_parked();
    assert_eq!(linked_to(&mut cx), None);
    let order = store.read_with(&cx, |store, _| {
        let mut root = store
            .nodes
            .iter()
            .filter(|node| node.parent_id.is_none() && node.pinned_position.is_none())
            .collect::<Vec<_>>();
        root.sort_by_key(|node| node.position);
        root.into_iter()
            .map(|node| node.id.clone())
            .collect::<Vec<_>>()
    });
    let tab_at = order.iter().position(|id| *id == tab).unwrap();
    assert_eq!(
        order[tab_at + 1],
        m3,
        "an unlinked mission lands right below its group"
    );
}

struct ScopeRender {
    sidebar: Entity<Sidebar>,
}

impl Render for ScopeRender {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.sidebar.update(cx, |sidebar, cx| {
            let tabs = TabSet::from_rows(&sidebar.app_store.read(cx).nodes);
            let rows = sidebar.resolved_sidebar_rows_for_tabs(&tabs, cx);
            let groups = LinkGroups::of(&rows);
            let scope = sidebar.scope_rows(&rows, None);
            sidebar.render_scope(
                scope,
                &rows,
                &groups,
                None,
                DropKind::Leaf,
                None,
                &HashMap::new(),
                cx,
            )
        });
        div().w(px(300.)).flex().flex_col().children(rows)
    }
}

#[test]
fn projects_draw_the_group_outline_and_collapse_it_to_a_summary() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, tab) = store_with_group(&mut cx, temp.path(), &[("chat", "codex")]);
    link(&store, &mut cx, "m1", Some(linked("chat")));
    link(&store, &mut cx, "m2", Some(linked("chat")));
    store.update(&mut cx, |store, _| {
        for summary in &mut store.missions {
            summary.phase = Some(if summary.mission.id == "m1" {
                MissionPhase::NeedsYou
            } else {
                MissionPhase::Working
            });
        }
    });
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    let host = cx.add_window(|_, _| ScopeRender {
        sidebar: sidebar.clone(),
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(400.), px(600.)));
    visual.run_until_parked();

    let group = visual
        .debug_bounds(leak(format!("LINK_GROUP {tab}")))
        .unwrap();
    for (mission, inside) in [("m1", true), ("m2", true), ("m3", false)] {
        let row = visual
            .debug_bounds(leak(format!("SIDEBAR_ROW sidebar-mission-{mission}")))
            .unwrap();
        assert_eq!(group.contains(&row.center()), inside, "{mission}");
    }
    assert!(visual
        .debug_bounds(leak(format!("LINK_GROUP_SUMMARY {tab}")))
        .is_none());

    let toggle = visual
        .debug_bounds(leak(format!("LINK_GROUP_TOGGLE {tab}")))
        .unwrap();
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(visual
        .debug_bounds(leak(format!("LINK_GROUP_SUMMARY {tab}")))
        .is_some());
    assert!(visual
        .debug_bounds("SIDEBAR_ROW sidebar-mission-m1")
        .is_none());
    assert!(visual
        .debug_bounds("SIDEBAR_ROW sidebar-mission-m3")
        .is_some());
    store.read_with(&visual, |store, _| {
        assert!(store.settings.sidebar_collapsed_link_groups.contains(&tab));
    });
}

struct ActiveRender {
    sidebar: Entity<Sidebar>,
}

impl Render for ActiveRender {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(self.sidebar.update(cx, |sidebar, cx| {
                let tabs = TabSet::from_rows(&sidebar.app_store.read(cx).nodes);
                sidebar.render_active_groups(sidebar.resolved_active_rows_for_tabs(&tabs, cx), cx)
            }))
    }
}

#[test]
fn active_lists_chats_and_missions_under_agents_with_the_linked_group() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, tab) = store_with_group(&mut cx, temp.path(), &[("chat", "codex")]);
    link(&store, &mut cx, "m2", Some(linked("chat")));
    store.update(&mut cx, |store, _| {
        store.session_statuses.insert("chat".into(), idle());
    });
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    sidebar.update(&mut cx, |sidebar, cx| {
        let tabs = TabSet::from_rows(&sidebar.app_store.read(cx).nodes);
        let rows = sidebar.resolved_active_rows_for_tabs(&tabs, cx);
        assert!(rows
            .iter()
            .all(|row| row.group() == super::active::ActiveGroup::Agents));
        assert_eq!(rows.len(), 4, "the chat and its three missions");
    });
    let host = cx.add_window(|_, _| ActiveRender {
        sidebar: sidebar.clone(),
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(400.), px(800.)));
    visual.run_until_parked();
    let group = visual
        .debug_bounds(leak(format!("ACTIVE_LINK_GROUP {tab}")))
        .unwrap();
    let m2 = mission_node(&store, &mut visual, "m2");
    let m1 = mission_node(&store, &mut visual, "m1");
    let title = |id: &str| leak(format!("ACTIVE_TITLE {id}"));
    assert!(group.contains(&visual.debug_bounds(title(&m2)).unwrap().center()));
    assert!(!group.contains(&visual.debug_bounds(title(&m1)).unwrap().center()));
    assert!(visual.debug_bounds("ACTIVE_GROUP_TOGGLE agents").is_some());
    assert!(visual
        .debug_bounds("ACTIVE_GROUP_TOGGLE missions")
        .is_none());
    // The same line and alignment as Projects: from the chat's mark, flags 20 px right.
    let near = |a: gpui::Pixels, b: gpui::Pixels| (a - b).abs() <= px(1.);
    let row = |visual: &mut VisualTestContext, id: &str| {
        visual
            .debug_bounds(leak(format!("SIDEBAR_ROW active-{id}")))
            .unwrap()
    };
    let bounds = |visual: &mut VisualTestContext, selector: String| {
        visual
            .debug_bounds(leak(selector.clone()))
            .unwrap_or_else(|| panic!("missing {selector}"))
    };
    let (head, flat, grouped) = (
        row(&mut visual, &tab),
        row(&mut visual, &m1),
        row(&mut visual, &m2),
    );
    let mark = |visual: &mut VisualTestContext, id: &str| {
        bounds(visual, format!("SIDEBAR_MARK active-{id}"))
    };
    let (head_mark, flat_mark, flag) = (
        mark(&mut visual, &tab),
        mark(&mut visual, &m1),
        mark(&mut visual, &m2),
    );
    assert!(near(head.left(), flat.left()), "{head:?} {flat:?}");
    // The chevron's room moves the chat's mark, line and missions right (Jason, 2026-11-03).
    assert!(
        near(head_mark.left(), flat_mark.left() + px(15.)),
        "{head_mark:?} {flat_mark:?}"
    );
    assert!(near(flag.left(), head_mark.left() + px(20.)), "{flag:?}");
    assert!(near(grouped.right(), flat.right()) && near(head.right(), flat.right()));
    let stub = bounds(&mut visual, format!("LINK_STUB {tab}"));
    assert!(near(stub.center().x, head_mark.center().x), "{stub:?}");
    let branch = bounds(&mut visual, format!("LINK_BRANCH {tab} 0"));
    assert!(branch.right() < flag.left());
    assert!(near(branch.bottom(), grouped.center().y));
    let toggle = bounds(&mut visual, format!("LINK_GROUP_TOGGLE {tab}"));
    let outline = bounds(&mut visual, format!("LINK_GROUP_OUTLINE {tab}"));
    assert!(
        near(toggle.left() - (outline.left() + px(1.)), px(9.)),
        "the chevron sits 9 px inside the outline: {toggle:?} {outline:?}"
    );
    assert!(
        toggle.left() >= head.left() && head_mark.left() - toggle.right() >= px(6.) - px(0.5),
        "inside the chat row's box, at least 6 px before its mark: {toggle:?} {head_mark:?}"
    );
    assert!(near(toggle.center().y, head.center().y));
    assert!(
        grouped.left() == head.left()
            && near(grouped.left(), outline.left())
            && near(grouped.right(), outline.right()),
        "a linked mission's row spans the group like its chat's: {grouped:?} {head:?} {outline:?}"
    );
    assert!(
        near(flat_mark.left(), flat.left() + px(11.)),
        "a flat row keeps its padding: {flat_mark:?} {flat:?}"
    );

    // Collapsing here collapses the group in Projects too: one state per group.
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    store.read_with(&visual, |store, _| {
        assert!(store.settings.sidebar_collapsed_link_groups.contains(&tab));
    });
    let collapsed = row(&mut visual, &tab);
    let outline = bounds(&mut visual, format!("LINK_GROUP_OUTLINE {tab}"));
    assert!(
        visual.debug_bounds(title(&m2)).is_none(),
        "its missions hide"
    );
    assert!(
        near(collapsed.size.height, flat.size.height)
            && outline.size.height - collapsed.size.height <= px(2.5),
        "collapsed, the group is one row of the usual height: {collapsed:?} {outline:?} {flat:?}"
    );
    let summary = bounds(&mut visual, format!("LINK_GROUP_SUMMARY {tab}"));
    let collapsed_title = bounds(&mut visual, format!("ACTIVE_TITLE {tab}"));
    assert!(
        collapsed.contains(&summary.center()) && summary.top() >= collapsed_title.bottom() - px(1.),
        "the summary is the row's second line: {summary:?} {collapsed_title:?}"
    );
    assert!(
        visual
            .debug_bounds(leak(format!("ACTIVE_LOCATION {tab}")))
            .is_none(),
        "in place of the folder line"
    );
    // A hovered row shows its actions instead of its state.
    visual.simulate_mouse_move(
        gpui::point(px(390.), px(790.)),
        None,
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    assert!(visual
        .debug_bounds(leak(format!("LINK_GROUP_STATE {tab} NeedsYou")))
        .or_else(|| {
            ["None", "Unavailable", "Unread", "Working", "Error"]
                .into_iter()
                .find_map(|state| {
                    visual.debug_bounds(leak(format!("LINK_GROUP_STATE {tab} {state}")))
                })
        })
        .is_some_and(|glyph| collapsed.contains(&glyph.center())));
    let toggle = bounds(&mut visual, format!("LINK_GROUP_TOGGLE {tab}"));
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(
        visual.debug_bounds(title(&m2)).is_some(),
        "expanding shows them again"
    );
    assert!(visual
        .debug_bounds(leak(format!("ACTIVE_LOCATION {tab}")))
        .is_some());
    let order = sidebar.update(&mut visual, |sidebar, cx| {
        let tabs = TabSet::from_rows(&sidebar.app_store.read(cx).nodes);
        sidebar
            .resolved_active_rows_for_tabs(&tabs, cx)
            .iter()
            .map(|row| row.shortcut_row(&sidebar.app_store.read(cx).nodes).node_id)
            .collect::<Vec<_>>()
    });
    let m3 = mission_node(&store, &mut visual, "m3");
    assert_eq!(
        order,
        [m1, m2, m3, tab],
        "chats and missions in creation order"
    );
}

/// A real chat row and a window to drop in, for the drop paths.
fn drop_fixture(
    cx: &mut TestAppContext,
    temp: &std::path::Path,
) -> (Entity<AppStore>, Entity<Sidebar>, VisualTestContext, String) {
    let (store, tab) = store_with_group(cx, temp, &[("chat", "codex")]);
    let core = store.read_with(cx, |store, _| store.test_core.clone());
    core.db
        .get()
        .unwrap()
        .execute_batch(
            "INSERT INTO sessions (id, status, agent_runtime, agent_command, started_at)
             VALUES ('chat', 'running', 'codex', 'codex', '2026-11-03T00:00:00Z')",
        )
        .unwrap();
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    let host = cx.add_window(|_, _| ScopeRender {
        sidebar: sidebar.clone(),
    });
    let visual = VisualTestContext::from_window(host.into(), cx);
    (store, sidebar, visual, tab)
}

fn linked_session(
    store: &Entity<AppStore>,
    cx: &mut VisualTestContext,
    mission: &str,
) -> Option<String> {
    store.read_with(cx, |store, _| {
        store
            .missions
            .iter()
            .find(|summary| summary.mission.id == mission)
            .and_then(|summary| summary.mission.linked_chat.as_ref())
            .map(|chat| chat.session_id.clone())
    })
}

#[test]
fn dropping_on_the_middle_of_a_chat_row_links_the_mission() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, sidebar, mut visual, tab) = drop_fixture(&mut cx, temp.path());
    let m1 = mission_node(&store, &mut visual, "m1");

    sidebar.update(&mut visual, |sidebar, cx| {
        sidebar.start_sidebar_drag(m1.clone(), cx);
        sidebar.set_link_drop(
            super::link::LinkDrop {
                mission_node: m1.clone(),
                tab_node: tab.clone(),
                chats: vec![("chat".into(), "chat title".into())],
            },
            cx,
        );
    });
    visual.update(|window, cx| {
        sidebar.update(cx, |sidebar, cx| {
            sidebar.commit_sidebar_drop(&m1, window, cx)
        })
    });
    visual.run_until_parked();
    assert_eq!(
        linked_session(&store, &mut visual, "m1").as_deref(),
        Some("chat")
    );
}

#[test]
fn a_drop_into_another_group_links_out_of_its_own_unlinks_and_within_it_keeps_the_link() {
    use super::link::{link_change, LinkChange};
    assert_eq!(
        link_change(Some("tab"), None),
        Some(LinkChange::Link("tab".into()))
    );
    assert_eq!(
        link_change(Some("other"), Some("tab")),
        Some(LinkChange::Link("other".into())),
        "into another chat's group moves the link"
    );
    assert_eq!(link_change(None, Some("tab")), Some(LinkChange::Unlink));
    assert_eq!(
        link_change(Some("tab"), Some("tab")),
        None,
        "reordering keeps the link"
    );
    assert_eq!(link_change(None, None), None);
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn set_phase(
    store: &Entity<AppStore>,
    cx: &mut TestAppContext,
    mission: &str,
    phase: MissionPhase,
) {
    store.update(cx, |store, _| {
        let summary = store
            .missions
            .iter_mut()
            .find(|summary| summary.mission.id == mission)
            .unwrap();
        summary.phase = Some(phase);
    });
}

fn row_attention(
    sidebar: &Entity<Sidebar>,
    cx: &mut TestAppContext,
    mission_node: &str,
) -> crate::surfaces::sidebar::logic::AttentionState {
    sidebar.update(cx, |sidebar, cx| {
        let tabs = TabSet::from_rows(&sidebar.app_store.read(cx).nodes);
        sidebar
            .resolved_sidebar_rows_for_tabs(&tabs, cx)
            .into_iter()
            .find(|row| row.node().id == mission_node)
            .unwrap()
            .attention()
    })
}

#[test]
fn mission_rows_show_the_daemons_state_over_their_members_statuses() {
    use crate::surfaces::sidebar::logic::AttentionState;
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, tab) = store_with_group(&mut cx, temp.path(), &[("chat", "codex")]);
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    let [m1, m2, m3] = ["m1", "m2", "m3"].map(|id| mission_node(&store, &mut cx, id));

    // A pending question while every member idles.
    set_phase(&store, &mut cx, "m1", MissionPhase::NeedsYou);
    assert_eq!(
        row_attention(&sidebar, &mut cx, &m1),
        AttentionState::NeedsYou
    );
    // A crashed member beside one waiting: the mission needs the person first.
    store.update(&mut cx, |store, _| {
        let summary = store
            .missions
            .iter_mut()
            .find(|s| s.mission.id == "m2")
            .unwrap();
        summary.session_statuses = vec![
            (
                "crashed".into(),
                AgentStatus {
                    lifecycle: Lifecycle::Error,
                    ..Default::default()
                },
            ),
            ("waiting".into(), idle()),
        ];
    });
    set_phase(&store, &mut cx, "m2", MissionPhase::NeedsYou);
    assert_eq!(
        row_attention(&sidebar, &mut cx, &m2),
        AttentionState::NeedsYou
    );
    // Idle members within the settle window are still working.
    set_phase(&store, &mut cx, "m3", MissionPhase::Working);
    assert_eq!(
        row_attention(&sidebar, &mut cx, &m3),
        AttentionState::Working
    );
    set_phase(&store, &mut cx, "m3", MissionPhase::Ended);
    assert_eq!(row_attention(&sidebar, &mut cx, &m3), AttentionState::None);

    let host = cx.add_window(|_, _| ScopeRender {
        sidebar: sidebar.clone(),
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(400.), px(600.)));
    // A hovered row shows its actions instead of its state.
    visual.simulate_mouse_move(
        gpui::point(px(390.), px(590.)),
        None,
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    assert!(visual
        .debug_bounds(leak(format!("MISSION_STATE {m1} NeedsYou")))
        .is_some());
    assert!(visual
        .debug_bounds(leak(format!("MISSION_STATE {m3} Ended")))
        .is_some());

    link(&store, &mut visual, "m1", Some(linked("chat")));
    link(&store, &mut visual, "m3", Some(linked("chat")));
    store.update(&mut visual, |store, _| {
        store
            .settings
            .sidebar_collapsed_link_groups
            .insert(tab.clone());
    });
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds(leak(format!("LINK_GROUP_STATE {tab} NeedsYou")))
            .is_some(),
        "a collapsed group's head shows its most urgent mission"
    );
}

#[test]
fn an_ended_linked_mission_stays_in_its_active_group_until_archived() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, tab) = store_with_group(&mut cx, temp.path(), &[("chat", "codex")]);
    link(&store, &mut cx, "m2", Some(linked("chat")));
    store.update(&mut cx, |store, _| {
        store.session_statuses.insert("chat".into(), idle());
        let summary = store
            .missions
            .iter_mut()
            .find(|s| s.mission.id == "m2")
            .unwrap();
        summary.any_session_live = false;
        summary.session_statuses = vec![(
            "m2-member".into(),
            AgentStatus {
                lifecycle: Lifecycle::Stopped,
                ..Default::default()
            },
        )];
        summary.phase = Some(MissionPhase::Ended);
    });
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    let m2 = mission_node(&store, &mut cx, "m2");
    let active = |cx: &mut TestAppContext| {
        sidebar.update(cx, |sidebar, cx| {
            let tabs = TabSet::from_rows(&sidebar.app_store.read(cx).nodes);
            sidebar
                .resolved_active_rows_for_tabs(&tabs, cx)
                .iter()
                .map(|row| row.source.node().id.clone())
                .collect::<Vec<_>>()
        })
    };
    let rows = active(&mut cx);
    assert!(rows.contains(&m2) && rows.contains(&tab));

    store.update(&mut cx, |store, _| {
        let summary = store
            .missions
            .iter_mut()
            .find(|s| s.mission.id == "m2")
            .unwrap();
        summary.mission.archived_at = Some(chrono::Utc::now());
    });
    assert!(!active(&mut cx).contains(&m2), "archiving takes it out");
}

#[test]
fn shortcuts_number_rows_in_the_order_groups_draw() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, tab) = store_with_group(&mut cx, temp.path(), &[("chat", "codex")]);
    // m1..m3 sit below the tab; linking m3 draws it right after the tab.
    link(&store, &mut cx, "m3", Some(linked("chat")));
    let [m1, m2, m3] = ["m1", "m2", "m3"].map(|id| mission_node(&store, &mut cx, id));
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    let numbered = |cx: &mut TestAppContext| {
        sidebar.update(cx, |sidebar, cx| {
            let tabs = TabSet::from_rows(&sidebar.app_store.read(cx).nodes);
            sidebar.refresh_shortcut_rows(&tabs, cx);
            sidebar
                .numbered_shortcut_rows
                .iter()
                .map(|row| row.node_id.clone())
                .collect::<Vec<_>>()
        })
    };
    let order = numbered(&mut cx);
    let flat = order
        .iter()
        .filter(|id| **id != tab && **id != m3)
        .cloned()
        .collect::<Vec<_>>();
    let tab_at = order.iter().position(|id| *id == tab).unwrap();
    assert_eq!(
        order[tab_at + 1],
        m3,
        "a linked mission is numbered right after its chat"
    );
    assert_eq!(flat.len(), 2);
    assert!(flat.contains(&m1) && flat.contains(&m2));

    store.update(&mut cx, |store, _| {
        store
            .settings
            .sidebar_collapsed_link_groups
            .insert(tab.clone());
    });
    assert!(
        !numbered(&mut cx).contains(&m3),
        "a collapsed group's missions get no numbers"
    );

    store.update(&mut cx, |store, _| {
        store.settings.sidebar_collapsed_link_groups.clear();
        store.settings.sidebar_active_only = true;
        store.session_statuses.insert("chat".into(), idle());
    });
    let order = numbered(&mut cx);
    let tab_at = order.iter().position(|id| *id == tab).unwrap();
    assert_eq!(
        order[tab_at + 1],
        m3,
        "Active numbers a group's missions under its chat"
    );
    store.update(&mut cx, |store, _| {
        store
            .settings
            .sidebar_collapsed_link_groups
            .insert(tab.clone());
    });
    assert!(
        !numbered(&mut cx).contains(&m3),
        "nor while the group is collapsed, in Active too"
    );
}

#[test]
fn recency_follows_the_chats_last_activity_not_its_start() {
    let mut started_yesterday = chat("old", "codex");
    started_yesterday.started_at = Some(chrono::Utc::now() - chrono::Duration::days(1));
    let mut started_today = chat("new", "codex");
    started_today.started_at = Some(chrono::Utc::now() - chrono::Duration::hours(3));
    let layout = PaneLayout::single(Some("old"), &["old".into()]);
    let node = runner_core::protocol::node::NodeRow {
        id: "tab".into(),
        parent_id: None,
        position: 0,
        node_type: runner_core::protocol::node::NodeType::Tab,
        name: None,
        ref_id: None,
        layout: Some(layout.serialize().unwrap()),
        pinned_position: None,
        last_completed_at: Some(chrono::Utc::now().to_rfc3339()),
        last_viewed_at: None,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let nodes = [node];
    let old = super::link::chat_active_at(&started_yesterday, &nodes).unwrap();
    let new = super::link::chat_active_at(&started_today, &nodes).unwrap();
    assert!(
        old > new,
        "the chat used a moment ago is the more recent one"
    );
}

#[test]
fn the_picker_scrolls_within_a_small_window_and_keeps_the_highlight_visible() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, _) = store_with_group(&mut cx, temp.path(), &[("chat", "codex")]);
    store.update(&mut cx, |store, _| {
        store.sessions = (0..40)
            .map(|i| chat(&format!("chat-{i:02}"), "codex"))
            .collect();
    });
    let sidebar = cx.new(|cx| Sidebar::new(WeakEntity::new_invalid(), store.clone(), None, cx));
    struct PickerHost {
        sidebar: Entity<Sidebar>,
    }
    impl Render for PickerHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .children(self.sidebar.read(cx).link_picker())
        }
    }
    let host = cx.add_window(|_, _| PickerHost {
        sidebar: sidebar.clone(),
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(500.), px(320.)));
    visual.update(|window, cx| {
        sidebar.update(cx, |sidebar, cx| {
            sidebar.open_link_picker(
                "m1".into(),
                None,
                None,
                super::link::PickerAnchor::Pointer,
                window,
                cx,
            )
        })
    });
    visual.run_until_parked();
    // Type to search every chat, then walk far down the list.
    visual.simulate_input("chat-");
    visual.run_until_parked();
    let list = visual.debug_bounds("LINK_PICKER_LIST").unwrap();
    assert!(
        list.bottom() <= px(320.),
        "the list stays inside the window: {list:?}"
    );
    for _ in 0..30 {
        visual.simulate_keystrokes("down");
    }
    visual.run_until_parked();
    let list = visual.debug_bounds("LINK_PICKER_LIST").unwrap();
    let highlighted = visual.debug_bounds("LINK_PICKER_CHAT chat-30").unwrap();
    assert!(
        list.top() <= highlighted.top() && highlighted.bottom() <= list.bottom() + px(1.),
        "the highlighted chat is scrolled into view: {highlighted:?} in {list:?}"
    );

    // A changed search highlights its first match, scrolled back into view.
    visual.simulate_keystrokes("backspace");
    visual.run_until_parked();
    let list = visual.debug_bounds("LINK_PICKER_LIST").unwrap();
    let first = visual.debug_bounds("LINK_PICKER_CHAT chat-00").unwrap();
    assert!(
        list.top() <= first.top() + px(1.) && first.bottom() <= list.bottom() + px(1.),
        "the new first match is in view: {first:?} in {list:?}"
    );
}

/// A real app window over a store whose `chats` are direct chat rows in one tab.
struct RootFixture {
    store: Entity<AppStore>,
    window: gpui::WindowHandle<NativeRoot>,
    sidebar: Entity<Sidebar>,
    tab: String,
    visual: VisualTestContext,
}

impl RootFixture {
    fn new(cx: &mut TestAppContext, temp: &std::path::Path, chats: &[&str]) -> Self {
        let pairs = chats.iter().map(|id| (*id, "codex")).collect::<Vec<_>>();
        let (store, tab) = store_with_group(cx, temp, &pairs);
        let core = store.read_with(cx, |store, _| store.test_core.clone());
        for chat in chats {
            core.db
                .get()
                .unwrap()
                .execute_batch(&format!(
                    "INSERT INTO sessions (id, status, agent_runtime, agent_command, title, started_at)
                     VALUES ('{chat}', 'running', 'codex', 'codex', '{chat} title', '2026-11-03T00:00:00Z')"
                ))
                .unwrap();
        }
        let window =
            crate::surfaces::remote_client::tests::add_window(cx, &store, temp, "link-root");
        let sidebar = window
            .update(cx, |root, _, _| root.sidebar.clone())
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        store.update(&mut visual, |store, cx| {
            store.refresh(crate::app_store::StoreRefreshKind::All, cx)
        });
        visual.run_until_parked();
        Self {
            store,
            window,
            sidebar,
            tab,
            visual,
        }
    }

    fn link(&mut self, mission: &str, chat: &str) {
        self.sidebar.update(&mut self.visual, |sidebar, cx| {
            sidebar
                .front_page
                .link_mission(mission.into(), chat.into())
                .unwrap();
            sidebar.refresh_store(crate::app_store::StoreRefreshKind::All, cx);
        });
        self.visual.run_until_parked();
    }

    fn node(&mut self, mission: &str) -> String {
        mission_node(&self.store, &mut self.visual, mission)
    }

    fn linked(&mut self, mission: &str) -> Option<String> {
        linked_session(&self.store, &mut self.visual, mission)
    }

    fn groups(&mut self) -> LinkGroups {
        self.sidebar.update(&mut self.visual, |sidebar, cx| {
            LinkGroups::of(&sidebar.resolved_sidebar_rows(cx))
        })
    }

    /// The root's own rows, as Recents draws them: grouped missions left out.
    fn flat(&mut self) -> Vec<String> {
        self.sidebar.update(&mut self.visual, |sidebar, cx| {
            let rows = sidebar.resolved_sidebar_rows(cx);
            let groups = LinkGroups::of(&rows);
            sidebar
                .scope_rows(&rows, None)
                .into_iter()
                .map(|row| row.node().id.clone())
                .filter(|id| groups.head(id).is_none())
                .collect()
        })
    }

    /// Hover then drop in one update, as a real drop arrives: a render between
    /// them clears a drag that no pointer is holding.
    fn drag_drop(
        &mut self,
        dragged: &str,
        kind: DropKind,
        visible: &[String],
        hovered: &str,
        group: Option<String>,
    ) {
        self.drag_drop_at(dragged, kind, visible, hovered, true, group);
    }

    fn drag_drop_at(
        &mut self,
        dragged: &str,
        kind: DropKind,
        visible: &[String],
        hovered: &str,
        after: bool,
        group: Option<String>,
    ) {
        let sidebar = self.sidebar.clone();
        let dragged = dragged.to_owned();
        let visible = visible.to_vec();
        let hovered = hovered.to_owned();
        self.visual.update(|window, cx| {
            sidebar.update(cx, |sidebar, cx| {
                sidebar.start_sidebar_drag(dragged.clone(), cx);
                sidebar.update_row_drop_target(
                    &dragged,
                    kind,
                    None,
                    &visible,
                    &hovered,
                    after,
                    format!("test:{hovered}:{after}"),
                    group,
                    cx,
                );
                assert!(sidebar.drop_target.is_some(), "a drop target");
                sidebar.commit_sidebar_drop(&dragged, window, cx);
            })
        });
        self.visual.run_until_parked();
    }

    /// The label a reorder line shows while `dragged` hovers after `hovered`.
    fn hint(
        &mut self,
        dragged: &str,
        visible: &[String],
        hovered: &str,
        group: Option<String>,
    ) -> Option<super::link::DropHint> {
        self.sidebar.update(&mut self.visual, |sidebar, cx| {
            sidebar.start_sidebar_drag(dragged.to_owned(), cx);
            sidebar.update_row_drop_target(
                dragged,
                DropKind::Leaf,
                None,
                visible,
                hovered,
                true,
                format!("test:{hovered}:true"),
                group,
                cx,
            );
            let rows = sidebar.resolved_sidebar_rows(cx);
            let hint = sidebar.drop_hint(&rows, &LinkGroups::of(&rows), cx);
            sidebar.clear_sidebar_drag("test", cx);
            hint
        })
    }

    fn pinned(&mut self) -> Vec<String> {
        self.store.read_with(&self.visual, |store, _| {
            let mut pinned = store
                .nodes
                .iter()
                .filter(|node| node.pinned_position.is_some())
                .collect::<Vec<_>>();
            pinned.sort_by_key(|node| node.pinned_position);
            pinned.into_iter().map(|node| node.id.clone()).collect()
        })
    }
}

#[test]
fn dragging_out_of_the_group_unlinks_where_it_drops_and_the_last_one_dissolves_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat"]);
    f.link("m1", "chat");
    f.link("m2", "chat");
    let (m1, m2) = (f.node("m1"), f.node("m2"));
    assert_eq!(f.groups().missions(&f.tab).len(), 2);

    let flat = f.flat();
    let tab_at = flat.iter().position(|id| *id == f.tab).unwrap();
    f.drag_drop(&m1, DropKind::Leaf, &flat, flat.last().unwrap(), None);
    assert_eq!(f.linked("m1"), None);
    assert_eq!(
        f.flat().last(),
        Some(&m1),
        "it lands exactly where it was dropped"
    );

    let flat = f.flat();
    f.drag_drop(&m2, DropKind::Leaf, &flat, flat.last().unwrap(), None);
    assert_eq!(f.linked("m2"), None);
    assert!(
        f.groups().missions(&f.tab).is_empty(),
        "the group dissolves"
    );
    let flat = f.flat();
    assert_eq!(
        flat.iter().position(|id| *id == f.tab),
        Some(tab_at),
        "the chat row keeps its place"
    );
}

#[test]
fn a_reorder_line_names_the_chat_its_drop_links_to_or_unlinks_from() {
    use super::link::DropHint;
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat"]);
    f.link("m1", "chat");
    f.link("m2", "chat");
    let (m1, m2, m3) = (f.node("m1"), f.node("m2"), f.node("m3"));
    let tab = f.tab.clone();
    let children = [m1.clone(), m2.clone()];

    let flat = f.flat();
    assert_eq!(
        f.hint(&m1, &flat, flat.last().unwrap(), None),
        Some(DropHint::Unlink("chat title".into())),
        "out of its group among the project's rows"
    );
    assert_eq!(
        f.hint(&m3, &children, &m2, Some(tab.clone())),
        Some(DropHint::Link("chat title".into())),
        "a flat mission dropped after a group's last mission links to its chat"
    );
    assert_eq!(
        f.hint(&m1, &children, &m2, Some(tab)),
        None,
        "a reorder within its own group keeps the link"
    );
    assert_eq!(f.hint(&m3, &flat, &f.tab.clone(), None), None);
}

#[test]
fn dragging_out_into_the_pinned_rows_unlinks_and_pins_it_there() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat"]);
    let m3 = f.node("m3");
    f.sidebar.update(&mut f.visual, |sidebar, cx| {
        let node = sidebar
            .source_nodes(cx)
            .into_iter()
            .find(|node| node.id == m3)
            .unwrap();
        sidebar.front_page.pin(&node, true).unwrap();
        sidebar.refresh_store(crate::app_store::StoreRefreshKind::All, cx);
    });
    f.visual.run_until_parked();
    f.link("m1", "chat");
    let m1 = f.node("m1");
    assert_eq!(f.pinned(), std::slice::from_ref(&m3));

    f.drag_drop(&m1, DropKind::Pinned, std::slice::from_ref(&m3), &m3, None);
    assert_eq!(f.linked("m1"), None);
    assert_eq!(f.pinned(), [m3, m1], "pinned exactly where it was dropped");
}

#[test]
fn dragging_out_into_the_pinned_rows_lands_before_the_first_or_between_two() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat"]);
    for mission in ["m2", "m3"] {
        let node = f.node(mission);
        f.sidebar.update(&mut f.visual, |sidebar, cx| {
            let node = sidebar
                .source_nodes(cx)
                .into_iter()
                .find(|row| row.id == node)
                .unwrap();
            sidebar.front_page.pin(&node, true).unwrap();
            sidebar.refresh_store(crate::app_store::StoreRefreshKind::All, cx);
        });
        f.visual.run_until_parked();
    }
    let m1 = f.node("m1");
    let pinned = f.pinned();
    assert_eq!(pinned.len(), 2);

    f.link("m1", "chat");
    f.drag_drop_at(&m1, DropKind::Pinned, &pinned, &pinned[0], false, None);
    assert_eq!(f.linked("m1"), None);
    assert_eq!(
        f.pinned(),
        [m1.clone(), pinned[0].clone(), pinned[1].clone()],
        "before the first pinned row"
    );

    f.link("m1", "chat");
    assert_eq!(
        f.pinned(),
        pinned,
        "linking takes the mission's own pin away"
    );
    f.drag_drop_at(&m1, DropKind::Pinned, &pinned, &pinned[0], true, None);
    assert_eq!(f.linked("m1"), None);
    assert_eq!(
        f.pinned(),
        [pinned[0].clone(), m1, pinned[1].clone()],
        "between the two pinned rows"
    );
}

#[test]
fn a_drop_between_a_several_chat_groups_missions_waits_for_the_chat_and_its_place() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat", "chat2"]);
    f.link("m1", "chat");
    let (m1, m3) = (f.node("m1"), f.node("m3"));
    let order_before = f.flat();

    // Cancelled: nothing changes.
    let tab = f.tab.clone();
    f.drag_drop(
        &m3,
        DropKind::Leaf,
        std::slice::from_ref(&m1),
        &m1,
        Some(tab),
    );
    assert!(f
        .sidebar
        .read_with(&f.visual, |sidebar, _| sidebar.link_picker.is_some()));
    assert_eq!(
        f.linked("m3"),
        None,
        "nothing links before a chat is picked"
    );
    f.visual.simulate_keystrokes("escape");
    f.visual.run_until_parked();
    assert!(f
        .sidebar
        .read_with(&f.visual, |sidebar, _| sidebar.link_picker.is_none()));
    assert_eq!(f.linked("m3"), None);
    assert_eq!(f.flat(), order_before);

    // Picked: the second chat, at the dropped place.
    let tab = f.tab.clone();
    f.drag_drop(
        &m3,
        DropKind::Leaf,
        std::slice::from_ref(&m1),
        &m1,
        Some(tab),
    );
    f.visual.simulate_keystrokes("down enter");
    f.visual.run_until_parked();
    assert_eq!(f.linked("m3").as_deref(), Some("chat2"));
    assert_eq!(
        f.groups().missions(&f.tab),
        [m1, m3],
        "after the mission it was dropped below"
    );
}

#[test]
fn a_chat_the_daemon_rejects_leaves_the_link_and_the_order_as_they_were() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat", "chat2"]);
    f.link("m1", "chat");
    let (m1, m3) = (f.node("m1"), f.node("m3"));
    let order_before = f.flat();

    let tab = f.tab.clone();
    f.drag_drop(
        &m3,
        DropKind::Leaf,
        std::slice::from_ref(&m1),
        &m1,
        Some(tab),
    );
    // The second chat is archived while the picker is open.
    let core = f
        .store
        .read_with(&f.visual, |store, _| store.test_core.clone());
    core.db
        .get()
        .unwrap()
        .execute_batch(
            "UPDATE sessions SET archived_at = '2026-11-03T00:00:00Z' WHERE id = 'chat2'",
        )
        .unwrap();
    f.visual.simulate_keystrokes("down enter");
    f.visual.run_until_parked();
    assert!(f
        .sidebar
        .read_with(&f.visual, |sidebar, _| sidebar.link_picker.is_none()));
    assert_eq!(f.linked("m3"), None, "the link was rejected");
    assert_eq!(f.flat(), order_before, "and the move with it");
    assert_eq!(f.groups().missions(&f.tab), std::slice::from_ref(&m1));
}

#[test]
fn link_to_chat_opens_as_a_submenu_beside_its_row_by_keyboard_and_mouse() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat"]);
    let m1 = f.node("m1");
    let open_menu = |f: &mut RootFixture| {
        let sidebar = f.sidebar.clone();
        let m1 = m1.clone();
        f.visual.update(|window, cx| {
            sidebar.update(cx, |sidebar, cx| {
                let node = sidebar
                    .source_nodes(cx)
                    .into_iter()
                    .find(|node| node.id == m1)
                    .unwrap();
                let summary = sidebar
                    .app_store
                    .read(cx)
                    .missions
                    .iter()
                    .find(|summary| summary.mission.id == "m1")
                    .cloned()
                    .unwrap();
                sidebar.open_mission_menu(
                    node,
                    summary.into(),
                    gpui::point(px(40.), px(40.)),
                    window,
                    cx,
                );
            })
        });
        f.visual.run_until_parked();
    };
    let state = |f: &mut RootFixture| {
        f.sidebar.read_with(&f.visual, |sidebar, _| {
            (
                sidebar.context_menu.is_some(),
                sidebar.link_picker.is_some(),
            )
        })
    };

    open_menu(&mut f);
    f.visual.simulate_keystrokes("down down right");
    f.visual.run_until_parked();
    assert_eq!(
        state(&mut f),
        (true, true),
        "the menu stays while its submenu opens"
    );
    f.visual.simulate_keystrokes("escape");
    f.visual.run_until_parked();
    assert_eq!(state(&mut f), (true, false), "Escape returns to the menu");

    let row = f.visual.debug_bounds("CONTEXT_MENU_ITEM_2").unwrap();
    f.visual
        .simulate_click(row.center(), gpui::Modifiers::default());
    f.visual.run_until_parked();
    assert_eq!(state(&mut f), (true, true));
    let picker = f.visual.debug_bounds("LINK_PICKER").unwrap();
    assert!(
        picker.left() >= row.right(),
        "beside its row: {picker:?} {row:?}"
    );

    // A press inside the submenu is the submenu's, not its backdrop's.
    let chat = f.visual.debug_bounds("LINK_PICKER_CHAT chat").unwrap();
    f.visual
        .simulate_click(chat.center(), gpui::Modifiers::default());
    f.visual.run_until_parked();
    assert_eq!(state(&mut f), (false, false), "picking closes both");
    assert_eq!(f.linked("m1").as_deref(), Some("chat"));
}

#[test]
fn the_mission_header_links_back_to_its_chat_and_follows_link_changes() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat"]);
    f.link("m1", "chat");
    let window = f.window;
    window
        .update(&mut f.visual, |root, window, cx| {
            root.open_mission("m1".into(), window, cx)
        })
        .unwrap();
    f.visual.run_until_parked();
    let caption = f.visual.debug_bounds("MISSION_LINKED_CHAT").unwrap();

    f.store.update(&mut f.visual, |store, _| {
        let summary = store
            .missions
            .iter_mut()
            .find(|s| s.mission.id == "m1")
            .unwrap();
        summary.mission.linked_chat.as_mut().unwrap().archived = true;
    });
    f.visual.update(|window, _| window.refresh());
    f.visual.run_until_parked();
    f.visual
        .simulate_click(caption.center(), gpui::Modifiers::default());
    f.visual.run_until_parked();
    let route = window
        .read_with(&f.visual, |root, _| root.route.clone())
        .unwrap();
    assert_eq!(
        route,
        AppRoute::Mission("m1".into()),
        "an archived chat opens nothing"
    );

    f.store.update(&mut f.visual, |store, _| {
        let summary = store
            .missions
            .iter_mut()
            .find(|s| s.mission.id == "m1")
            .unwrap();
        summary.mission.linked_chat.as_mut().unwrap().archived = false;
    });
    f.visual.update(|window, _| window.refresh());
    f.visual.run_until_parked();
    let caption = f.visual.debug_bounds("MISSION_LINKED_CHAT").unwrap();
    f.visual
        .simulate_click(caption.center(), gpui::Modifiers::default());
    f.visual.run_until_parked();
    let (route, active) = window
        .read_with(&f.visual, |root, _| {
            (
                root.route.clone(),
                root.tabs.active_tab_id().map(str::to_owned),
            )
        })
        .unwrap();
    assert_eq!(route, AppRoute::Chat, "the caption opens the chat");
    assert_eq!(active.as_deref(), Some(f.tab.as_str()));

    window
        .update(&mut f.visual, |root, window, cx| {
            root.open_mission("m1".into(), window, cx)
        })
        .unwrap();
    f.sidebar.update(&mut f.visual, |sidebar, cx| {
        sidebar.unlink_mission("m1".into(), cx)
    });
    f.visual.run_until_parked();
    f.visual.update(|window, _| window.refresh());
    f.visual.run_until_parked();
    assert!(
        f.visual.debug_bounds("MISSION_LINKED_CHAT").is_none(),
        "unlinking takes the caption away"
    );
}

#[test]
fn a_linked_missions_pin_reads_and_acts_on_its_group() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat"]);
    f.link("m1", "chat");
    let m1 = f.node("m1");
    let pinned = |f: &mut RootFixture| {
        let m1 = m1.clone();
        f.sidebar.update(&mut f.visual, |sidebar, cx| {
            let node = sidebar
                .source_nodes(cx)
                .into_iter()
                .find(|node| node.id == m1)
                .unwrap();
            sidebar.mission_menu_pinned(&node, cx)
        })
    };
    assert!(!pinned(&mut f));

    // The menu's Pin goes through the mission's node; the daemon pins the group.
    f.sidebar.update(&mut f.visual, |sidebar, cx| {
        let node = sidebar
            .source_nodes(cx)
            .into_iter()
            .find(|node| node.id == m1)
            .unwrap();
        sidebar.front_page.pin(&node, true).unwrap();
        sidebar.refresh_store(crate::app_store::StoreRefreshKind::All, cx);
    });
    f.visual.run_until_parked();
    assert_eq!(
        f.pinned(),
        [f.tab.clone()],
        "the group's chat is pinned, not the mission"
    );
    assert!(pinned(&mut f), "the mission's menu now offers Unpin");
}

#[test]
fn the_mission_header_pins_and_unpins_a_linked_missions_group() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat"]);
    f.link("m1", "chat");
    let window = f.window;
    window
        .update(&mut f.visual, |root, window, cx| {
            root.open_mission("m1".into(), window, cx)
        })
        .unwrap();
    f.visual.run_until_parked();
    let workspace = window
        .read_with(&f.visual, |root, _| root.mission_workspace.clone())
        .unwrap();
    let toggle = |f: &mut RootFixture| {
        let workspace = workspace.clone();
        f.visual.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.toggle_pin_for_test(window, cx)
            })
        });
        f.visual.run_until_parked();
    };
    let offered =
        |f: &mut RootFixture| workspace.read_with(&f.visual, |w, _| w.menu_pinned_for_test());
    assert!(!offered(&mut f), "the header offers Pin");

    toggle(&mut f);
    assert_eq!(f.pinned(), [f.tab.clone()], "Pin pins the group");
    assert!(offered(&mut f), "the header now offers Unpin");
    toggle(&mut f);
    assert!(f.pinned().is_empty(), "Unpin unpins the group");
    assert!(!offered(&mut f));

    // Pinned from the sidebar, the header follows.
    let tab = f.tab.clone();
    f.sidebar.update(&mut f.visual, |sidebar, cx| {
        let node = sidebar
            .source_nodes(cx)
            .into_iter()
            .find(|node| node.id == tab)
            .unwrap();
        sidebar.front_page.pin(&node, true).unwrap();
        sidebar.refresh_store(crate::app_store::StoreRefreshKind::All, cx);
    });
    f.visual.run_until_parked();
    assert!(offered(&mut f), "the header follows the sidebar's pin");
}

#[test]
fn a_remote_window_gates_link_and_unlink_drops_on_host_capabilities() {
    use crate::surfaces::remote_client::tests as remote_test;
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for capable in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
        remote_test::enable_missions(&fake);
        if !capable {
            fake.without_features(&["linkMission", "unlinkMission"]);
        }
        let mission = |id: &str, linked: bool| {
            let mut mission = serde_json::json!({"id":id,"project":null,"title":id,"crew":"Duo","state":"running","pinned":false,"members":[],"asks":[],"shells":[]});
            if linked {
                mission["linkedChat"] = serde_json::json!({"sessionId":"c","title":"Chat","archived":false,"reports":true});
            }
            mission
        };
        fake.set_overview(
            serde_json::from_value(serde_json::json!({
                "projects":[],
                "tabs":[
                    {"id":"chat-tab","project":null,"name":"Chat tab","pinned":false,"panes":{"session":"c"},"shells":[]},
                    {"id":"pinned-tab","project":null,"name":"Pinned tab","pinned":true,"panes":{"session":"d"},"shells":[]}
                ],
                "missions":[mission("linked-a", true), mission("linked-b", true), mission("flat", false)],
                "sessions":{
                    "c":{"kind":"chat","title":"Chat","runtime":"codex","status":"working","unread":false},
                    "d":{"kind":"chat","title":"Pinned","runtime":"codex","status":"working","unread":false}
                }
            }))
            .unwrap(),
        );
        let window = remote_test::add_window(&mut cx, &store, temp.path(), "remote-link-drops");
        window
            .update(&mut cx, |root, window, cx| {
                remote_test::choose_host(root, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let sidebar = window
            .read_with(&cx, |root, _| {
                root.remote_client.workspace().unwrap().sidebar.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.update(|_, cx| {
            sidebar.update(cx, |sidebar, cx| {
                let rows = sidebar.resolved_sidebar_rows(cx);
                let groups = LinkGroups::of(&rows);
                let node = |mission: &str| {
                    rows.iter()
                        .find_map(|row| match row {
                            SidebarRow::Mission { node, summary, .. }
                                if summary.mission.id == mission =>
                            {
                                Some(node.id.clone())
                            }
                            _ => None,
                        })
                        .unwrap()
                };
                let (a, b, flat) = (node("linked-a"), node("linked-b"), node("flat"));
                let tab = groups.head(&a).unwrap().to_owned();
                assert_eq!(groups.missions(&tab), [a.clone(), b.clone()]);
                let chats = vec![("c".to_owned(), "Chat".to_owned())];
                assert_eq!(
                    sidebar.can_link_drop(&flat, &tab, &chats, cx),
                    capable,
                    "capable={capable}"
                );
                assert!(
                    !sidebar.can_link_drop(&a, &tab, &chats, cx),
                    "already linked there"
                );
                let own_rows = sidebar
                    .scope_rows(&rows, None)
                    .into_iter()
                    .map(|row| row.node().id.clone())
                    .filter(|id| groups.head(id).is_none())
                    .collect::<Vec<_>>();
                let pinned = rows
                    .iter()
                    .filter(|row| row.node().pinned_position.is_some())
                    .map(|row| row.node().id.clone())
                    .collect::<Vec<_>>();
                let mut target = |sidebar: &mut Sidebar,
                                  dragged: &str,
                                  kind: DropKind,
                                  visible: &[String],
                                  hovered: &str,
                                  group: Option<String>| {
                    sidebar.update_row_drop_target(
                        dragged,
                        kind,
                        None,
                        visible,
                        hovered,
                        true,
                        format!("test:{hovered}"),
                        group,
                        cx,
                    );
                    sidebar.drop_target.is_some()
                };
                let last = own_rows.last().unwrap().clone();
                assert_eq!(
                    target(sidebar, &a, DropKind::Leaf, &own_rows, &last, None),
                    capable,
                    "drag out of the group (capable={capable})"
                );
                assert_eq!(
                    target(sidebar, &a, DropKind::Pinned, &pinned, &pinned[0], None),
                    capable,
                    "drag into the pinned rows (capable={capable})"
                );
                let missions = groups.missions(&tab).to_vec();
                assert_eq!(
                    target(
                        sidebar,
                        &flat,
                        DropKind::Leaf,
                        &missions,
                        &a,
                        Some(tab.clone())
                    ),
                    capable,
                    "drag into a group (capable={capable})"
                );
                assert!(
                    target(
                        sidebar,
                        &a,
                        DropKind::Leaf,
                        &missions,
                        &b,
                        Some(tab.clone())
                    ),
                    "a reorder within the group remains"
                );
                assert!(
                    target(sidebar, &flat, DropKind::Leaf, &own_rows, &last, None),
                    "and an unlinked mission still reorders"
                );
            })
        });
    }
}

#[test]
fn remote_mission_menus_and_picker_source_follow_host_capabilities_and_project_scope() {
    use crate::surfaces::remote_client::tests as remote_test;
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for capable in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
        remote_test::enable_missions(&fake);
        if !capable {
            fake.without_features(&["linkMission", "unlinkMission"]);
        }
        // Seed a colliding local project and chat in AppStore so the test proves the
        // remote picker reads only the host overview, never local store rows.
        store.update(&mut cx, |store, _| {
            store
                .projects
                .push(runner_core::protocol::project::ProjectRow {
                    id: "proj-a".into(),
                    name: "Local Colliding Project".into(),
                    cwd: "/tmp".into(),
                    position: 0,
                    created_at: "2026-11-13T00:00:00Z".into(),
                });
            let mut local_chat = chat("local-only-chat", "codex");
            local_chat.project_id = Some("proj-a".into());
            store.sessions.push(local_chat);
        });
        let host_overview: runner_core::companion::Overview =
            serde_json::from_value(serde_json::json!({
                "projects":[
                    {"id":"proj-a","name":"Host Project A"},
                    {"id":"proj-b","name":"Host Project B"}
                ],
                "tabs":[
                    {
                        "id":"proj-a-split",
                        "project":"proj-a",
                        "name":"Split tab",
                        "pinned":false,
                        "panes":{
                            "split":"column",
                            "sizes":[0.5, 0.5],
                            "a":{"session":"chat-1"},
                            "b":{"split":"row","sizes":[0.5, 0.5],"a":{"session":"shell-1"},"b":{"session":"chat-2"}}
                        },
                        "shells":["shell-1"]
                    },
                    {
                        "id":"proj-b-tab",
                        "project":"proj-b",
                        "name":"Other project tab",
                        "pinned":false,
                        "panes":{"session":"other-chat"},
                        "shells":[]
                    },
                    {
                        "id":"recents-tab",
                        "project":null,
                        "name":"Recents tab",
                        "pinned":false,
                        "panes":{"session":"recents-chat"},
                        "shells":[]
                    }
                ],
                "missions":[
                    {
                        "id":"linked-m",
                        "project":"proj-a",
                        "title":"Linked mission",
                        "crew":"Duo",
                        "state":"running",
                        "pinned":false,
                        "linkedChat":{"sessionId":"chat-1","title":"Host Chat 1","archived":false,"reports":true},
                        "members":[{"handle":"coder","role":"Coder","runtime":"codex","session":"member-1","lead":true,"status":"working","unread":false}],
                        "asks":[],
                        "shells":[]
                    },
                    {
                        "id":"flat-m",
                        "project":"proj-a",
                        "title":"Flat mission",
                        "crew":"Duo",
                        "state":"running",
                        "pinned":false,
                        "members":[],
                        "asks":[],
                        "shells":[]
                    }
                ],
                "sessions":{
                    "chat-1":{"kind":"chat","title":"Host Chat 1","runtime":"codex","status":"working","unread":false},
                    "chat-2":{"kind":"chat","title":"","runtime":"claude","status":"stopped","unread":false},
                    "shell-1":{"kind":"shell","title":"zsh","status":"working","unread":false},
                    "member-1":{"kind":"agent","title":"@coder","runtime":"codex","status":"working","unread":false},
                    "other-chat":{"kind":"chat","title":"Other Project Chat","runtime":"codex","status":"working","unread":false},
                    "recents-chat":{"kind":"chat","title":"Recents Chat","runtime":"codex","status":"working","unread":false}
                }
            }))
            .unwrap();
        fake.set_overview(host_overview);
        let window = remote_test::add_window(&mut cx, &store, temp.path(), "remote-link-menus");
        window
            .update(&mut cx, |root, window, cx| {
                remote_test::choose_host(root, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let sidebar = window
            .read_with(&cx, |root, _| {
                root.remote_client.workspace().unwrap().sidebar.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);

        let supported_actions = |visual: &mut VisualTestContext, mission_id: &str| {
            visual.update(|_, cx| {
                let s = sidebar.read(cx);
                let rows = s.resolved_sidebar_rows(cx);
                let (node, summary) = rows
                    .into_iter()
                    .find_map(|row| match row {
                        SidebarRow::Mission { node, summary, .. }
                            if summary.mission.id == mission_id =>
                        {
                            Some((node, summary))
                        }
                        _ => None,
                    })
                    .unwrap();
                menus::mission_menu_entries(
                    &node.id,
                    s.mission_menu_pinned(&node, cx),
                    &summary.mission.id,
                    summary.mission.title,
                    summary.mission.linked_chat.as_ref(),
                )
                .into_iter()
                .filter(|(_, action)| s.menu_action_supported(action, cx))
                .map(|(_, action)| action)
                .collect::<Vec<_>>()
            })
        };

        let linked_actions = supported_actions(&mut visual, "linked-m");
        assert_eq!(
            linked_actions
                .iter()
                .any(|a| matches!(a, SidebarMenuAction::LinkToChat(id) if id == "linked-m")),
            capable
        );
        assert_eq!(
            linked_actions
                .iter()
                .any(|a| matches!(a, SidebarMenuAction::UnlinkMission(id) if id == "linked-m")),
            capable
        );
        let flat_actions = supported_actions(&mut visual, "flat-m");
        assert_eq!(
            flat_actions
                .iter()
                .any(|a| matches!(a, SidebarMenuAction::LinkToChat(id) if id == "flat-m")),
            capable
        );
        assert!(!flat_actions
            .iter()
            .any(|a| matches!(a, SidebarMenuAction::UnlinkMission(_))));

        if !capable {
            continue;
        }

        visual.update(|window, cx| {
            sidebar.update(cx, |sidebar, cx| {
                let choices = sidebar.chat_choices(Some("proj-a"), None, cx);
                assert_eq!(
                    choices
                        .iter()
                        .map(|c| (c.session_id.as_str(), c.title.as_str(), c.missions))
                        .collect::<Vec<_>>(),
                    [("chat-1", "Host Chat 1", 1), ("chat-2", "claude", 0)],
                    "only direct agent chats in proj-a appear, with host titles and mission counts"
                );
                sidebar.open_link_picker(
                    "flat-m".into(),
                    None,
                    None,
                    super::link::PickerAnchor::Pointer,
                    window,
                    cx,
                );
            });
        });
        visual.update(|window, _| window.refresh());
        visual.run_until_parked();
        let placeholder = sidebar.read_with(&visual, |sidebar, cx| {
            sidebar
                .link_picker
                .as_ref()
                .unwrap()
                .read(cx)
                .search_placeholder(cx)
        });
        assert_eq!(placeholder, "Search chats in Host Project A");
        assert!(visual.debug_bounds("LINK_PICKER_CHAT chat-1").is_some());
        assert!(visual.debug_bounds("LINK_PICKER_CHAT chat-2").is_some());
        assert!(visual.debug_bounds("LINK_PICKER_CHAT shell-1").is_none());
        assert!(visual.debug_bounds("LINK_PICKER_CHAT member-1").is_none());
        assert!(visual.debug_bounds("LINK_PICKER_CHAT other-chat").is_none());
        assert!(visual
            .debug_bounds("LINK_PICKER_CHAT local-only-chat")
            .is_none());
    }
}

#[test]
fn remote_link_unlink_and_drag_wait_for_overview_push_and_surface_errors_without_regrouping() {
    use crate::surfaces::remote_client::tests as remote_test;
    use runner_core::protocol::ClientError;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
    remote_test::enable_missions(&fake);

    let build_overview = |m1_chat: Option<(&str, &str)>, m2_chat: Option<(&str, &str)>| {
        let mission_json = |id: &str, linked: Option<(&str, &str)>| {
            let mut m = serde_json::json!({
                "id": id,
                "project": null,
                "title": id,
                "crew": "Duo",
                "state": "running",
                "pinned": false,
                "members": [],
                "asks": [],
                "shells": []
            });
            if let Some((session_id, title)) = linked {
                m["linkedChat"] = serde_json::json!({
                    "sessionId": session_id,
                    "title": title,
                    "archived": false,
                    "reports": true
                });
            }
            m
        };
        serde_json::from_value(serde_json::json!({
            "projects": [],
            "tabs": [
                {
                    "id": "single-tab",
                    "project": null,
                    "name": "Single tab",
                    "pinned": false,
                    "panes": {"session": "c1"},
                    "shells": []
                },
                {
                    "id": "multi-tab",
                    "project": null,
                    "name": "Multi tab",
                    "pinned": false,
                    "panes": {
                        "split": "column",
                        "sizes": [0.5, 0.5],
                        "a": {"session": "c2"},
                        "b": {"session": "c3"}
                    },
                    "shells": []
                },
                {
                    "id": "pinned-tab",
                    "project": null,
                    "name": "Pinned tab",
                    "pinned": true,
                    "panes": {"session": "cp"},
                    "shells": []
                }
            ],
            "missions": [mission_json("m1", m1_chat), mission_json("m2", m2_chat)],
            "sessions": {
                "c1": {"kind": "chat", "title": "Chat 1", "runtime": "codex", "status": "working", "unread": false},
                "c2": {"kind": "chat", "title": "Chat 2", "runtime": "codex", "status": "working", "unread": false},
                "c3": {"kind": "chat", "title": "Chat 3", "runtime": "claude", "status": "working", "unread": false},
                "cp": {"kind": "chat", "title": "Pinned Chat", "runtime": "codex", "status": "working", "unread": false}
            }
        }))
        .unwrap()
    };

    fake.set_overview(build_overview(None, Some(("c2", "Chat 2"))));
    let window = remote_test::add_window(&mut cx, &store, temp.path(), "remote-link-actions");
    window
        .update(&mut cx, |root, window, cx| {
            remote_test::choose_host(root, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let local_calls_before = fake.local_call_count();
    let sidebar = window
        .read_with(&cx, |root, _| {
            root.remote_client.workspace().unwrap().sidebar.clone()
        })
        .unwrap();
    let window_entity = window;
    let mut visual = VisualTestContext::from_window(window.into(), &cx);

    let group_head = |visual: &VisualTestContext, mission_id: &str| {
        sidebar.read_with(visual, |sidebar, cx| {
            let rows = sidebar.resolved_sidebar_rows(cx);
            let groups = LinkGroups::of(&rows);
            let node_id = rows
                .iter()
                .find_map(|row| match row {
                    SidebarRow::Mission { node, summary, .. }
                        if summary.mission.id == mission_id =>
                    {
                        Some(node.id.clone())
                    }
                    _ => None,
                })
                .unwrap();
            groups.head(&node_id).map(str::to_owned)
        })
    };
    let action_error = |visual: &VisualTestContext| {
        window_entity
            .read_with(visual, |root, _| {
                root.remote_client.workspace().unwrap().action_error.clone()
            })
            .unwrap()
    };

    // 1. Menu LinkToChat -> picker selection sends linkMission, does NOT regroup optimistically,
    //    and regroups only once the host pushes an updated overview.
    visual.update(|window, cx| {
        sidebar.update(cx, |sidebar, cx| {
            sidebar.handle_sidebar_menu_action(
                SidebarMenuAction::LinkToChat("m1".into()),
                window,
                cx,
            );
        });
    });
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    let c1_row = visual.debug_bounds("LINK_PICKER_CHAT c1").unwrap();
    visual.simulate_click(c1_row.center(), gpui::Modifiers::default());
    visual.run_until_parked();

    assert_eq!(
        remote_test::action_trace(&fake, "linkMission"),
        [serde_json::json!({"id": 0, "type": "linkMission", "mission": "m1", "session": "c1"})]
    );
    assert_eq!(
        group_head(&visual, "m1"),
        None,
        "remote sidebar must not optimistically regroup before host overview arrives"
    );
    fake.set_overview(build_overview(
        Some(("c1", "Chat 1")),
        Some(("c2", "Chat 2")),
    ));
    visual.run_until_parked();
    assert_eq!(
        group_head(&visual, "m1").as_deref(),
        Some("single-tab"),
        "regroups once host overview includes linkedChat"
    );
    assert_eq!(action_error(&visual), None);

    // 2. Menu UnlinkMission sends unlinkMission, waits for overview push, then ungroups.
    visual.update(|window, cx| {
        sidebar.update(cx, |sidebar, cx| {
            sidebar.handle_sidebar_menu_action(
                SidebarMenuAction::UnlinkMission("m1".into()),
                window,
                cx,
            );
        });
    });
    visual.run_until_parked();
    assert_eq!(
        remote_test::action_trace(&fake, "unlinkMission"),
        [serde_json::json!({"id": 0, "type": "unlinkMission", "mission": "m1"})]
    );
    assert_eq!(
        group_head(&visual, "m1").as_deref(),
        Some("single-tab"),
        "remote sidebar must not optimistically ungroup before host overview arrives"
    );
    fake.set_overview(build_overview(None, Some(("c2", "Chat 2"))));
    visual.run_until_parked();
    assert_eq!(group_head(&visual, "m1"), None);

    // 3. Drag onto a single-chat row sends linkMission; drag into a multi-chat group opens the
    //    picker with PendingMove and sends linkMission followed by move.
    let (m1_node, m2_node) = sidebar.read_with(&visual, |sidebar, cx| {
        let rows = sidebar.resolved_sidebar_rows(cx);
        let find = |id: &str| {
            rows.iter()
                .find_map(|row| match row {
                    SidebarRow::Mission { node, summary, .. } if summary.mission.id == id => {
                        Some(node.id.clone())
                    }
                    _ => None,
                })
                .unwrap()
        };
        (find("m1"), find("m2"))
    });

    visual.update(|window, cx| {
        sidebar.update(cx, |sidebar, cx| {
            sidebar.set_link_drop(
                super::link::LinkDrop {
                    mission_node: m1_node.clone(),
                    tab_node: "single-tab".into(),
                    chats: vec![("c1".into(), "Chat 1".into())],
                },
                cx,
            );
            sidebar.commit_sidebar_drop(&m1_node, window, cx);
        });
    });
    visual.run_until_parked();
    assert_eq!(remote_test::action_trace(&fake, "linkMission").len(), 2);
    assert_eq!(
        remote_test::action_trace(&fake, "linkMission")[1],
        serde_json::json!({"id": 0, "type": "linkMission", "mission": "m1", "session": "c1"})
    );

    // Drag `m1` into `multi-tab`'s link group (`m2` is already linked to `c2` in `multi-tab`).
    visual.update(|window, cx| {
        sidebar.update(cx, |sidebar, cx| {
            sidebar.update_row_drop_target(
                &m1_node,
                DropKind::Leaf,
                None,
                std::slice::from_ref(&m2_node),
                &m2_node,
                true,
                "test:multi".into(),
                Some("multi-tab".into()),
                cx,
            );
            assert!(sidebar.drop_target.is_some());
            sidebar.commit_sidebar_drop(&m1_node, window, cx);
            assert!(
                sidebar.link_picker.is_some(),
                "dropping into a multi-chat group opens the chat picker"
            );
        });
    });
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();

    // First verify a rejected linkMission with PendingMove surfaces action_error, skips move,
    // and leaves grouping untouched.
    let moves_before_reject = remote_test::action_trace(&fake, "move").len();
    fake.queue_result(
        "linkMission",
        Err(ClientError::msg("The host could not complete the request.")),
    );
    let c3_row = visual.debug_bounds("LINK_PICKER_CHAT c3").unwrap();
    visual.simulate_click(c3_row.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        action_error(&visual).as_deref(),
        Some("The host could not complete the request.")
    );
    assert_eq!(
        remote_test::action_trace(&fake, "move").len(),
        moves_before_reject,
        "a rejected linkMission short-circuits before sending move"
    );
    assert_eq!(group_head(&visual, "m1"), None);

    // Now succeed with the multi-chat group drop -> sends linkMission then move.
    visual.update(|window, cx| {
        sidebar.update(cx, |sidebar, cx| {
            sidebar.update_row_drop_target(
                &m1_node,
                DropKind::Leaf,
                None,
                std::slice::from_ref(&m2_node),
                &m2_node,
                true,
                "test:multi".into(),
                Some("multi-tab".into()),
                cx,
            );
            sidebar.commit_sidebar_drop(&m1_node, window, cx);
        });
    });
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    let c3_row = visual.debug_bounds("LINK_PICKER_CHAT c3").unwrap();
    visual.simulate_click(c3_row.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert_eq!(action_error(&visual), None);
    assert_eq!(
        remote_test::action_trace(&fake, "linkMission")
            .last()
            .unwrap()["session"],
        "c3"
    );
    assert_eq!(
        remote_test::action_trace(&fake, "move").len(),
        moves_before_reject + 1
    );

    // 4. Drag `m2` out of its group: first verify rejected unlinkMission leaves grouping unchanged
    //    and skips move/pin, then verify accepted drops send unlinkMission + move (and + pin + reorderPinned).
    fake.queue_result(
        "unlinkMission",
        Err(ClientError::msg("Cannot unlink right now.")),
    );
    visual.update(|window, cx| {
        sidebar.update(cx, |sidebar, cx| {
            let own = vec![
                "single-tab".to_owned(),
                "multi-tab".to_owned(),
                m1_node.clone(),
            ];
            sidebar.update_row_drop_target(
                &m2_node,
                DropKind::Leaf,
                None,
                &own,
                "single-tab",
                true,
                "test:unlink-reject".into(),
                None,
                cx,
            );
            sidebar.commit_sidebar_drop(&m2_node, window, cx);
        });
    });
    visual.run_until_parked();
    assert_eq!(
        action_error(&visual).as_deref(),
        Some("Cannot unlink right now.")
    );
    assert_eq!(
        remote_test::action_trace(&fake, "move").len(),
        moves_before_reject + 1,
        "rejected unlinkMission skips move"
    );
    assert_eq!(group_head(&visual, "m2").as_deref(), Some("multi-tab"));

    // Accepted drag out to pinned rows sends unlinkMission, pin, and reorderPinned.
    let pins_before = remote_test::action_trace(&fake, "pin").len();
    let reorders_before = remote_test::action_trace(&fake, "reorderPinned").len();
    visual.update(|window, cx| {
        sidebar.update(cx, |sidebar, cx| {
            let pinned = vec!["pinned-tab".to_owned()];
            sidebar.update_row_drop_target(
                &m2_node,
                DropKind::Pinned,
                None,
                &pinned,
                "pinned-tab",
                true,
                "test:unlink-pin".into(),
                None,
                cx,
            );
            sidebar.commit_sidebar_drop(&m2_node, window, cx);
        });
    });
    visual.run_until_parked();
    assert_eq!(action_error(&visual), None);
    assert_eq!(
        remote_test::action_trace(&fake, "unlinkMission")
            .last()
            .unwrap()["mission"],
        "m2"
    );
    assert_eq!(
        remote_test::action_trace(&fake, "pin").len(),
        pins_before + 1
    );
    assert_eq!(
        remote_test::action_trace(&fake, "reorderPinned").len(),
        reorders_before + 1
    );
    assert_eq!(
        fake.local_call_count(),
        local_calls_before,
        "remote link/unlink operations never call the local daemon"
    );
}

#[test]
fn a_group_takes_the_selected_border_while_one_of_its_rows_is_selected_without_moving() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat"]);
    f.link("m1", "chat");
    f.visual.update(|window, _| window.refresh());
    f.visual.run_until_parked();
    let selector = leak(format!("LINK_GROUP_OUTLINE {}", f.tab));
    let outline = f.visual.debug_bounds(selector).unwrap();
    assert_eq!(
        crate::theme_snapshot::recorded_border("LINK_GROUP_OUTLINE_STYLE"),
        Some(theme::border())
    );

    let window = f.window;
    window
        .update(&mut f.visual, |root, window, cx| {
            root.open_mission("m1".into(), window, cx)
        })
        .unwrap();
    f.visual.run_until_parked();
    assert_eq!(
        crate::theme_snapshot::recorded_border("LINK_GROUP_OUTLINE_STYLE"),
        Some(theme::chrome_selected_border()),
        "a selected linked mission selects its group's outline"
    );
    assert_eq!(
        f.visual.debug_bounds(selector),
        Some(outline),
        "nothing moves"
    );
    assert_eq!(
        crate::theme_snapshot::recorded_border("LINKED_MISSION_ROW_STYLE"),
        Some(gpui::transparent_black()),
        "its row shows the fill and no border of its own"
    );

    crate::theme_snapshot::clear_border("LINKED_MISSION_ROW_STYLE");
    f.sidebar.update(&mut f.visual, |sidebar, cx| {
        sidebar.update_app_settings(cx, true, |settings| {
            settings.sidebar_active_only = true;
            true
        });
    });
    f.visual.update(|window, _| window.refresh());
    f.visual.run_until_parked();
    let node = f.node("m1");
    assert!(f
        .visual
        .debug_bounds(leak(format!("SIDEBAR_ROW active-{node}")))
        .is_some());
    assert_eq!(
        crate::theme_snapshot::recorded_border("LINKED_MISSION_ROW_STYLE"),
        Some(gpui::transparent_black()),
        "nor in Active"
    );
}

#[test]
fn only_the_last_linked_mission_rounds_into_the_outlines_corners() {
    let corners = |shape: MissionShape| {
        let mut row = shape.apply(gpui::div().id("row"));
        let radii = row.style().corner_radii.clone();
        [
            radii.top_left,
            radii.top_right,
            radii.bottom_left,
            radii.bottom_right,
        ]
    };
    let radius = |px: f32| Some(gpui::AbsoluteLength::from(rems(px / 16.)));
    let square = radius(0.);
    for shape in [
        MissionShape::projects(26., false),
        MissionShape::active(false),
    ] {
        assert!(
            corners(shape) == [square; 4],
            "a middle row meets the outline square"
        );
    }
    assert!(
        corners(MissionShape::projects(26., true)) == [square, square, radius(5.), radius(5.)],
        "Projects' row starts at the outline's inner edge"
    );
    assert!(
        corners(MissionShape::active(true)) == [square, square, radius(6.), radius(6.)],
        "Active's lies under the outline itself"
    );
}

#[test]
fn a_pointer_drag_from_left_of_a_linked_missions_flag_reorders_it_and_takes_it_out() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat"]);
    f.link("m1", "chat");
    f.link("m2", "chat");
    f.visual.update(|window, _| window.refresh());
    f.visual.run_until_parked();
    let bounds = |f: &mut RootFixture, selector: String| {
        f.visual
            .debug_bounds(leak(selector.clone()))
            .unwrap_or_else(|| panic!("missing {selector}"))
    };
    let mission_row = |f: &mut RootFixture, mission: &str| {
        bounds(f, format!("SIDEBAR_ROW sidebar-mission-{mission}"))
    };
    let tab = f.tab.clone();
    let outline = bounds(&mut f, format!("LINK_GROUP_OUTLINE {tab}"));
    let drag = |f: &mut RootFixture, mission: &str, to: gpui::Point<gpui::Pixels>| {
        let row = mission_row(f, mission);
        let flag = bounds(f, format!("SIDEBAR_MARK sidebar-mission-{mission}"));
        let from = gpui::point(outline.left() + px(2.), row.center().y);
        assert!(from.x < flag.left(), "{row:?} {flag:?}");
        let none = gpui::Modifiers::default();
        let left = gpui::MouseButton::Left;
        f.visual.simulate_mouse_move(from, None, none);
        f.visual.simulate_mouse_down(from, left, none);
        let started = gpui::point(from.x, from.y + px(4.));
        f.visual.simulate_mouse_move(started, Some(left), none);
        f.visual.simulate_mouse_move(to, Some(left), none);
        f.visual.simulate_mouse_up(to, left, none);
        f.visual.run_until_parked();
        f.visual.update(|window, _| window.refresh());
        f.visual.run_until_parked();
    };
    let m1 = f.node("m1");
    let order = f.groups().missions(&tab).to_vec();
    let (top, bottom) = if order[0] == m1 {
        ("m1", "m2")
    } else {
        ("m2", "m1")
    };

    let below = mission_row(&mut f, bottom);
    let to = gpui::point(below.center().x, below.bottom() - px(2.));
    drag(&mut f, top, to);
    assert_eq!(
        f.groups().missions(&tab),
        [order[1].clone(), order[0].clone()],
        "a drag within the group reorders it"
    );
    assert_eq!(f.linked(top).as_deref(), Some("chat"), "and keeps the link");

    let flat = mission_row(&mut f, "m3");
    let to = gpui::point(flat.center().x, flat.bottom() - px(2.));
    drag(&mut f, bottom, to);
    assert_eq!(f.linked(bottom), None, "a drag out of the group unlinks it");
    assert_eq!(f.linked(top).as_deref(), Some("chat"));
}

#[test]
fn a_linked_missions_whole_row_answers_and_renaming_it_keeps_its_place() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat"]);
    f.link("m1", "chat");
    f.visual.update(|window, _| window.refresh());
    f.visual.run_until_parked();
    let bounds = |f: &mut RootFixture, selector: String| {
        f.visual
            .debug_bounds(leak(selector.clone()))
            .unwrap_or_else(|| panic!("missing {selector}"))
    };
    let row = bounds(&mut f, "SIDEBAR_ROW sidebar-mission-m1".into());
    let flag = bounds(&mut f, "SIDEBAR_MARK sidebar-mission-m1".into());
    let tab = f.tab.clone();
    let outline = bounds(&mut f, format!("LINK_GROUP_OUTLINE {tab}"));
    let beside = gpui::point(outline.left() + px(2.), row.center().y);
    assert!(
        beside.x < flag.left() && row.contains(&beside),
        "{row:?} {flag:?}"
    );

    f.visual
        .simulate_mouse_move(beside, None, gpui::Modifiers::default());
    f.visual.run_until_parked();
    let node = f.node("m1");
    f.sidebar.read_with(&f.visual, |sidebar, _| {
        assert_eq!(sidebar.hovered_row.as_deref(), Some(node.as_str()), "hover");
    });
    f.visual.simulate_click(beside, gpui::Modifiers::default());
    f.visual.run_until_parked();
    f.sidebar.read_with(&f.visual, |sidebar, cx| {
        assert!(sidebar.mission_selected("m1", cx), "a click opens it");
    });
    f.visual
        .simulate_mouse_down(beside, gpui::MouseButton::Right, gpui::Modifiers::default());
    f.visual.run_until_parked();
    f.sidebar.read_with(&f.visual, |sidebar, _| {
        assert!(
            sidebar.context_menu.is_some(),
            "a right-click opens its menu"
        );
    });
    f.visual.update(|window, cx| {
        f.sidebar
            .update(cx, |sidebar, cx| sidebar.close_context_menu(window, cx))
    });
    f.visual.run_until_parked();

    f.visual.update(|window, cx| {
        f.sidebar.update(cx, |sidebar, cx| {
            sidebar.begin_sidebar_rename(
                super::SidebarRenameTarget::Mission {
                    mission_id: "m1".into(),
                    original: "Mission".into(),
                },
                "Mission".into(),
                "Mission name".into(),
                window,
                cx,
            )
        })
    });
    f.visual.update(|window, _| window.refresh());
    f.visual.run_until_parked();
    let field = bounds(&mut f, "SIDEBAR_ROW sidebar-rename-m1".into());
    let mark = bounds(&mut f, "SIDEBAR_MARK sidebar-rename-m1".into());
    assert_eq!(
        (field.left(), field.right(), mark.left()),
        (row.left(), row.right(), flag.left()),
        "renaming, the row keeps its span and its flag: {field:?} {mark:?} {row:?} {flag:?}"
    );
    assert_eq!(
        crate::theme_snapshot::recorded_border("LINKED_MISSION_ROW_STYLE"),
        Some(gpui::transparent_black()),
        "and no border of its own"
    );
}

#[test]
fn a_groups_line_starts_at_the_chats_mark_and_its_rows_align_with_flat_rows() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat"]);
    f.link("m1", "chat");
    f.link("m2", "chat");
    f.visual.update(|window, _| window.refresh());
    f.visual.run_until_parked();
    let tab = f.tab.clone();
    let bounds = |f: &mut RootFixture, selector: String| {
        f.visual
            .debug_bounds(leak(selector.clone()))
            .unwrap_or_else(|| panic!("missing {selector}"))
    };
    let near = |a: gpui::Pixels, b: gpui::Pixels| (a - b).abs() <= px(1.);
    let head = bounds(&mut f, format!("SIDEBAR_ROW sidebar-tab-{tab}"));
    let flat = bounds(&mut f, "SIDEBAR_ROW sidebar-mission-m3".into());
    let (m1, m2) = (
        bounds(&mut f, "SIDEBAR_ROW sidebar-mission-m1".into()),
        bounds(&mut f, "SIDEBAR_ROW sidebar-mission-m2".into()),
    );
    let (m1_flag, m2_flag) = (
        bounds(&mut f, "SIDEBAR_MARK sidebar-mission-m1".into()),
        bounds(&mut f, "SIDEBAR_MARK sidebar-mission-m2".into()),
    );
    let (first, last, first_flag, last_flag) = if m1.top() < m2.top() {
        (m1, m2, m1_flag, m2_flag)
    } else {
        (m2, m1, m2_flag, m1_flag)
    };

    // The chat's row moves right to give the chevron room (Jason, 2026-11-03).
    let head_mark = bounds(&mut f, format!("SIDEBAR_MARK sidebar-tab-{tab}"));
    let flat_mark = bounds(&mut f, "SIDEBAR_MARK sidebar-mission-m3".into());
    let mark = head_mark.center().x;
    assert!(
        near(head_mark.left(), flat_mark.left() + px(9.)),
        "the chat's mark sits right of a flat row's to fit the chevron: {head_mark:?} {flat_mark:?}"
    );
    for flag in [first_flag, last_flag] {
        assert!(
            near(flag.left(), head_mark.left() + px(20.)),
            "a mission's flag sits 20 px right of the chat's mark: {flag:?} {head_mark:?}"
        );
    }
    assert!(
        near(flat_mark.left(), flat.left() + px(11.)),
        "a flat row keeps its padding: {flat_mark:?} {flat:?}"
    );
    for row in [head, first, last] {
        assert!(
            near(row.right(), flat.right()),
            "trailing icons line up with flat rows: {row:?} {flat:?}"
        );
    }

    let toggle = bounds(&mut f, format!("LINK_GROUP_TOGGLE {tab}"));
    let outline = bounds(&mut f, format!("LINK_GROUP_OUTLINE {tab}"));
    assert!(
        near(toggle.center().y, head.center().y),
        "the chevron is centred on the chat's row"
    );
    assert!(
        near(head.left(), outline.left() + px(1.)) && near(head.right(), outline.right()),
        "the chat row's box spans the group's inner width: {head:?} {outline:?}"
    );
    for row in [first, last] {
        assert!(
            row.left() == head.left() && near(row.right(), outline.right()),
            "a linked mission's row spans it too: {row:?} {head:?} {outline:?}"
        );
    }
    assert!(
        toggle.left() >= head.left() && toggle.right() <= head_mark.left(),
        "the chevron sits inside the chat row's box, before its mark"
    );
    assert!(
        near(toggle.left() - (outline.left() + px(1.)), px(9.)),
        "the chevron sits 9 px inside the outline: {toggle:?} {outline:?}"
    );
    assert!(
        head_mark.left() - toggle.right() >= px(6.) - px(0.5),
        "at least 6 px from the chevron to the chat's mark: {toggle:?} {head_mark:?}"
    );

    let stub = bounds(&mut f, format!("LINK_STUB {tab}"));
    assert!(
        near(stub.center().x, mark),
        "the line starts at the chat's mark: {stub:?}"
    );
    assert!(stub.top() < head.bottom() && stub.bottom() >= head.bottom());
    let line = bounds(&mut f, format!("LINK_LINE {tab} 0"));
    assert!(near(line.center().x, mark));
    assert!(
        line.top() <= first.top() && line.bottom() >= first.bottom(),
        "the line runs past the first mission: {line:?} {first:?}"
    );
    assert!(line.top() <= stub.bottom(), "the line continues the stub");
    for (index, row, flag) in [(0, first, first_flag), (1, last, last_flag)] {
        let branch = bounds(&mut f, format!("LINK_BRANCH {tab} {index}"));
        assert!(near(branch.left(), mark - px(0.5)), "{index}: {branch:?}");
        assert!(
            branch.right() < flag.left(),
            "{index}: the branch ends before the flag"
        );
        let y = if index == 1 {
            branch.bottom()
        } else {
            branch.center().y
        };
        assert!(
            near(y, row.center().y),
            "{index}: the branch meets the mission's row"
        );
    }
    crate::theme_snapshot::assert_fill_color(&mut f.visual, "LINK_STUB_FILL", Some(theme::faint()));
}

#[test]
fn a_pinned_groups_chevron_clears_the_pin_and_its_line_follows_the_drawn_mark() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat"]);
    f.link("m1", "chat");
    f.link("m2", "chat");
    let tab = f.tab.clone();
    let pin_tab = tab.clone();
    f.sidebar.update(&mut f.visual, |sidebar, cx| {
        let node = sidebar
            .source_nodes(cx)
            .into_iter()
            .find(|node| node.id == pin_tab)
            .unwrap();
        sidebar.front_page.pin(&node, true).unwrap();
        sidebar.refresh_store(crate::app_store::StoreRefreshKind::All, cx);
    });
    f.visual.run_until_parked();
    let redraw = |f: &mut RootFixture| {
        f.visual.update(|window, _| window.refresh());
        f.visual.run_until_parked();
    };
    redraw(&mut f);
    let bounds = |f: &mut RootFixture, selector: String| {
        f.visual
            .debug_bounds(leak(selector.clone()))
            .unwrap_or_else(|| panic!("missing {selector}"))
    };
    let near = |a: gpui::Pixels, b: gpui::Pixels| (a - b).abs() <= px(1.);

    // Expanded: the chevron clears the pin; the line starts at the mark after it.
    let head = bounds(&mut f, format!("SIDEBAR_ROW sidebar-tab-{tab}"));
    let pin = bounds(&mut f, format!("SIDEBAR_PIN sidebar-tab-{tab}"));
    let mark = bounds(&mut f, format!("SIDEBAR_MARK sidebar-tab-{tab}"));
    let toggle = bounds(&mut f, format!("LINK_GROUP_TOGGLE {tab}"));
    let outline = bounds(&mut f, format!("LINK_GROUP_OUTLINE {tab}"));
    assert!(pin.right() <= mark.left(), "the pin leads the mark");
    assert!(
        near(toggle.left() - (outline.left() + px(1.)), px(9.)),
        "the chevron sits 9 px inside the outline: {toggle:?} {outline:?}"
    );
    assert!(
        pin.left() - toggle.right() >= px(6.) - px(0.5),
        "at least 6 px from the chevron to the pin: {toggle:?} {pin:?}"
    );
    assert!(near(toggle.center().y, head.center().y));
    let stub = bounds(&mut f, format!("LINK_STUB {tab}"));
    assert!(near(stub.center().x, mark.center().x), "{stub:?} {mark:?}");
    for mission in ["m1", "m2"] {
        let flag = bounds(&mut f, format!("SIDEBAR_MARK sidebar-mission-{mission}"));
        assert!(
            near(flag.left(), mark.left() + px(20.)),
            "{mission}: {flag:?} {mark:?}"
        );
        let row = bounds(&mut f, format!("SIDEBAR_ROW sidebar-mission-{mission}"));
        assert!(
            row.left() == head.left() && near(row.right(), outline.right()),
            "{mission}: the row still spans the group: {row:?} {head:?} {outline:?}"
        );
    }
    for index in 0..2 {
        let branch = bounds(&mut f, format!("LINK_BRANCH {tab} {index}"));
        assert!(
            near(branch.left(), mark.center().x - px(0.5)),
            "{index}: {branch:?}"
        );
    }

    // Renaming: the field shows no pin, and the line follows the mark it draws. The
    // row keeps its full-width box with the chevron inside it.
    let rename = |f: &mut RootFixture, on: bool| {
        let tab = f.tab.clone();
        f.visual.update(|window, cx| {
            f.sidebar.update(cx, |sidebar, cx| {
                if !on {
                    sidebar.rename = None;
                    cx.notify();
                    return;
                }
                sidebar.begin_sidebar_rename(
                    super::SidebarRenameTarget::Tab {
                        node_id: tab,
                        original: "Frontman".into(),
                        session_id: None,
                    },
                    "Frontman".into(),
                    "Tab name".into(),
                    window,
                    cx,
                )
            })
        });
        f.visual.update(|window, _| window.refresh());
        f.visual.run_until_parked();
    };
    rename(&mut f, true);
    let renamed = bounds(&mut f, format!("SIDEBAR_MARK sidebar-rename-{tab}"));
    let stub = bounds(&mut f, format!("LINK_STUB {tab}"));
    assert!(
        near(stub.center().x, renamed.center().x),
        "{stub:?} {renamed:?}"
    );
    let flag = bounds(&mut f, "SIDEBAR_MARK sidebar-mission-m1".into());
    assert!(near(flag.left(), renamed.left() + px(20.)));
    let field = bounds(&mut f, format!("SIDEBAR_ROW sidebar-rename-{tab}"));
    let toggle = bounds(&mut f, format!("LINK_GROUP_TOGGLE {tab}"));
    assert!(
        near(field.left(), outline.left() + px(1.)) && near(field.right(), outline.right()),
        "renaming, the chat row's box spans the group's inner width: {field:?} {outline:?}"
    );
    assert!(
        toggle.left() >= field.left() && renamed.left() - toggle.right() >= px(6.) - px(0.5),
        "renaming, the chevron stays inside the row's box, before its mark: {toggle:?} {renamed:?}"
    );
    rename(&mut f, false);

    // Collapsed: the chevron still clears the pin.
    let collapse_tab = tab.clone();
    f.sidebar.update(&mut f.visual, |sidebar, cx| {
        sidebar.update_app_settings(cx, true, move |settings| {
            settings
                .sidebar_collapsed_link_groups
                .insert(collapse_tab.clone());
            true
        });
    });
    redraw(&mut f);
    assert!(f
        .visual
        .debug_bounds(leak(format!("LINK_GROUP_SUMMARY {tab}")))
        .is_some());
    let head = bounds(&mut f, format!("SIDEBAR_ROW sidebar-tab-{tab}"));
    let pin = bounds(&mut f, format!("SIDEBAR_PIN sidebar-tab-{tab}"));
    let toggle = bounds(&mut f, format!("LINK_GROUP_TOGGLE {tab}"));
    let outline = bounds(&mut f, format!("LINK_GROUP_OUTLINE {tab}"));
    let summary = bounds(&mut f, format!("LINK_GROUP_SUMMARY {tab}"));
    let mark = bounds(&mut f, format!("SIDEBAR_MARK sidebar-tab-{tab}"));
    assert!(
        head.contains(&summary.center()) && near(summary.left(), mark.right() + px(6.)),
        "collapsed, the summary is the chat row's second line, in its label column: {summary:?} {mark:?} {head:?}"
    );
    assert!(
        near(head.left(), outline.left() + px(1.)) && near(head.right(), outline.right()),
        "collapsed, the group is one row: {head:?} {outline:?}"
    );
    assert!(
        near(toggle.left() - (outline.left() + px(1.)), px(9.)),
        "collapsed: {toggle:?} {outline:?}"
    );
    assert!(
        pin.left() - toggle.right() >= px(6.) - px(0.5),
        "collapsed: {toggle:?} {pin:?}"
    );
    assert!(near(toggle.center().y, head.center().y));
    let group_state = |f: &mut RootFixture| {
        [
            "None",
            "Unavailable",
            "Unread",
            "Working",
            "NeedsYou",
            "Error",
        ]
        .into_iter()
        .find_map(|state| {
            f.visual
                .debug_bounds(leak(format!("LINK_GROUP_STATE {} {state}", f.tab)))
        })
        .expect("the collapsed head shows its group's state")
    };
    assert!(head.contains(&group_state(&mut f).center()));

    // Renaming a collapsed group: the field replaces only the title.
    rename(&mut f, true);
    let field = bounds(&mut f, format!("SIDEBAR_ROW sidebar-rename-{tab}"));
    let mark = bounds(&mut f, format!("SIDEBAR_MARK sidebar-rename-{tab}"));
    let summary = bounds(&mut f, format!("LINK_GROUP_SUMMARY {tab}"));
    let toggle = bounds(&mut f, format!("LINK_GROUP_TOGGLE {tab}"));
    assert!(
        near(field.left(), outline.left() + px(1.)) && near(field.right(), outline.right()),
        "collapsed and renaming, the group is still one row: {field:?} {outline:?}"
    );
    assert!(
        field.contains(&summary.center()) && near(summary.left(), mark.right() + px(6.)),
        "the summary stays under the field: {summary:?} {mark:?}"
    );
    assert!(field.contains(&group_state(&mut f).center()));
    assert!(toggle.left() >= field.left() && toggle.right() <= mark.left());
}

#[test]
fn archiving_a_linked_chat_prompts_cancels_and_archives_missions_while_unlinked_stays_instant() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat"]);

    // Unlinked single-chat sidebar archive stays immediate with no confirmation dialog.
    let unlinked_core = f
        .store
        .read_with(&f.visual, |store, _| store.test_core.clone());
    let unlinked_tab = {
        let conn = unlinked_core.db.get().unwrap();
        conn.execute_batch(
            "INSERT INTO sessions (id, status, agent_runtime, agent_command, title, started_at)
             VALUES ('unlinked', 'stopped', 'codex', 'codex', 'Unlinked', '2026-11-03T00:01:00Z')",
        )
        .unwrap();
        let layout = PaneLayout::single(Some("unlinked"), &["unlinked".to_owned()]);
        runner_daemon::repo::node::create_tab(
            &conn,
            None,
            "Unlinked",
            10,
            &layout.serialize().unwrap(),
        )
        .unwrap()
        .id
    };
    f.store.update(&mut f.visual, |store, cx| {
        store.refresh(crate::app_store::StoreRefreshKind::All, cx);
    });
    f.window
        .update(&mut f.visual, |root, _, cx| {
            root.reload_tabs(cx).unwrap();
        })
        .unwrap();
    f.visual.run_until_parked();

    f.window
        .update(&mut f.visual, |root, window, cx| {
            root.request_archive_all(
                Some(&unlinked_tab),
                vec!["unlinked".into()],
                crate::ArchiveAllSource::Sidebar,
                window,
                cx,
            );
            assert!(
                root.terminal_close_confirm.is_none(),
                "unlinked chat without shells archives immediately"
            );
        })
        .unwrap();
    f.visual.run_until_parked();
    assert!(
        runner_daemon::ops::session::session_get(&unlinked_core, "unlinked")
            .unwrap()
            .unwrap()
            .archived_at
            .is_some(),
        "unlinked chat was archived directly"
    );

    // Link m1 and m2 to `chat`; leave m3 unlinked.
    f.link("m1", "chat");
    f.link("m2", "chat");
    let tab = f.tab.clone();

    // Sidebar ArchiveTab on the linked group must open Option A's ConfirmDialog instead of archiving.
    f.window
        .update(&mut f.visual, |root, window, cx| {
            root.request_archive_all(
                Some(&tab),
                vec!["chat".into()],
                crate::ArchiveAllSource::Sidebar,
                window,
                cx,
            );
            assert!(
                root.terminal_close_confirm.is_some(),
                "linked chat opens confirmation"
            );
        })
        .unwrap();
    f.visual.update(|window, _| window.refresh());
    f.visual.run_until_parked();
    assert!(f
        .visual
        .debug_bounds("CONFIRM_DIALOG_LINKED_MISSIONS")
        .is_some());
    assert!(f.visual.debug_bounds("CONFIRM_LINKED_MISSION m1").is_some());
    assert!(f.visual.debug_bounds("CONFIRM_LINKED_MISSION m2").is_some());
    assert!(f.visual.debug_bounds("CONFIRM_LINKED_MISSION m3").is_none());

    // Cancel via Escape keeps chat, m1, and m2 unarchived and grouped.
    f.visual.simulate_keystrokes("escape");
    f.visual.run_until_parked();
    f.window
        .read_with(&f.visual, |root, _| {
            assert!(root.terminal_close_confirm.is_none());
        })
        .unwrap();
    assert!(
        runner_daemon::ops::session::session_get(&unlinked_core, "chat")
            .unwrap()
            .unwrap()
            .archived_at
            .is_none()
    );
    assert_eq!(f.groups().missions(&tab).len(), 2);

    // Stopped chat card path (`archive_chat_sessions`) also gates behind Option A.
    unlinked_core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE sessions SET status = 'stopped' WHERE id = 'chat'",
            [],
        )
        .unwrap();
    f.store.update(&mut f.visual, |store, cx| {
        store.refresh(crate::app_store::StoreRefreshKind::All, cx);
    });
    f.visual.run_until_parked();

    f.window
        .update(&mut f.visual, |root, window, cx| {
            root.open_mission("m1".into(), window, cx);
            assert_eq!(root.route, crate::AppRoute::Mission("m1".into()));
            root.archive_chat_sessions(vec!["chat".into()], window, cx);
            assert!(
                root.terminal_close_confirm.is_some(),
                "StoppedChatCard archive_chat_sessions prompts when linked missions exist"
            );
            root.confirm_terminal_close(window, cx);
        })
        .unwrap();
    f.visual.run_until_parked();

    assert!(
        runner_daemon::ops::session::session_get(&unlinked_core, "chat")
            .unwrap()
            .unwrap()
            .archived_at
            .is_some()
    );
    for id in ["m1", "m2"] {
        let mission = runner_daemon::ops::mission::mission_get(&unlinked_core, id).unwrap();
        assert!(mission.archived_at.is_some(), "{id} should be archived");
        assert_eq!(
            mission.linked_chat.as_ref().map(|c| c.session_id.as_str()),
            Some("chat"),
            "{id} retains linked_session_id"
        );
    }
    assert!(
        runner_daemon::ops::mission::mission_get(&unlinked_core, "m3")
            .unwrap()
            .archived_at
            .is_none(),
        "unlinked mission m3 stays untouched"
    );
    f.window
        .read_with(&f.visual, |root, _| {
            assert_eq!(
                root.route,
                crate::AppRoute::Chat,
                "archiving an open linked mission leaves AppRoute::Mission cleanly"
            );
        })
        .unwrap();
}

#[test]
fn split_tab_single_pane_close_scopes_linked_missions_to_that_pane() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let mut f = RootFixture::new(&mut cx, temp.path(), &["chat-a", "chat-b"]);
    f.link("m1", "chat-a");
    f.link("m2", "chat-b");
    let core = f
        .store
        .read_with(&f.visual, |store, _| store.test_core.clone());
    let tab = f.tab.clone();

    let pane_a = f
        .window
        .update(&mut f.visual, |root, window, cx| {
            root.reload_tabs(cx).unwrap();
            root.tabs.activate(&tab);
            root.route = crate::AppRoute::Chat;
            let pane_a = root
                .tabs
                .active()
                .unwrap()
                .root
                .leaves()
                .into_iter()
                .find(|leaf| leaf.session_id.as_deref() == Some("chat-a"))
                .unwrap()
                .id
                .clone();
            root.request_close_chat_pane(&pane_a, "chat-a", window, cx);
            assert!(root.terminal_close_confirm.is_some());
            pane_a
        })
        .unwrap();
    let _ = pane_a;
    f.visual.update(|window, _| window.refresh());
    f.visual.run_until_parked();

    assert!(f.visual.debug_bounds("CONFIRM_LINKED_MISSION m1").is_some());
    assert!(
        f.visual.debug_bounds("CONFIRM_LINKED_MISSION m2").is_none(),
        "sibling pane's linked mission is excluded from single-pane close prompt"
    );

    f.window
        .update(&mut f.visual, |root, window, cx| {
            root.confirm_terminal_close(window, cx);
        })
        .unwrap();
    f.visual.run_until_parked();

    assert!(runner_daemon::ops::session::session_get(&core, "chat-a")
        .unwrap()
        .unwrap()
        .archived_at
        .is_some());
    assert!(runner_daemon::ops::mission::mission_get(&core, "m1")
        .unwrap()
        .archived_at
        .is_some());
    assert!(
        runner_daemon::ops::session::session_get(&core, "chat-b")
            .unwrap()
            .unwrap()
            .archived_at
            .is_none(),
        "chat-b stays open in the surviving pane"
    );
    assert!(
        runner_daemon::ops::mission::mission_get(&core, "m2")
            .unwrap()
            .archived_at
            .is_none(),
        "chat-b's linked mission m2 stays unarchived"
    );
}
