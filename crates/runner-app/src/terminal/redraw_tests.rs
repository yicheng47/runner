use std::sync::Arc;
use std::time::Duration;

use gpui::{px, size, Entity, TestAppContext, VisualTestContext};

use crate::render_counts::RenderCounts;
use crate::{AppRoute, NativeRoot, PaneLayout, SplitOrientation};

fn settle(
    cx: &mut TestAppContext,
    store: &Entity<crate::app_store::AppStore>,
    counts: &RenderCounts,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut previous = (
        store.read_with(cx, |store, _| store.revisions),
        counts.0.borrow().clone(),
    );
    let mut stable = 0;
    while std::time::Instant::now() < deadline {
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(10));
        cx.run_until_parked();
        let current = (
            store.read_with(cx, |store, _| store.revisions),
            counts.0.borrow().clone(),
        );
        if current == previous {
            stable += 1;
            if stable == 5 {
                return;
            }
        } else {
            stable = 0;
            previous = current;
        }
        std::thread::yield_now();
    }
    panic!("redraw fixture did not settle: {previous:?}");
}

#[test]
fn terminal_output_reuses_split_siblings_and_title_changes_update_sidebar() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let (node_id, idle_pane, core, terminal) = store.update(&mut cx, |store, cx| {
        crate::app_store::seed_mixed_working_sessions(store);
        for status in store.session_statuses.values_mut() {
            status.observation.activity = runner_core::protocol::status::Activity::Idle;
        }
        let core = store.test_core.clone();
        let events: Arc<dyn runner_daemon::session::manager::SessionEvents> =
            Arc::new(core.session_events());
        for id in ["busy-shell", "direct-agent"] {
            core.sessions
                .prepare_unlisted_terminal(id, (80, 24), &core.db, &events)
                .unwrap();
            // Output is injected with test_feed; resize acknowledgments must not race it.
            store.bridge.attach(id).unwrap().test_stop_frames();
        }
        let mut layout = PaneLayout::single(Some("busy-shell"), &[]);
        let idle_pane = layout.split("p1", SplitOrientation::Row).unwrap();
        layout.assign_session(&idle_pane, "direct-agent").unwrap();
        let conn = core.db.get().unwrap();
        let node =
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
                .unwrap();
        store.replace_nodes(runner_daemon::repo::node::list(&conn).unwrap(), cx);
        let terminal = store.bridge.session("busy-shell").unwrap();
        (node.id, idle_pane, core, terminal)
    });
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "redraw-test".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.apply_tab_rows(cx);
        assert!(root.tabs.activate_session("direct-agent"));
        root.route = AppRoute::Chat;
        root.sync_active_chat_detail(cx);
        root.ensure_active_tab_attached(window, cx).unwrap();
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1440.), px(900.)));
    terminal.test_feed(1, b"ready");
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    assert!(visual.debug_bounds("CHAT_SIDE_PANEL").is_some());
    assert!(visual.debug_bounds("APP_SIDEBAR").is_some());
    assert_eq!(
        host.read_with(&cx, |root, _| root.route.clone()).unwrap(),
        AppRoute::Chat
    );

    let counts = RenderCounts::default();
    cx.update(|cx| cx.set_global(counts.clone()));
    // Seed the counters with one real frame so a missing surface cannot pass as zero renders.
    host.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
    settle(&mut cx, &store, &counts);
    let initial = counts.0.borrow().clone();
    for key in [
        "sidebar",
        "side-panel",
        "pane:p1",
        &format!("pane:{idle_pane}"),
        "terminal:busy-shell",
        "terminal:direct-agent",
    ] {
        assert!(
            initial.get(key).copied().unwrap_or_default() > 0,
            "missing render counter: {key}: {initial:?}"
        );
    }
    counts.0.borrow_mut().clear();
    for seq in 2..=25 {
        terminal.test_feed(seq, b"\r|");
    }
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(4));
    cx.run_until_parked();
    let rendered = counts.0.borrow().clone();
    assert_eq!(
        rendered.get("terminal:busy-shell"),
        Some(&1),
        "one output burst should render once: {rendered:?}"
    );
    for key in [
        "sidebar",
        "side-panel",
        &format!("pane:{idle_pane}"),
        "terminal:direct-agent",
    ] {
        assert_eq!(
            rendered.get(key).copied().unwrap_or_default(),
            0,
            "unchanged view re-rendered: {key}: {rendered:?}"
        );
    }

    assert!(terminal.viewer_count() > 0, "busy terminal has no viewer");
    settle(&mut cx, &store, &counts);
    let wake_before = store.read_with(&cx, |store, _| store.revisions.terminal_wake);
    counts.0.borrow_mut().clear();
    let model = core.sessions.terminal_model("busy-shell").unwrap();
    model.feed_output(26, b"\x1b]0;Updated shell title\x07");
    model.flush_events();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while model.title() != "Updated shell title" && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(model.title(), "Updated shell title");
    terminal.refresh_metadata();
    assert_eq!(terminal.title(), "Updated shell title");
    terminal.test_feed(26, b"\x1b]0;Updated shell title\x07");
    cx.run_until_parked();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        cx.executor().advance_clock(Duration::from_millis(10));
        cx.run_until_parked();
        if counts
            .0
            .borrow()
            .get("sidebar")
            .copied()
            .unwrap_or_default()
            > 0
            && store.read_with(&cx, |store, _| store.revisions.terminal_wake) != wake_before
        {
            break;
        }
        std::thread::yield_now();
    }
    let wake_after = store.read_with(&cx, |store, _| store.revisions.terminal_wake);
    assert_ne!(
        wake_after, wake_before,
        "live title must update the store revision"
    );
    let rendered = counts.0.borrow();
    assert!(
        rendered.get("sidebar").copied().unwrap_or_default() > 0,
        "title wake must invalidate the sidebar: {rendered:?}, wake {wake_before}->{wake_after}, viewers {}, title {:?}",
        terminal.viewer_count(), terminal.title()
    );
    assert!(
        rendered
            .get(&format!("sidebar-row:{node_id}"))
            .copied()
            .unwrap_or_default()
            > 0,
        "live title must update its row: {rendered:?}"
    );
    drop(rendered);

    let agent_model = core.sessions.terminal_model("direct-agent").unwrap();
    let agent = store.read_with(&cx, |store, _| {
        store.bridge.session("direct-agent").unwrap()
    });
    agent_model.feed_output(1, b"\x1b]0;Stable topic\x07");
    agent_model.flush_events();
    assert_eq!(agent_model.title(), "Stable topic");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(10));
        if store.read_with(&cx, |store, _| {
            store
                .sessions
                .iter()
                .find(|session| session.session_id == "direct-agent")
                .and_then(|session| session.live_title.as_deref())
                == Some("Stable topic")
        }) {
            break;
        }
        std::thread::yield_now();
    }
    assert_eq!(
        store.read_with(&cx, |store, _| store
            .sessions
            .iter()
            .find(|session| session.session_id == "direct-agent")
            .unwrap()
            .live_title
            .clone()),
        Some("Stable topic".into())
    );
    agent.refresh_metadata();
    agent.test_feed(1, b"\x1b]0;Stable topic\x07");
    settle(&mut cx, &store, &counts);
    let revisions_before = store.read_with(&cx, |store, _| store.revisions);
    counts.0.borrow_mut().clear();
    for (seq, glyph) in (2..).zip(['/', '-', '\\', '|', '/', '-', '\\']) {
        let output = format!("\x1b]0;{glyph} Stable topic\x07");
        agent_model.feed_output(seq, output.as_bytes());
        agent.test_feed(seq, output.as_bytes());
    }
    agent_model.flush_events();
    agent.refresh_metadata();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(4));
    cx.run_until_parked();
    let rendered = counts.0.borrow().clone();
    assert_eq!(
        rendered.get("terminal:direct-agent"),
        Some(&1),
        "title spinner burst must paint once: {rendered:?}"
    );
    for key in ["sidebar", "side-panel", "pane:p1", "terminal:busy-shell"] {
        assert_eq!(
            rendered.get(key).copied().unwrap_or_default(),
            0,
            "title spinner invalidated an unchanged view: {key}: {rendered:?}"
        );
    }

    assert_eq!(agent.title(), "Stable topic");
    assert_eq!(
        store.read_with(&cx, |store, _| store.revisions),
        revisions_before,
        "spinner-only titles must not refresh shared metadata"
    );
}
