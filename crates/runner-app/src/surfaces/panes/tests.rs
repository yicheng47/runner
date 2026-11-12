use super::{
    adjacent_pane_index, archive_chat_confirm_body, drop_allowed, drop_preview, drop_side,
    header_fork_state, maximized_leaf, pane_body_opacity, pane_close_behavior, pane_identity_icon,
    pane_menu_actions, pane_menu_items, pane_rename_key, prune_maximized, side_panel_available,
    side_panel_surface, split_allowed, split_decision, split_menu_items, split_tab_archiving,
    starting_overlay_label, terminal_drawer_tooltip, toggle_maximized, HeaderForkState,
    PaneCloseBehavior, PaneRenameKey, PanelSurface, SplitDecision, MIN_SPLIT_PANE_HEIGHT,
    MIN_SPLIT_PANE_WIDTH, TOO_SMALL_TO_SPLIT, UNFOCUSED_PANE_OPACITY,
};
use crate::app_settings::AppSettings;
use crate::keymap;
use gpui::{point, px, size, AppContext, Bounds};
use runner_app::pane_layout::{DropSide, PaneLayout, SplitOrientation};
use runner_core::protocol::model::SessionStatus;
use runner_core::protocol::session::DirectSessionEntry;

fn direct_session(runtime: &str, native_fork: bool, forkable: bool) -> DirectSessionEntry {
    DirectSessionEntry {
        claude_account: None,
        session_id: format!("{runtime}-session"),
        project_id: None,
        role_id: None,
        handle: None,
        agent_runtime: runtime.into(),
        agent_command: runtime.into(),
        agent_model: None,
        agent_effort: None,
        display_name: runtime.into(),
        status: SessionStatus::Running,
        title: None,
        live_title: None,
        cwd: None,
        started_at: None,
        stopped_at: None,
        resumable: forkable,
        native_fork,
        forkable,
        agent_session_key: forkable.then(|| "key".into()),
        pinned: false,
        archived_at: None,
    }
}

#[test]
fn island_lifecycle_controls_follow_lifecycle_and_never_control_shells() {
    use runner_app::ui::{agent_status::StatusKind::*, SessionControlKind};
    for status in [
        Working,
        Idle,
        Interrupted,
        Approval,
        Answer,
        NeedsYou,
        Error,
        ResponseFailed,
        Unavailable,
    ] {
        assert_eq!(
            super::pane_lifecycle_action("codex", true, status, false, false),
            Some(SessionControlKind::Stop)
        );
        assert_eq!(
            super::pane_lifecycle_action("shell", true, status, false, false),
            None
        );
        assert_eq!(
            super::pane_lifecycle_action("codex", true, status, true, false),
            None
        );
        assert_eq!(
            super::pane_lifecycle_action("codex", true, status, false, true),
            None
        );
    }
    for status in [Starting, Resuming] {
        assert_eq!(
            super::pane_lifecycle_action("codex", true, status, false, false),
            None
        );
    }
    assert_eq!(
        super::pane_lifecycle_action("codex", false, Stopped, false, false),
        Some(SessionControlKind::Resume)
    );
    assert_eq!(
        super::pane_lifecycle_action("shell", false, Stopped, false, false),
        None
    );
}

#[test]
fn panel_toggle_follows_the_top_right_corner_across_nested_splits() {
    let mut layout = PaneLayout::single(Some("chat"), &[]);
    let left = layout.focused_pane_id.clone();
    layout.split(&left, SplitOrientation::Row).unwrap();
    let right = layout.focused_pane_id.clone();
    assert_eq!(super::top_right_pane(&layout.root).id, right);
    layout.split(&right, SplitOrientation::Column).unwrap();
    assert_eq!(super::top_right_pane(&layout.root).id, right);
    layout.split(&right, SplitOrientation::Row).unwrap();
    assert_eq!(
        super::top_right_pane(&layout.root).id,
        layout.focused_pane_id
    );
    layout.focus_pane(&left);
    assert_ne!(super::top_right_pane(&layout.root).id, left);
}

#[test]
fn dragging_an_empty_pane_grip_preserves_its_id_and_sibling_chat() {
    use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, cx| {
        crate::app_store::seed_mixed_working_sessions(store);
        let mut layout = PaneLayout::single(Some("direct-agent"), &[]);
        layout.split("p1", SplitOrientation::Row).unwrap();
        {
            let conn = store.test_core.db.get().unwrap();
            conn.execute("UPDATE sessions SET status = 'stopped'", [])
                .unwrap();
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
                .unwrap();
        }
        store.refresh(crate::app_store::StoreRefreshKind::All, cx);
    });
    let window = cx.add_window(|window, cx| {
        let mut root = crate::NativeRoot::new(
            "empty-pane-drag".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.apply_tab_rows(cx);
        assert!(root.tabs.activate_session("direct-agent"));
        root.route = crate::AppRoute::Chat;
        root.sync_active_chat_detail(cx);
        root
    });
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(size(px(1600.), px(900.)));
    visual.run_until_parked();
    #[cfg(target_os = "macos")]
    {
        let margin = visual.debug_bounds("WINDOW_TOP_MARGIN_DRAG").unwrap();
        let islands = visual.debug_bounds("APP_CONTENT_COLUMN").unwrap();
        let at = point(islands.center().x, px(4.));
        assert!(margin.contains(&at));
        crate::platform_ui::take_titlebar_actions();
        visual.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
        assert_eq!(
            crate::platform_ui::take_titlebar_actions(),
            [crate::platform_ui::TitlebarAction::Move]
        );
        visual.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
    }
    #[cfg(target_os = "macos")]
    {
        let label = window
            .read_with(&visual, |root, cx| {
                root.front_page_session("direct-agent", cx)
                    .unwrap()
                    .label(None)
            })
            .unwrap();
        let title = visual
            .debug_bounds(Box::leak(format!("PANE_TITLE:{label}").into_boxed_str()))
            .unwrap();
        crate::platform_ui::take_titlebar_actions();
        visual.simulate_event(gpui::MouseDownEvent {
            position: title.center(),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 2,
            first_mouse: false,
        });
        assert!(
            crate::platform_ui::take_titlebar_actions().is_empty(),
            "title rename must not request a window move or zoom"
        );
        visual.simulate_event(gpui::MouseUpEvent {
            position: title.center(),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 2,
        });
        visual.run_until_parked();
        window
            .update(&mut visual, |root, _, cx| {
                assert!(root.pane_rename.is_some());
                root.pane_rename = None;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
    }
    let mark = visual.debug_bounds("PANE_DRAG_GRIP:p3").unwrap();
    let target = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
    let destination = point(target.center().x, target.top() + px(10.));
    #[cfg(target_os = "macos")]
    crate::platform_ui::take_titlebar_actions();
    visual.simulate_mouse_down(mark.center(), MouseButton::Left, Modifiers::default());
    #[cfg(target_os = "macos")]
    assert!(
        crate::platform_ui::take_titlebar_actions().is_empty(),
        "pane grip does not request native window movement"
    );
    window
        .read_with(&visual, |root, _| assert!(!root.titlebar_drag_armed))
        .unwrap();
    visual.simulate_mouse_move(
        mark.center() + point(px(12.), px(12.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    visual.simulate_mouse_move(destination, Some(MouseButton::Left), Modifiers::default());
    visual.simulate_mouse_up(destination, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| {
            let layout = root.tabs.active().unwrap();
            assert_eq!(
                layout
                    .root
                    .leaves()
                    .iter()
                    .map(|leaf| leaf.id.as_str())
                    .collect::<Vec<_>>(),
                ["p3", "p1"]
            );
            assert!(layout.root.leaves()[0].session_id.is_none());
            assert_eq!(layout.session_ids(), ["direct-agent"]);
            assert_eq!(layout.focused_pane_id, "p3");
        })
        .unwrap();
}

#[test]
fn split_header_keeps_its_grip_and_actions_within_narrow_zoomed_panes() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            let conn = store.test_core.db.get().unwrap();
            conn.execute("UPDATE sessions SET status='stopped', agent_runtime='claude-code', agent_command='claude', title='Claude Code' WHERE id='direct-agent'", []).unwrap();
            let mut layout = PaneLayout::single(Some("direct-agent"), &[]);
            layout.split("p1", SplitOrientation::Row).unwrap();
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap()).unwrap();
            drop(conn);
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
    let window = cx.add_window(|window, cx| {
        let mut root = crate::NativeRoot::new(
            "split-grip-budget".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.tabs.activate_session("direct-agent");
        root.route = crate::AppRoute::Chat;
        root
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
    let chrome_width = store.read_with(&cx, |store, _| {
        store.settings.sidebar_width + store.settings.chat_panel_width + 26.
    });
    for zoom in [1., 1.3] {
        window
            .update(&mut visual, |root, window, cx| {
                root.set_zoom(zoom, window, cx)
            })
            .unwrap();
        for width in [240., 360.] {
            visual.simulate_resize(size(
                px((2. * width + chrome_width) * zoom),
                px(800. * zoom),
            ));
            visual.run_until_parked();
            let header = visual.debug_bounds("PANE_HEADER:direct-agent").unwrap();
            let grip = visual.debug_bounds("PANE_DRAG_GRIP:p1").unwrap();
            let mark = visual.debug_bounds("PANE_RUNTIME_MARK:p1").unwrap();
            let resume = visual.debug_bounds("PANE_LIFECYCLE:Resume chat").unwrap();
            assert!((grip.size.width - px(14. * zoom)).abs() < px(1.));
            assert!((grip.left() - header.left() - px(8. * zoom)).abs() < px(1.));
            assert!((mark.left() - grip.right() - px(4. * zoom)).abs() < px(1.));
            assert!(grip.left() >= header.left() && resume.right() <= header.right());
            assert!(mark.right() < resume.left());
        }
    }
}

#[test]
fn a_maximized_pane_fills_the_tab_and_its_toggle_restores_the_split() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let other = store.update(&mut cx, |store, cx| {
        crate::app_store::seed_mixed_working_sessions(store);
        store.settings.chat_panel_open = false;
        let conn = store.test_core.db.get().unwrap();
        let mut layout = PaneLayout::single(Some("direct-agent"), &[]);
        let other = layout.split("p1", SplitOrientation::Row).unwrap();
        runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
            .unwrap();
        drop(conn);
        store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        other
    });
    let other_island: &str = Box::leak(format!("PANE_ISLAND:{other}").into_boxed_str());
    let other_toggle: &str = Box::leak(format!("PANE_MAXIMIZE:{other}").into_boxed_str());
    let window = cx.add_window(|window, cx| {
        let mut root = crate::NativeRoot::new(
            "maximize-pane".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.tabs.activate_session("direct-agent");
        root.route = crate::AppRoute::Chat;
        root
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(size(px(1600.), px(800.)));
    visual.run_until_parked();
    let split_left = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
    let split_right = visual.debug_bounds(other_island).unwrap();
    assert!(visual.debug_bounds("PANE_MAXIMIZE:p1").is_some());
    assert!(visual.debug_bounds(other_toggle).is_some());
    let persisted = window
        .read_with(&visual, |root, _| {
            root.tabs.active().unwrap().serialize().unwrap()
        })
        .unwrap();

    let toggle = visual.debug_bounds("PANE_MAXIMIZE:p1").unwrap();
    visual.simulate_click(toggle.center(), Default::default());
    visual.run_until_parked();
    let maximized = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
    assert!(visual.debug_bounds(other_island).is_none());
    assert_eq!(maximized.left(), split_left.left());
    assert_eq!(maximized.right(), split_right.right());
    window
        .read_with(&visual, |root, _| {
            assert_eq!(root.tabs.active().unwrap().serialize().unwrap(), persisted);
        })
        .unwrap();

    let toggle = visual.debug_bounds("PANE_MAXIMIZE:p1").unwrap();
    visual.simulate_click(toggle.center(), Default::default());
    visual.run_until_parked();
    assert_eq!(visual.debug_bounds("PANE_ISLAND:p1"), Some(split_left));
    assert_eq!(visual.debug_bounds(other_island), Some(split_right));

    window
        .update(&mut visual, |root, window, cx| {
            root.toggle_maximize_pane(&other, window, cx);
            assert_eq!(root.tabs.active().unwrap().focused_pane_id, other);
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("PANE_ISLAND:p1").is_none());
    window
        .update(&mut visual, |root, window, cx| {
            root.focus_next_chat_pane(&crate::FocusNextPane, window, cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(visual.debug_bounds("PANE_ISLAND:p1"), Some(split_left));
    assert_eq!(visual.debug_bounds(other_island), Some(split_right));
    window
        .read_with(&visual, |root, _| {
            assert_eq!(root.tabs.active().unwrap().focused_pane_id, "p1");
            assert!(root.maximized_panes.is_empty());
        })
        .unwrap();

    window
        .update(&mut visual, |root, window, cx| {
            root.toggle_maximize_pane("p1", window, cx);
            root.split_pane("p1", SplitOrientation::Column, window, cx);
            assert!(root.maximized_panes.is_empty());
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds(other_island).is_some());
}

#[test]
fn a_maximized_pane_splits_by_its_restored_size() {
    let filled = size(px(958.), px(698.));
    let mut layout = PaneLayout::single(Some("chat"), &[]);
    let left = layout.focused_pane_id.clone();
    let right = layout.split(&left, SplitOrientation::Row).unwrap();
    let restored = super::restored_pane_size(filled, &layout, &left, 1.);
    assert_eq!(
        restored,
        size(px(475.), px(698.)),
        "neither the gutter nor the borders are shared"
    );
    assert!(matches!(
        split_decision(Some(filled), SplitOrientation::Row, 1.),
        SplitDecision::Allowed
    ));
    assert!(matches!(
        split_decision(Some(restored), SplitOrientation::Row, 1.),
        SplitDecision::Blocked(_)
    ));
    assert!(matches!(
        split_decision(Some(restored), SplitOrientation::Column, 1.),
        SplitDecision::Allowed
    ));

    let lower = layout.split(&right, SplitOrientation::Column).unwrap();
    assert_eq!(
        super::restored_pane_size(filled, &layout, &lower, 1.5),
        size(px((960. - 9.) / 2. - 2.), px((700. - 9.) / 2. - 2.)),
        "nested splits each take their zoomed gutter"
    );
}

#[test]
fn title_clicks_select_their_pane_and_maximize_respects_split_floors_and_disconnects() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let right = store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            store.settings.chat_panel_open = false;
            let conn = store.test_core.db.get().unwrap();
            conn.execute_batch("UPDATE sessions SET title='Left chat' WHERE id='direct-agent'; UPDATE sessions SET title='Right shell' WHERE id='busy-shell';").unwrap();
            let mut layout = PaneLayout::single(Some("direct-agent"), &[]);
            let right = layout.split("p1", SplitOrientation::Row).unwrap();
            layout.assign_session(&right, "busy-shell").unwrap();
            assert!(layout.focus_pane("p1"));
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap()).unwrap();
            drop(conn);
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
            right
        });
    let window = cx.add_window(|window, cx| {
        let mut root = crate::NativeRoot::new(
            "title-focus".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.tabs.activate_session("direct-agent");
        root.route = crate::AppRoute::Chat;
        root
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(size(px(1600.), px(800.)));
    visual.run_until_parked();
    let focused = |visual: &mut gpui::VisualTestContext| {
        window
            .read_with(visual, |root, _| {
                root.tabs.active().unwrap().focused_pane_id.clone()
            })
            .unwrap()
    };
    assert_eq!(focused(&mut visual), "p1");

    let title = visual.debug_bounds("PANE_TITLE:Right shell").unwrap();
    visual.simulate_click(title.center(), Default::default());
    visual.run_until_parked();
    assert_eq!(
        focused(&mut visual),
        right,
        "a title click selects its pane"
    );
    assert!(visual.debug_bounds("POPOVER_MENU_PANEL").is_some());
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(visual.debug_bounds("POPOVER_MENU_PANEL").is_none());
    assert_eq!(focused(&mut visual), right);
    window
        .update(&mut visual, |root, window, cx| {
            root.toggle_maximize_pane_action(&crate::ToggleMaximizePane, window, cx)
        })
        .unwrap();
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| {
            assert_eq!(root.maximized_panes.values().collect::<Vec<_>>(), [&right]);
        })
        .unwrap();
    assert!(visual.debug_bounds("PANE_ISLAND:p1").is_none());

    store.update(&mut visual, |store, cx| {
        store.daemon_disconnected = true;
        store.revisions.error = store.revisions.error.wrapping_add(1);
        cx.notify();
    });
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| assert!(root.maximized_panes.is_empty()))
        .unwrap();
    store.update(&mut visual, |store, cx| {
        store.daemon_disconnected = false;
        store.revisions.error = store.revisions.error.wrapping_add(1);
        cx.notify();
    });

    // At the floor's edge: side by side each pane is a few pixels short
    // of a split, though it is wide enough while it fills the tab, and
    // maximize must reach the same decision as the split view.
    let split_width = |visual: &mut gpui::VisualTestContext| {
        let left = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
        let right: &str = Box::leak(format!("PANE_ISLAND:{right}").into_boxed_str());
        let right = visual.debug_bounds(right).unwrap();
        (left, right.right() - left.left())
    };
    let (_, tab_width) = split_width(&mut visual);
    let chrome = f32::from(px(1600.) - tab_width);
    for zoom in [1., 1.3] {
        window
            .update(&mut visual, |root, window, cx| {
                root.set_zoom(zoom, window, cx)
            })
            .unwrap();
        visual.simulate_resize(size(
            px(chrome * zoom + 2. * MIN_SPLIT_PANE_WIDTH * zoom * 2. + 3.),
            px(800.),
        ));
        visual.run_until_parked();
        let split = window
            .read_with(&visual, |root, _| {
                let tab = root.tabs.active().unwrap().id.clone();
                root.front_page_pane_bounds()[&super::PaneKey::new(&tab, "p1")]
            })
            .unwrap();
        assert!(
            split.width < px(2. * MIN_SPLIT_PANE_WIDTH * zoom),
            "{split:?}"
        );
        assert!(matches!(
            split_decision(Some(split), SplitOrientation::Row, zoom),
            SplitDecision::Blocked(_)
        ));
        window
            .update(&mut visual, |root, window, cx| {
                root.toggle_maximize_pane("p1", window, cx)
            })
            .unwrap();
        visual.run_until_parked();
        let filled = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
        assert!(matches!(
            split_decision(Some(filled.size), SplitOrientation::Row, zoom),
            SplitDecision::Allowed
        ));
        window
            .update(&mut visual, |root, window, cx| {
                let tab = root.tabs.active().unwrap().id.clone();
                let restored = root.split_pane_bounds(&tab, "p1", cx).unwrap();
                assert!(
                    (restored.width - split.width).abs() < px(0.5)
                        && (restored.height - split.height).abs() < px(0.5),
                    "zoom {zoom}: estimated {restored:?}, measured {split:?}"
                );
                assert!(matches!(
                    split_decision(Some(restored), SplitOrientation::Row, zoom),
                    SplitDecision::Blocked(_)
                ));
                root.split_pane("p1", SplitOrientation::Row, window, cx);
                assert_eq!(root.tabs.active().unwrap().root.leaves().len(), 2);
                assert!(
                    !root.maximized_panes.is_empty(),
                    "a refused split stays maximized"
                );
                root.toggle_maximize_pane("p1", window, cx);
            })
            .unwrap();
        visual.run_until_parked();
    }
    window
        .update(&mut visual, |root, window, cx| {
            root.set_zoom(1., window, cx);
            root.toggle_maximize_pane("p1", window, cx);
        })
        .unwrap();

    visual.simulate_resize(size(px(1600.), px(800.)));
    visual.run_until_parked();
    window
        .update(&mut visual, |root, window, cx| {
            root.split_pane("p1", SplitOrientation::Row, window, cx);
            assert_eq!(root.tabs.active().unwrap().root.leaves().len(), 3);
            assert!(root.maximized_panes.is_empty());
        })
        .unwrap();
}

#[test]
fn clicking_away_from_an_inline_rename_saves_it() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            store.settings.chat_panel_open = false;
            let conn = store.test_core.db.get().unwrap();
            conn.execute_batch("UPDATE sessions SET title='Before' WHERE id='direct-agent'; UPDATE sessions SET title='Right' WHERE id='busy-shell';").unwrap();
            let mut layout = PaneLayout::single(Some("direct-agent"), &[]);
            let right = layout.split("p1", SplitOrientation::Row).unwrap();
            layout.assign_session(&right, "busy-shell").unwrap();
            assert!(layout.focus_pane("p1"));
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
                .unwrap();
            drop(conn);
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
    let window = cx.add_window(|window, cx| {
        let mut root = crate::NativeRoot::new(
            "rename-blur".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.tabs.activate_session("direct-agent");
        root.route = crate::AppRoute::Chat;
        root
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(size(px(1600.), px(800.)));
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    let input = window
        .update(&mut visual, |root, window, cx| {
            root.begin_pane_rename(
                "direct-agent".into(),
                "Before".into(),
                "codex".into(),
                window,
                cx,
            );
            root.pane_rename.as_ref().unwrap().input.clone()
        })
        .unwrap();
    input.update(&mut visual, |field, cx| field.set_text("After", cx));
    visual.run_until_parked();
    let other = visual.debug_bounds("PANE_TITLE:Right").unwrap();
    visual.simulate_click(other.center(), Default::default());
    visual.run_until_parked();
    window
        .read_with(&visual, |root, cx| {
            assert!(root.pane_rename.is_none(), "losing focus ends the rename");
            assert_eq!(
                root.front_page_session("direct-agent", cx)
                    .and_then(|entry| entry.title),
                Some("After".into())
            );
        })
        .unwrap();
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    window
        .read_with(&visual, |root, _| {
            assert_eq!(
                root.pane_rename
                    .as_ref()
                    .map(|rename| rename.session_id.as_str()),
                Some("busy-shell"),
                "the clicked title's menu keeps the keyboard"
            );
        })
        .unwrap();
}

#[test]
fn pane_title_menu_labels_fit_one_line() {
    use crate::i18n::{testing::with_locale, Language, Locale};
    use gpui::{Focusable, TestAppContext, VisualTestContext};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            for zoom in [1., 1.5] {
                for running in [true, false] {
                    let temp = tempfile::tempdir().unwrap();
                    let mut cx = TestAppContext::single();
                    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                    store.update(&mut cx, |store, cx| {
                            crate::app_store::seed_mixed_working_sessions(store);
                            store.settings.app_zoom = zoom;
                            store.settings.language = if locale == Locale::English { Language::English } else { Language::SimplifiedChinese };
                            if !running {
                                store.test_core.db.get().unwrap().execute("UPDATE sessions SET status = 'stopped' WHERE id = 'direct-agent'", []).unwrap();
                            }
                            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
                            store.sync_locale(cx);
                        });
                    let window = cx.add_window(|window, cx| {
                        let mut root = crate::NativeRoot::new(
                            "pane-menu-fit".into(),
                            temp.path().join("logs"),
                            None,
                            None,
                            store.clone(),
                            window,
                            cx,
                        );
                        root.apply_tab_rows(cx);
                        assert!(root.tabs.activate_session("direct-agent"));
                        root.route = crate::AppRoute::Chat;
                        root.sync_active_chat_detail(cx);
                        root
                    });
                    let mut visual = VisualTestContext::from_window(window.into(), &cx);
                    visual.simulate_resize(size(px(1799.), px(1128.)));
                    visual.run_until_parked();
                    let menu = window
                        .read_with(&visual, |root, _| {
                            root.pane_action_menus["direct-agent"].clone()
                        })
                        .unwrap();
                    visual.update(|window, cx| {
                        menu.read(cx).focus_handle(cx).focus(window, cx);
                    });
                    visual.simulate_keystrokes("down");
                    visual.run_until_parked();
                    let panel = visual.debug_bounds("POPOVER_MENU_PANEL").unwrap();
                    assert!(
                        panel.right() <= px(1799.),
                        "menu must stay inside the window: {panel:?}"
                    );
                    for (label, key) in [
                        ("POPOVER_LABEL_0", "ui.rename_chat_action"),
                        ("POPOVER_LABEL_1", "ui.fork_chat_action"),
                        ("POPOVER_LABEL_2", "ui.pin"),
                    ] {
                        let bounds = visual.debug_bounds(label).unwrap();
                        let full_text_width = visual.update(|window, _| {
                            let text = crate::i18n::text(key);
                            let style = window.text_style();
                            window
                                .text_system()
                                .shape_line(
                                    text.into(),
                                    px(13. * zoom),
                                    &[style.to_run(text.len())],
                                    None,
                                )
                                .width
                        });
                        assert!(
                            bounds.size.height <= px(18. * zoom + 1.),
                            "{locale:?}, zoom={zoom}, running={running}: {label} {bounds:?}"
                        );
                        assert!(
                            bounds.size.width >= full_text_width,
                            "full menu label must fit: {bounds:?}"
                        );
                    }
                }
            }
        });
    }
}

#[test]
fn island_status_details_truncate_inside_the_header_without_displacing_controls() {
    use gpui::{TestAppContext, VisualTestContext};
    use runner_core::protocol::status::{
        Activity, AgentObservation, AgentStatus, HumanInteraction, Lifecycle,
        ProgramStatusEvidence, WaitReason, WorkDetail,
    };
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for approval in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute(
                    "UPDATE sessions SET title='Header detail fixture' WHERE id='direct-agent'",
                    [],
                )
                .unwrap();
            store.settings.chat_panel_open = false;
            store.settings.sidebar_collapsed = false;
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
            store.session_statuses.insert(
                "direct-agent".into(),
                AgentStatus {
                    lifecycle: Lifecycle::Running,
                    observation: AgentObservation {
                        activity: Activity::Working,
                        detail: (!approval).then_some(WorkDetail::CompactingContext),
                        program_status: Some(ProgramStatusEvidence {
                            message: approval.then(|| {
                                "Allow this command with a very long program status message? "
                                    .repeat(40)
                            }),
                        }),
                        interactions: if approval {
                            vec![HumanInteraction {
                                id: "approval".into(),
                                reason: WaitReason::Approval,
                                owners: vec![],
                                since: chrono::Utc::now().timestamp_millis(),
                            }]
                        } else {
                            vec![]
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
        });
        let reported = store.read_with(&cx, |store, _| {
            store.session_statuses["direct-agent"].clone()
        });
        let sidebar_width = store.read_with(&cx, |store, _| store.settings.sidebar_width);
        let host = cx.add_window(|window, cx| {
            let mut root = crate::NativeRoot::new(
                "status-detail-fit".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.apply_tab_rows(cx);
            assert!(root.tabs.activate_session("direct-agent"));
            root.route = crate::AppRoute::Chat;
            root.sync_active_chat_detail(cx);
            root
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        for zoom in [1., 1.3] {
            host.update(&mut visual, |root, window, cx| {
                root.set_zoom(zoom, window, cx)
            })
            .unwrap();
            for pane_width in if approval { [410., 600.] } else { [570., 800.] } {
                visual.simulate_resize(size(
                    px((pane_width + sidebar_width + 14.) * zoom),
                    px(900.),
                ));
                visual.run_until_parked();
                host.update(&mut visual, |_, _, cx| {
                    store.update(cx, |store, _| {
                        store
                            .session_statuses
                            .insert("direct-agent".into(), reported.clone());
                    });
                    cx.notify();
                })
                .unwrap();
                visual.run_until_parked();
                let pane = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
                let header = visual.debug_bounds("PANE_HEADER:direct-agent").unwrap();
                let status = visual.debug_bounds("ISLAND_STATUS_INDICATOR").unwrap();
                let state = host
                    .read_with(&visual, |root, cx| {
                        root.front_page_status("direct-agent", cx).unwrap()
                    })
                    .unwrap();
                assert_eq!(
                    state.kind,
                    if approval {
                        runner_app::ui::agent_status::StatusKind::Approval
                    } else {
                        runner_app::ui::agent_status::StatusKind::Working
                    }
                );
                let detail = visual.debug_bounds("ISLAND_STATUS_DETAIL");
                let actions = visual.debug_bounds("PANE_HEADER_ACTIONS").unwrap();
                if let Some(detail) = detail {
                    assert!((status.size.width - px(180. * zoom)).abs() < px(0.5));
                    assert!(detail.size.width > px(0.) && detail.right() <= status.right());
                }
                assert_eq!(detail.is_some(), pane_width >= 570.);
                assert!(status.left() >= header.left() && status.right() <= actions.left());
                let lifecycle = visual.debug_bounds("PANE_LIFECYCLE:Stop chat").unwrap();
                assert!(
                        lifecycle.left() >= status.right() && lifecycle.right() <= actions.left(), "pane={pane_width}, status={status:?}, action={lifecycle:?}, controls={actions:?}, identity={:?}", visual.debug_bounds("ISLAND_HEADER_IDENTITY")
                    );
                assert!((actions.left() - lifecycle.right() - px(8. * zoom + 1.)).abs() < px(0.5));
                assert!(actions.right() <= header.right() && header.right() < pane.right());
                host.update(&mut visual, |_, _, cx| {
                    store.update(cx, |store, _| {
                        store.session_statuses.insert(
                            "direct-agent".into(),
                            AgentStatus {
                                lifecycle: Lifecycle::Running,
                                observation: AgentObservation {
                                    activity: Activity::Ready,
                                    ..Default::default()
                                },
                                ..Default::default()
                            },
                        );
                    });
                    cx.notify();
                })
                .unwrap();
                visual.run_until_parked();
                assert!(visual.debug_bounds("ISLAND_STATUS_DETAIL").is_none());
                assert_eq!(
                    visual.debug_bounds("PANE_LIFECYCLE:Stop chat"),
                    Some(lifecycle),
                    "pane={pane_width}, zoom={zoom}"
                );
            }
        }
    }
}

#[test]
fn status_grows_leftward_and_the_lifecycle_slot_never_moves() {
    use gpui::{TestAppContext, VisualTestContext};
    use runner_core::protocol::status::{
        Activity, AgentObservation, AgentStatus, HumanInteraction, Lifecycle, WaitReason,
    };
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let status = |activity, approval: bool| AgentStatus {
        lifecycle: Lifecycle::Running,
        observation: AgentObservation {
            activity,
            interactions: if approval {
                vec![HumanInteraction {
                    id: "approval".into(),
                    reason: WaitReason::Approval,
                    owners: vec![],
                    since: chrono::Utc::now().timestamp_millis(),
                }]
            } else {
                vec![]
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, cx| {
        crate::app_store::seed_mixed_working_sessions(store);
        store
            .test_core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE sessions SET title='Slot fixture' WHERE id='direct-agent'",
                [],
            )
            .unwrap();
        store.settings.chat_panel_open = false;
        store.settings.sidebar_collapsed = false;
        store.refresh(crate::app_store::StoreRefreshKind::All, cx);
    });
    let sidebar_width = store.read_with(&cx, |store, _| store.settings.sidebar_width);
    let host = cx.add_window(|window, cx| {
        let mut root = crate::NativeRoot::new(
            "lifecycle-slot".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.apply_tab_rows(cx);
        assert!(root.tabs.activate_session("direct-agent"));
        root.route = crate::AppRoute::Chat;
        root.sync_active_chat_detail(cx);
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    for zoom in [1., 1.3] {
        host.update(&mut visual, |root, window, cx| {
            root.chat_transitions.remove("direct-agent");
            root.set_zoom(zoom, window, cx)
        })
        .unwrap();
        visual.simulate_resize(size(px((700. + sidebar_width + 14.) * zoom), px(900.)));
        let mut slot_at = None;
        for (activity, approval) in [
            (Activity::Working, false),
            (Activity::Ready, false),
            (Activity::Working, true),
        ] {
            host.update(&mut visual, |_, _, cx| {
                store.update(cx, |store, _| {
                    store
                        .session_statuses
                        .insert("direct-agent".into(), status(activity, approval));
                });
                cx.notify();
            })
            .unwrap();
            visual.run_until_parked();
            assert!(
                visual.debug_bounds("PANE_MAXIMIZE:p1").is_none(),
                "a single pane has no maximize toggle"
            );
            let title = visual.debug_bounds("PANE_TITLE:Slot fixture").unwrap();
            let indicator = visual.debug_bounds("ISLAND_STATUS_INDICATOR").unwrap();
            let slot = visual.debug_bounds("PANE_LIFECYCLE_SLOT").unwrap();
            let stop = visual.debug_bounds("PANE_LIFECYCLE:Stop chat").unwrap();
            assert_eq!(stop, slot);
            assert!(title.right() < indicator.left());
            assert!(
                (slot.left() - indicator.right() - px(8. * zoom)).abs() < px(1.),
                "{activity:?}/{approval}: status {indicator:?}, slot {slot:?}"
            );
            assert_eq!(
                *slot_at.get_or_insert(slot),
                slot,
                "{activity:?}/{approval}"
            );
        }
        host.update(&mut visual, |root, _, cx| {
            root.chat_transitions.insert(
                "direct-agent".into(),
                crate::ChatTransition {
                    kind: crate::surfaces::chat_lifecycle::TransitionKind::Starting,
                    started_at: std::time::Instant::now(),
                    baseline_seq: 0,
                    generation: 0,
                },
            );
            cx.notify();
        })
        .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("PANE_LIFECYCLE:Stop chat").is_none());
        let slot = visual.debug_bounds("PANE_LIFECYCLE_SLOT").unwrap();
        assert_eq!(Some(slot), slot_at, "Starting keeps the button's place");
        let indicator = visual.debug_bounds("ISLAND_STATUS_INDICATOR").unwrap();
        assert!((slot.left() - indicator.right() - px(8. * zoom)).abs() < px(1.));
    }
}

#[test]
fn terminal_drawer_header_has_no_fill_of_its_own() {
    use crate::theme;
    use crate::theme_snapshot::{assert_fill, ThemeGuard};
    use gpui::prelude::*;
    use gpui::{div, App, Context, Render, TestAppContext, VisualTestContext, Window};
    use std::rc::Rc;

    struct DrawerHeader;
    impl Render for DrawerHeader {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .bg(theme::bg())
                .child(super::render_terminal_drawer_strip(
                    "probe",
                    &[],
                    None,
                    &[],
                    super::TerminalDrawerCallbacks {
                        can_add: true,
                        can_close: true,
                        activate: Rc::new(|_, _: &mut Window, _: &mut App| {}),
                        close: Rc::new(|_, _, _| {}),
                        add: Rc::new(|_, _| {}),
                        hide: Rc::new(|_, _| {}),
                    },
                ))
        }
    }
    let _theme = ThemeGuard::new();
    let mut cx = TestAppContext::single();
    let host = cx.add_window(|_, _| DrawerHeader);
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    assert_fill(&mut visual, "DRAWER_HEADER", None);
}

#[test]
fn side_panel_keeps_the_last_focused_chat_while_a_split_terminal_has_focus() {
    use crate::{AppRoute, NativeRoot};
    use gpui::{TestAppContext, VisualTestContext};

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            let core = store.test_core.clone();
            let conn = core.db.get().unwrap();
            conn.execute_batch(
                "INSERT INTO sessions (id, role_id, mission_id, slot_id, status, agent_runtime, agent_command)
                 VALUES ('second-agent', 'test-shell-role', NULL, NULL, 'stopped', 'codex', 'codex');
                 UPDATE sessions SET status = 'stopped' WHERE mission_id IS NULL;",
            )
            .unwrap();
            let mut layout = PaneLayout::single(Some("direct-agent"), &[]);
            let second = layout.split("p1", SplitOrientation::Row).unwrap();
            layout.assign_session(&second, "second-agent").unwrap();
            let shell = layout.split(&second, SplitOrientation::Row).unwrap();
            layout.assign_session(&shell, "busy-shell").unwrap();
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
                .unwrap();
            store.replace_nodes(runner_daemon::repo::node::list(&conn).unwrap(), cx);
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
            store.settings.chat_panel_open = true;
        });
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "side-panel-terminal-split".into(),
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
        root.chat_surfaces
            .open
            .open(crate::surfaces::side_panel::PanelViewId::Info);
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1600.), px(900.)));
    let focus = |session_id: &str, visual: &mut VisualTestContext| {
        host.update(visual, |root, _, cx| {
            assert!(root.tabs.active_mut().unwrap().focus_session(session_id));
            cx.notify();
        })
        .unwrap();
        visual.run_until_parked();
        host.read_with(visual, |root, _| {
            (
                root.chat_panel_visibility.target,
                root.active_chat_detail
                    .as_ref()
                    .map(|detail| detail.session_id.clone()),
            )
        })
        .unwrap()
    };

    for (focused, described) in [
        ("direct-agent", "direct-agent"),
        ("busy-shell", "direct-agent"),
        ("second-agent", "second-agent"),
        ("busy-shell", "second-agent"),
    ] {
        assert_eq!(
            focus(focused, &mut visual),
            (1., Some(described.to_owned())),
            "focused {focused}"
        );
        assert!(visual.debug_bounds("CHAT_SIDE_PANEL").is_some());
        assert!(visual.debug_bounds("PANEL_TAB Info").is_some());
        assert!(visual.debug_bounds("PANEL_ADD_SURFACE").is_some());
    }

    host.update(&mut visual, |root, _, cx| {
        root.set_chat_panel_open(false, cx)
    })
    .unwrap();
    assert_eq!(focus("busy-shell", &mut visual).0, 0.);
    assert!(visual.debug_bounds("CHAT_PANEL_TOGGLE").is_some());
}

#[test]
fn a_hovered_link_names_the_panel_its_own_pane_will_show() {
    use crate::file_links::{FileLinkTarget, LAUNCHED};
    use crate::surfaces::side_panel::PanelSubject;
    use crate::{AppRoute, NativeRoot};
    use gpui::{TestAppContext, VisualTestContext};

    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    for root in [&first, &second] {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(root.join("lib.rs"), "one\ntwo\n").unwrap();
    }
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            let core = store.test_core.clone();
            let conn = core.db.get().unwrap();
            conn.execute_batch(
                "INSERT INTO sessions (id, role_id, mission_id, slot_id, status, agent_runtime, agent_command)
                 VALUES ('second-agent', 'test-shell-role', NULL, NULL, 'stopped', 'codex', 'codex');
                 UPDATE sessions SET status = 'stopped' WHERE mission_id IS NULL;",
            )
            .unwrap();
            for (id, root) in [("direct-agent", &first), ("second-agent", &second)] {
                conn.execute(
                    "UPDATE sessions SET files_root = ?1 WHERE id = ?2",
                    [root.to_string_lossy().as_ref(), id],
                )
                .unwrap();
            }
            let mut layout = PaneLayout::single(Some("direct-agent"), &[]);
            let pane = layout.split("p1", SplitOrientation::Row).unwrap();
            layout.assign_session(&pane, "second-agent").unwrap();
            let shell = layout.split(&pane, SplitOrientation::Row).unwrap();
            layout.assign_session(&shell, "busy-shell").unwrap();
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
                .unwrap();
            store.replace_nodes(runner_daemon::repo::node::list(&conn).unwrap(), cx);
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
            store.settings.chat_panel_open = true;
        });
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "link-two-chats".into(),
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
        root.chat_surfaces
            .open
            .open(crate::surfaces::side_panel::PanelViewId::Info);
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1600.), px(900.)));
    visual.run_until_parked();
    let since = std::time::Instant::now();
    let previews = |visual: &mut VisualTestContext, pane: &str, path: std::path::PathBuf| {
        let route = host
            .update(visual, |root, _, cx| root.file_link_route(pane, cx))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            let answer =
                visual.update(|_, cx| route.previews(&path, since, std::rc::Rc::new(|_| {}), cx));
            if let Some(answer) = answer {
                return answer;
            }
            assert!(std::time::Instant::now() < deadline, "timed out: {pane}");
            std::thread::sleep(std::time::Duration::from_millis(20));
            visual.run_until_parked();
        }
    };

    // The first chat has focus; each pane's links name its own panel.
    assert!(previews(&mut visual, "second-agent", second.join("lib.rs")));
    assert!(!previews(&mut visual, "second-agent", first.join("lib.rs")));
    assert!(previews(&mut visual, "direct-agent", first.join("lib.rs")));
    assert!(!previews(
        &mut visual,
        "direct-agent",
        second.join("lib.rs")
    ));
    // A terminal follows the tab's last focused chat.
    assert!(previews(&mut visual, "busy-shell", first.join("lib.rs")));
    assert!(!previews(&mut visual, "busy-shell", second.join("lib.rs")));

    // Clicking in the second chat focuses it, as mouse-down does, and
    // the link opens in its panel.
    LAUNCHED.with(|launched| launched.borrow_mut().clear());
    let route = host
        .update(&mut visual, |root, _, cx| {
            assert!(root
                .tabs
                .active_mut()
                .unwrap()
                .focus_session("second-agent"));
            root.sync_active_chat_detail(cx);
            root.file_link_route("second-agent", cx)
        })
        .unwrap();
    let target = FileLinkTarget {
        path: second.join("lib.rs"),
        line: Some(2),
        column: None,
    };
    visual.update(|_, cx| route.open(target, cx));
    let subject = PanelSubject::Session("second-agent".into());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while host
        .read_with(&visual, |root, cx| {
            root.chat_files
                .view(&subject)
                .and_then(|view| view.read(cx).active_tab())
        })
        .unwrap()
        != Some("lib.rs".into())
    {
        assert!(std::time::Instant::now() < deadline, "timed out");
        std::thread::sleep(std::time::Duration::from_millis(20));
        visual.run_until_parked();
    }
    assert!(LAUNCHED.with(|launched| launched.borrow().is_empty()));
    // Now the shell follows the second chat.
    assert!(previews(&mut visual, "busy-shell", second.join("lib.rs")));
}

fn panel_root(
    session: &'static str,
) -> (
    tempfile::TempDir,
    gpui::TestAppContext,
    gpui::Entity<crate::app_store::AppStore>,
    gpui::WindowHandle<crate::NativeRoot>,
    gpui::VisualTestContext,
) {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, cx| {
        crate::app_store::seed_mixed_working_sessions(store);
        store
            .test_core
            .db
            .get()
            .unwrap()
            .execute_batch(
                "UPDATE sessions SET status = 'stopped' WHERE mission_id IS NULL;
                     UPDATE sessions SET cwd = '/tmp/panel-' || id WHERE mission_id IS NULL;",
            )
            .unwrap();
        store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        store.settings.chat_panel_open = true;
    });
    let host = cx.add_window(|window, cx| {
        let mut root = crate::NativeRoot::new(
            "side-panel-views".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.apply_tab_rows(cx);
        assert!(root.tabs.activate_session(session));
        root.route = crate::AppRoute::Chat;
        root.sync_active_chat_detail(cx);
        if session == "direct-agent" {
            root.chat_surfaces
                .open
                .open(crate::surfaces::side_panel::PanelViewId::Info);
        }
        root
    });
    let visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1600.), px(900.)));
    visual.run_until_parked();
    (temp, cx, store, host, visual)
}

fn set_panel_theme(
    store: &gpui::Entity<crate::app_store::AppStore>,
    visual: &mut gpui::VisualTestContext,
    intent: crate::theme::ThemeIntent,
) {
    store.update(visual, |store, cx| {
        store.update_settings(
            |settings| {
                settings.app_theme = intent;
                true
            },
            false,
            cx,
        );
    });
    visual.run_until_parked();
    visual.refresh().unwrap();
    visual.run_until_parked();
}

#[cfg(target_os = "macos")]
#[test]
fn double_clicking_the_panel_collapse_control_does_not_zoom_the_window() {
    use crate::platform_ui::{take_titlebar_actions, TitlebarAction};
    use gpui::{point, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent};

    let (_temp, _cx, store, host, mut visual) = panel_root("direct-agent");
    let panel = visual.debug_bounds("CHAT_SIDE_PANEL").unwrap();
    let info = visual.debug_bounds("PANEL_TAB Info").unwrap();
    let collapse = point(panel.right() - px(22.), info.center().y);
    let click = |visual: &mut gpui::VisualTestContext, click_count| {
        visual.simulate_event(MouseDownEvent {
            position: collapse,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count,
            first_mouse: false,
        });
        visual.simulate_event(MouseUpEvent {
            position: collapse,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count,
        });
        visual.run_until_parked();
    };
    take_titlebar_actions();
    click(&mut visual, 1);
    assert!(!store.read_with(&visual, |store, _| store.settings.chat_panel_open));
    assert!(take_titlebar_actions().is_empty());
    host.update(&mut visual, |root, _, cx| {
        // Hold the collapse halfway, where a real double-click's second
        // press lands while the panel is still shrinking.
        root.chat_panel_visibility.start = 0.5;
        root.chat_panel_visibility.target = 0.;
        root.chat_panel_visibility.started_at =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(3600));
        cx.notify();
    })
    .unwrap();
    visual.run_until_parked();
    click(&mut visual, 2);
    assert_eq!(
        take_titlebar_actions(),
        [TitlebarAction::Move],
        "the second click lands on the uncovered drag area as a fresh press"
    );
    click(&mut visual, 1);
    click(&mut visual, 2);
    assert_eq!(
        take_titlebar_actions(),
        [TitlebarAction::Move, TitlebarAction::Zoom],
        "a double-click on the drag area itself still zooms"
    );
}

fn chat_surfaces(
    host: &gpui::WindowHandle<crate::NativeRoot>,
    visual: &mut gpui::VisualTestContext,
) -> crate::surfaces::side_panel::OpenSurfaces {
    host.read_with(visual, |root, _| root.chat_surfaces.open.clone())
        .unwrap()
}

fn surfaces_of(
    ids: &[crate::surfaces::side_panel::PanelViewId],
    active: crate::surfaces::side_panel::PanelViewId,
) -> crate::surfaces::side_panel::OpenSurfaces {
    let mut surfaces = crate::surfaces::side_panel::OpenSurfaces::default();
    for id in ids {
        surfaces.open(*id);
    }
    surfaces.select(active);
    surfaces
}

fn click_selector(visual: &mut gpui::VisualTestContext, selector: &'static str) {
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    visual.simulate_click(bounds.center(), gpui::Modifiers::default());
    visual.run_until_parked();
}

#[test]
fn chat_panel_header_holds_the_open_surfaces_as_tabs_then_plus_in_both_themes() {
    use crate::i18n::{testing::with_locale, Locale};
    use crate::surfaces::side_panel::{PanelContext, PanelSubject, PanelViewId};
    use crate::theme::{self, ThemeIntent};
    use crate::theme_snapshot::{assert_fill_color, assert_text};

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let (_temp, _cx, store, host, mut visual) = panel_root("direct-agent");
            // Info open: its tab, `+`, then collapse.
            let panel = visual.debug_bounds("CHAT_SIDE_PANEL").unwrap();
            let info = visual.debug_bounds("PANEL_TAB Info").unwrap();
            let add = visual.debug_bounds("PANEL_ADD_SURFACE").unwrap();
            assert!(visual.debug_bounds("PANEL_TAB Files").is_none());
            assert!(visual.debug_bounds("PANEL_TAB Sessions").is_none());
            assert!(visual.debug_bounds("PANEL_TAB_CLOSE Info").is_some());
            assert!(info.left() - panel.left() <= px(12.), "{info:?} {panel:?}");
            assert_eq!(info.size.height, px(28.));
            assert_eq!(add.size, gpui::size(px(24.), px(24.)));
            assert!(info.right() < add.left() && add.left() - info.right() <= px(6.));
            assert_eq!(info.center().y, add.center().y);
            assert!(
                panel.right() - add.right() >= px(60.),
                "a gap, then collapse on the right: {add:?} {panel:?}"
            );
            assert!(visual.debug_bounds("CHAT_PANEL_META codex").is_some());
            for intent in [ThemeIntent::Dark, ThemeIntent::Light] {
                set_panel_theme(&store, &mut visual, intent);
                assert_fill_color(&mut visual, "PANEL_TAB Info", Some(theme::raised()));
                assert_text(&mut visual, "PANEL_TAB_INK Info", theme::text());
            }

            // `+` lists Info and Files, Info checked; Esc closes it.
            click_selector(&mut visual, "PANEL_ADD_SURFACE");
            let info_row = visual.debug_bounds("PANEL_MENU_ROW Info").unwrap();
            let files_row = visual.debug_bounds("PANEL_MENU_ROW Files").unwrap();
            assert!(info_row.bottom() <= files_row.top());
            assert!(info_row.top() > add.bottom(), "below `+`");
            assert!(visual.debug_bounds("PANEL_MENU_CHECK Info").is_some());
            assert!(visual.debug_bounds("PANEL_MENU_CHECK Files").is_none());
            for intent in [ThemeIntent::Dark, ThemeIntent::Light] {
                set_panel_theme(&store, &mut visual, intent);
                assert_fill_color(&mut visual, "PANEL_MENU", Some(theme::raised()));
                assert_fill_color(
                    &mut visual,
                    "PANEL_MENU_ROW Info",
                    Some(theme::sidebar_selected()),
                );
                assert_fill_color(&mut visual, "PANEL_MENU_ROW Files", None);
            }
            visual.simulate_keystrokes("escape");
            assert!(visual.debug_bounds("PANEL_MENU").is_none());

            // `F` while the menu has focus opens Files and closes it.
            click_selector(&mut visual, "PANEL_ADD_SURFACE");
            visual.simulate_keystrokes("f");
            assert!(visual.debug_bounds("PANEL_MENU").is_none());
            assert_eq!(
                chat_surfaces(&host, &mut visual),
                surfaces_of(&[PanelViewId::Info, PanelViewId::Files], PanelViewId::Files)
            );
            let context = PanelContext {
                subject: PanelSubject::Session("direct-agent".into()),
                surface: crate::surfaces::side_panel::PanelSurface::Chat,
                cwd: Some("/tmp/panel-direct-agent".into()),
                project: None,
                remote: false,
            };
            assert!(visual
                .debug_bounds(format!("FILES_VIEW {context:?}").leak())
                .is_some());
            assert!(visual.debug_bounds("CHAT_PANEL_META codex").is_none());
            let info = visual.debug_bounds("PANEL_TAB Info").unwrap();
            let files = visual.debug_bounds("PANEL_TAB Files").unwrap();
            assert!(info.right() < files.left());
            assert!(visual.debug_bounds("PANEL_TAB_CLOSE Info").is_none());
            assert!(visual.debug_bounds("PANEL_TAB_CLOSE Files").is_some());
            for intent in [ThemeIntent::Dark, ThemeIntent::Light] {
                set_panel_theme(&store, &mut visual, intent);
                assert_fill_color(&mut visual, "PANEL_TAB Info", None);
                assert_fill_color(&mut visual, "PANEL_TAB Files", Some(theme::raised()));
                assert_text(&mut visual, "PANEL_TAB_INK Info", theme::muted());
                assert_text(&mut visual, "PANEL_TAB_INK Files", theme::text());
            }

            // Picking an open surface switches to it; a tab click too.
            click_selector(&mut visual, "PANEL_ADD_SURFACE");
            assert!(visual.debug_bounds("PANEL_MENU_CHECK Files").is_some());
            click_selector(&mut visual, "PANEL_MENU_ROW Info");
            assert!(visual.debug_bounds("CHAT_PANEL_META codex").is_some());
            assert_eq!(
                chat_surfaces(&host, &mut visual),
                surfaces_of(&[PanelViewId::Info, PanelViewId::Files], PanelViewId::Info),
                "each surface opens once"
            );
            click_selector(&mut visual, "PANEL_TAB Files");
            assert!(visual
                .debug_bounds(format!("FILES_VIEW {context:?}").leak())
                .is_some());
        });
    }

    #[cfg(target_os = "macos")]
    {
        use crate::platform_ui::{take_titlebar_actions, TitlebarAction};
        let (_temp, _cx, _store, _host, mut visual) = panel_root("direct-agent");
        let info = visual.debug_bounds("PANEL_TAB Info").unwrap();
        let add = visual.debug_bounds("PANEL_ADD_SURFACE").unwrap();
        let gap = gpui::point(add.right() + px(40.), info.center().y);
        take_titlebar_actions();
        visual.simulate_mouse_down(gap, gpui::MouseButton::Left, gpui::Modifiers::default());
        visual.simulate_mouse_up(gap, gpui::MouseButton::Left, gpui::Modifiers::default());
        assert_eq!(take_titlebar_actions(), [TitlebarAction::Move]);
        for at in [info.center(), add.center()] {
            for click_count in [1, 2] {
                visual.simulate_event(gpui::MouseDownEvent {
                    button: gpui::MouseButton::Left,
                    position: at,
                    modifiers: gpui::Modifiers::default(),
                    click_count,
                    first_mouse: false,
                });
                visual.simulate_event(gpui::MouseUpEvent {
                    button: gpui::MouseButton::Left,
                    position: at,
                    modifiers: gpui::Modifiers::default(),
                    click_count,
                });
                visual.run_until_parked();
                assert!(take_titlebar_actions().is_empty(), "{at:?} x{click_count}");
            }
        }
    }
}

#[test]
fn closing_the_last_surface_shows_the_focused_surface_list_whose_letters_open_them() {
    use crate::i18n::{testing::with_locale, Locale};
    use crate::surfaces::side_panel::PanelViewId;
    use crate::theme::{self, ThemeIntent};
    use crate::theme_snapshot::assert_fill_color;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let (_temp, _cx, store, host, mut visual) = panel_root("direct-agent");
            click_selector(&mut visual, "PANEL_ADD_SURFACE");
            click_selector(&mut visual, "PANEL_MENU_ROW Files");
            // Closing Files shows the tab before it, Info.
            click_selector(&mut visual, "PANEL_TAB_CLOSE Files");
            assert_eq!(
                chat_surfaces(&host, &mut visual),
                surfaces_of(&[PanelViewId::Info], PanelViewId::Info)
            );
            assert!(visual.debug_bounds("CHAT_PANEL_META codex").is_some());
            // The last: the list, focused, with only `+` and collapse above.
            click_selector(&mut visual, "PANEL_TAB_CLOSE Info");
            assert!(visual.debug_bounds("PANEL_EMPTY").is_some());
            assert!(visual.debug_bounds("PANEL_TAB Info").is_none());
            assert!(visual.debug_bounds("PANEL_ADD_SURFACE").is_some());
            assert!(visual.debug_bounds("CHAT_PANEL_META codex").is_none());
            let focused = host
                .update(&mut visual, |root, window, _| {
                    root.chat_surfaces.list_focus().is_focused(window)
                })
                .unwrap();
            assert!(focused, "the list takes focus");
            let info = visual.debug_bounds("PANEL_EMPTY_ROW Info").unwrap();
            let files = visual.debug_bounds("PANEL_EMPTY_ROW Files").unwrap();
            assert!(info.bottom() <= files.top());
            assert!(visual.debug_bounds("PANEL_MENU_CHECK Info").is_none());
            for intent in [ThemeIntent::Dark, ThemeIntent::Light] {
                set_panel_theme(&store, &mut visual, intent);
                assert_fill_color(
                    &mut visual,
                    "PANEL_EMPTY_ROW Info",
                    Some(theme::sidebar_selected()),
                );
                assert_fill_color(&mut visual, "PANEL_EMPTY_ROW Files", None);
            }

            // `I` opens Info; from the list again, `F` opens Files.
            visual.simulate_keystrokes("i");
            assert_eq!(
                chat_surfaces(&host, &mut visual),
                surfaces_of(&[PanelViewId::Info], PanelViewId::Info)
            );
            assert!(visual.debug_bounds("PANEL_EMPTY").is_none());
            click_selector(&mut visual, "PANEL_TAB_CLOSE Info");
            visual.simulate_keystrokes("f");
            assert_eq!(
                chat_surfaces(&host, &mut visual),
                surfaces_of(&[PanelViewId::Files], PanelViewId::Files)
            );
            // The arrows and Enter pick from the list too.
            click_selector(&mut visual, "PANEL_TAB_CLOSE Files");
            visual.simulate_keystrokes("down enter");
            assert_eq!(
                chat_surfaces(&host, &mut visual),
                surfaces_of(&[PanelViewId::Files], PanelViewId::Files)
            );
            let focus = |visual: &mut gpui::VisualTestContext| {
                host.update(visual, |root, window, _| {
                    (
                        root.chat_surfaces.list_focus().is_focused(window),
                        root.chat_focus.is_focused(window),
                    )
                })
                .unwrap()
            };
            assert_eq!(focus(&mut visual), (false, true), "the chat takes the keys");

            // `+` opened from the focused list: Esc returns to the list,
            // and a pick hands the keys to the chat.
            click_selector(&mut visual, "PANEL_TAB_CLOSE Files");
            assert_eq!(focus(&mut visual), (true, false));
            click_selector(&mut visual, "PANEL_ADD_SURFACE");
            visual.simulate_keystrokes("escape");
            assert!(visual.debug_bounds("PANEL_MENU").is_none());
            assert_eq!(focus(&mut visual), (true, false), "Esc returns to the list");
            click_selector(&mut visual, "PANEL_ADD_SURFACE");
            visual.simulate_keystrokes("f");
            assert_eq!(
                chat_surfaces(&host, &mut visual),
                surfaces_of(&[PanelViewId::Files], PanelViewId::Files)
            );
            assert_eq!(focus(&mut visual), (false, true), "a pick from `+` too");

            // A collapsed panel reopened on the list gives it focus.
            click_selector(&mut visual, "PANEL_TAB_CLOSE Files");
            host.update(&mut visual, |root, window, cx| {
                root.set_chat_panel_open(false, cx);
                root.chat_focus.focus(window, cx);
            })
            .unwrap();
            visual.run_until_parked();
            click_selector(&mut visual, "CHAT_PANEL_TOGGLE");
            let focused = host
                .update(&mut visual, |root, window, _| {
                    root.chat_surfaces.list_focus().is_focused(window)
                })
                .unwrap();
            assert!(focused);
            // The letters act only while the list has focus.
            host.update(&mut visual, |root, window, cx| {
                root.chat_focus.focus(window, cx)
            })
            .unwrap();
            visual.simulate_keystrokes("i");
            assert_eq!(
                chat_surfaces(&host, &mut visual),
                Default::default(),
                "never globally"
            );
        });
    }
}

#[test]
fn a_new_chat_opens_on_the_surface_list_and_leaves_the_keys_with_the_chat() {
    use crate::surfaces::front_page::client::ChatLaunched;
    use crate::surfaces::side_panel::{OpenSurfaces, PanelViewId};

    let (_temp, _cx, _store, host, mut visual) = panel_root("direct-agent");
    host.update(&mut visual, |root, window, cx| {
        root.chat_surfaces.open =
            surfaces_of(&[PanelViewId::Files, PanelViewId::Info], PanelViewId::Info);
        root.finish_local_chat_launch(
            Some("direct-agent".into()),
            Ok(ChatLaunched {
                session: "direct-agent".into(),
                layout: None,
            }),
            window,
            cx,
        );
    })
    .unwrap();
    visual.run_until_parked();
    assert_eq!(chat_surfaces(&host, &mut visual), OpenSurfaces::default());
    assert!(visual.debug_bounds("PANEL_EMPTY").is_some());
    // Past the start transition, which focuses the terminal.
    visual
        .executor()
        .advance_clock(std::time::Duration::from_secs(30));
    visual.run_until_parked();
    let focused = host
        .update(&mut visual, |root, window, _| {
            root.chat_surfaces.list_focus().is_focused(window)
        })
        .unwrap();
    assert!(!focused, "typing goes to the new chat, not the list");

    // A collapsed panel, or a terminal-only tab, never takes focus.
    let (_temp, _cx, _store, host, mut visual) = panel_root("direct-agent");
    host.update(&mut visual, |root, window, cx| {
        root.set_chat_panel_open(false, cx);
        root.chat_surfaces.open = OpenSurfaces::default();
        root.focus_empty_chat_panel(window, cx);
    })
    .unwrap();
    visual.run_until_parked();
    let focused = host
        .update(&mut visual, |root, window, _| {
            root.chat_surfaces.list_focus().is_focused(window)
        })
        .unwrap();
    assert!(!focused, "a collapsed panel leaves focus alone");
    let (_temp, _cx, _store, host, mut visual) = panel_root("busy-shell");
    host.update(&mut visual, |root, window, cx| {
        root.chat_surfaces.open = OpenSurfaces::default();
        root.focus_empty_chat_panel(window, cx);
    })
    .unwrap();
    let focused = host
        .update(&mut visual, |root, window, _| {
            root.chat_surfaces.list_focus().is_focused(window)
        })
        .unwrap();
    assert!(!focused, "a terminal-only tab shows Files, not the list");
}

#[test]
fn chat_panel_header_keeps_plus_and_collapse_at_the_narrowest_panel_in_both_locales() {
    use crate::i18n::{testing::with_locale, Locale};
    use crate::surfaces::side_panel::PanelViewId;

    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let (_temp, _cx, _store, host, mut visual) = panel_root("direct-agent");
            host.update(&mut visual, |root, _, cx| {
                root.chat_surfaces.open =
                    surfaces_of(&[PanelViewId::Info, PanelViewId::Files], PanelViewId::Files);
                cx.notify();
            })
            .unwrap();
            for width in [
                crate::app_settings::CHAT_PANEL_DEFAULT,
                crate::app_settings::CHAT_PANEL_MIN,
            ] {
                host.update(&mut visual, |root, _, cx| {
                    root.update_app_settings(cx, false, |settings| {
                        settings.chat_panel_width = width;
                        true
                    });
                })
                .unwrap();
                visual.run_until_parked();
                let panel = visual.debug_bounds("CHAT_SIDE_PANEL").unwrap();
                let close = visual.debug_bounds("PANEL_TAB_CLOSE Files").unwrap();
                let add = visual.debug_bounds("PANEL_ADD_SURFACE").unwrap();
                let collapse = visual.debug_bounds("PANEL_COLLAPSE").unwrap();
                assert!(close.right() <= add.left(), "{locale:?} {width}");
                assert!(add.right() <= collapse.left(), "{locale:?} {width}");
                assert!(
                    collapse.right() <= panel.right(),
                    "{locale:?} {width}: {collapse:?} {panel:?}"
                );
                click_selector(&mut visual, "PANEL_TAB_CLOSE Files");
                assert_eq!(
                    chat_surfaces(&host, &mut visual),
                    surfaces_of(&[PanelViewId::Info], PanelViewId::Info)
                );
                click_selector(&mut visual, "PANEL_ADD_SURFACE");
                click_selector(&mut visual, "PANEL_MENU_ROW Files");
                let info = visual.debug_bounds("PANEL_TAB Info").unwrap();
                assert!(info.left() >= panel.left(), "{locale:?} {width}");
            }
        });
    }
}

#[test]
fn a_file_link_opens_in_the_panel_preview_of_a_chat_or_a_terminal_tab() {
    use crate::file_links::{FileLinkTarget, LAUNCHED};
    use crate::surfaces::side_panel::{PanelSubject, PanelViewId};

    for session in ["direct-agent", "busy-shell"] {
        let (temp, _cx, store, host, mut visual) = panel_root(session);
        let root = temp.path().join("checkout");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("lib.rs"), "one\ntwo\nthree\n").unwrap();
        let outside = temp.path().join("outside.txt");
        std::fs::write(&outside, "x\n").unwrap();
        store.update(&mut visual, |store, _| {
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute(
                    "UPDATE sessions SET files_root = ?1 WHERE id = ?2",
                    [root.to_string_lossy().as_ref(), session],
                )
                .unwrap();
        });
        visual.run_until_parked();
        // A terminal tab showed its Files view before the root was saved.
        host.update(&mut visual, |root, _, cx| {
            root.set_chat_panel_open(false, cx);
            root.chat_files.release(cx);
            assert!(!root.chat_panel_is_open(cx));
        })
        .unwrap();
        let wait = |visual: &mut gpui::VisualTestContext,
                    ready: &dyn Fn(&mut gpui::VisualTestContext) -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            while !ready(visual) {
                assert!(std::time::Instant::now() < deadline, "timed out: {session}");
                std::thread::sleep(std::time::Duration::from_millis(20));
                visual.run_until_parked();
            }
        };
        let click = |visual: &mut gpui::VisualTestContext, path: std::path::PathBuf| {
            LAUNCHED.with(|launched| launched.borrow_mut().clear());
            let route = host
                .update(visual, |root, _, cx| root.file_link_route(session, cx))
                .unwrap();
            let target = FileLinkTarget {
                path,
                line: Some(2),
                column: None,
            };
            visual.update(|_, cx| route.open(target, cx));
            visual.run_until_parked();
        };
        let launched = || LAUNCHED.with(|launched| launched.borrow().len());
        let panel_open = |visual: &mut gpui::VisualTestContext| {
            host.read_with(visual, |root, cx| root.chat_panel_is_open(cx))
                .unwrap()
        };

        click(&mut visual, outside.clone());
        wait(&mut visual, &|_| launched() == 1);
        assert!(!panel_open(&mut visual), "{session}");

        click(&mut visual, root.join("lib.rs"));
        let subject = PanelSubject::Session(session.into());
        wait(&mut visual, &|visual| {
            host.read_with(visual, |root, cx| {
                root.chat_files
                    .view(&subject)
                    .and_then(|view| view.read(cx).active_tab())
                    == Some("lib.rs".into())
            })
            .unwrap()
        });
        assert_eq!(launched(), 0, "{session}");
        assert!(
            panel_open(&mut visual),
            "{session}: the collapsed panel opens"
        );
        assert_eq!(
            chat_surfaces(&host, &mut visual),
            if session == "direct-agent" {
                surfaces_of(&[PanelViewId::Info, PanelViewId::Files], PanelViewId::Files)
            } else {
                surfaces_of(&[PanelViewId::Files], PanelViewId::Files)
            },
            "{session}: the closed Files tab opens only in its own tab"
        );

        for off in [
            (|settings: &mut AppSettings| settings.files_view = false) as fn(&mut AppSettings),
            |settings| settings.show_side_panel = false,
        ] {
            store.update(&mut visual, |store, cx| {
                store.update_settings(
                    |settings| {
                        settings.files_view = true;
                        settings.show_side_panel = true;
                        off(settings);
                        true
                    },
                    false,
                    cx,
                )
            });
            click(&mut visual, root.join("lib.rs"));
            assert_eq!(launched(), 1, "{session}");
        }
    }
}

#[test]
fn search_files_opens_the_collapsed_panel_on_files_and_its_finder() {
    let (temp, _cx, store, host, mut visual) = panel_root("direct-agent");
    let root = temp.path().join("checkout");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("lib.rs"), "one\n").unwrap();
    store.update(&mut visual, |store, _| {
        store
            .test_core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE sessions SET files_root = ?1 WHERE id = 'direct-agent'",
                [root.to_string_lossy().as_ref()],
            )
            .unwrap();
    });
    visual.run_until_parked();
    host.update(&mut visual, |root, _, cx| {
        root.set_chat_panel_open(false, cx);
        root.chat_files.release(cx);
        assert!(!root.chat_panel_is_open(cx));
    })
    .unwrap();

    visual.dispatch_action(crate::SearchFiles);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while visual.debug_bounds("FILES_FINDER_ITEM 0").is_none() {
        assert!(std::time::Instant::now() < deadline, "timed out");
        std::thread::sleep(std::time::Duration::from_millis(20));
        visual.run_until_parked();
    }
    assert!(host
        .read_with(&visual, |root, cx| root.chat_panel_is_open(cx))
        .unwrap());
    visual.simulate_keystrokes("enter");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while visual.debug_bounds("FILE_CODE").is_none() {
        assert!(std::time::Instant::now() < deadline, "timed out");
        std::thread::sleep(std::time::Duration::from_millis(20));
        visual.run_until_parked();
    }
}

/// The chat under a parent that records the keys that reach it, as the
/// terminal's key listener would.
struct KeyProbe {
    root: gpui::Entity<crate::NativeRoot>,
    keys: Vec<String>,
}

impl gpui::Render for KeyProbe {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::prelude::*;
        gpui::div()
            .size_full()
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, _| {
                this.keys.push(event.keystroke.key.clone());
            }))
            .child(self.root.clone())
    }
}

#[test]
fn the_finder_key_goes_back_to_the_terminal_when_there_is_no_view_to_search() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, cx| {
        crate::app_store::seed_mixed_working_sessions(store);
        store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        store.settings.files_view = false;
    });
    cx.update(|cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "ctrl-shift-o",
            crate::SearchFiles,
            None,
        )])
    });
    let host = cx.add_window(|window, cx| {
        let root = cx.new(|cx| {
            let mut root = crate::NativeRoot::new(
                "finder-key".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.apply_tab_rows(cx);
            assert!(root.tabs.activate_session("direct-agent"));
            root.route = crate::AppRoute::Chat;
            root.sync_active_chat_detail(cx);
            root
        });
        KeyProbe {
            root,
            keys: Vec::new(),
        }
    });
    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1600.), px(900.)));
    visual.run_until_parked();
    let root = host
        .read_with(&visual, |probe, _| probe.root.clone())
        .unwrap();
    visual.update(|window, cx| {
        let focus = root.read(cx).root_focus.clone();
        focus.focus(window, cx);
    });
    visual.run_until_parked();

    visual.simulate_keystrokes("ctrl-shift-o");
    visual.run_until_parked();
    let keys = host
        .read_with(&visual, |probe, _| probe.keys.clone())
        .unwrap();
    assert_eq!(keys, ["o"], "Files off: the key carries on past the action");

    store.update(&mut visual, |store, _| store.settings.files_view = true);
    visual.run_until_parked();
    host.update(&mut visual, |probe, _, _| probe.keys.clear())
        .unwrap();
    visual.simulate_keystrokes("ctrl-shift-o");
    visual.run_until_parked();
    let keys = host
        .read_with(&visual, |probe, _| probe.keys.clone())
        .unwrap();
    assert!(keys.is_empty(), "Files on: the finder took it, {keys:?}");
}

#[test]
fn terminal_tab_panel_holds_files_alone_and_keeps_its_own_panel_state() {
    use crate::surfaces::side_panel::{PanelContext, PanelSubject};
    use crate::theme::{self, ThemeIntent};
    use crate::theme_snapshot::assert_text;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let (_temp, _cx, store, host, mut visual) = panel_root("busy-shell");
    let panel = visual.debug_bounds("CHAT_SIDE_PANEL").unwrap();
    let width = store.read_with(&visual, |store, _| store.settings.chat_panel_width);
    assert!((panel.size.width - px(width)).abs() <= px(1.), "{panel:?}");
    assert!(visual.debug_bounds("CHAT_PANEL_GUTTER").is_some());
    // One surface: its permanent tab and collapse, with no × or `+`.
    for view in ["Info", "Sessions"] {
        assert!(
            visual
                .debug_bounds(format!("PANEL_TAB {view}").leak())
                .is_none(),
            "{view}"
        );
    }
    assert!(visual.debug_bounds("PANEL_TAB_CLOSE Files").is_none());
    assert!(visual.debug_bounds("PANEL_ADD_SURFACE").is_none());
    let tab = visual.debug_bounds("PANEL_TAB Files").unwrap();
    assert!(tab.left() - panel.left() <= px(12.), "{tab:?} {panel:?}");
    let context = PanelContext {
        subject: PanelSubject::Session("busy-shell".into()),
        surface: crate::surfaces::side_panel::PanelSurface::Terminal,
        cwd: Some("/tmp/panel-busy-shell".into()),
        project: None,
        remote: false,
    };
    assert!(visual
        .debug_bounds(format!("FILES_VIEW {context:?}").leak())
        .is_some());
    assert_eq!(
        chat_surfaces(&host, &mut visual),
        Default::default(),
        "the terminal tab keeps only its own surfaces"
    );
    for intent in [ThemeIntent::Dark, ThemeIntent::Light] {
        set_panel_theme(&store, &mut visual, intent);
        assert_text(&mut visual, "PANEL_TAB_INK Files", theme::text());
    }

    host.update(&mut visual, |root, _, cx| {
        root.update_app_settings(cx, false, |settings| {
            settings.chat_panel_width = 400.;
            true
        });
    })
    .unwrap();
    visual.run_until_parked();
    let panel = visual.debug_bounds("CHAT_SIDE_PANEL").unwrap();
    assert!((panel.size.width - px(400.)).abs() <= px(1.), "{panel:?}");

    host.update(&mut visual, |root, _, cx| {
        root.set_chat_panel_open(false, cx);
    })
    .unwrap();
    visual.run_until_parked();
    assert_eq!(
        host.read_with(&visual, |root, _| root.chat_panel_visibility.target)
            .unwrap(),
        0.
    );
    assert!(visual.debug_bounds("CHAT_PANEL_TOGGLE").is_some());
}

#[test]
fn chat_tabs_restore_their_surfaces_open_state_and_file_after_a_delayed_link() {
    use crate::file_links::FileLinkTarget;
    use crate::surfaces::side_panel::{PanelSubject, PanelViewId};

    let (temp, _cx, store, host, mut visual) = panel_root("direct-agent");
    let folder = temp.path().join("checkout");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("lib.rs"), "one\ntwo\n").unwrap();
    store.update(&mut visual, |store, cx| {
            store.test_core.db.get().unwrap().execute(
                "UPDATE sessions SET files_root = ?1, agent_runtime = 'codex' WHERE mission_id IS NULL",
                [folder.to_string_lossy().as_ref()],
            ).unwrap();
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
    visual.run_until_parked();
    let switch = |session: &str, visual: &mut gpui::VisualTestContext| {
        host.update(visual, |root, _, cx| {
            assert!(root.tabs.activate_session(session));
            root.sync_active_chat_detail(cx);
            cx.notify();
        })
        .unwrap();
        visual.run_until_parked();
    };
    switch("busy-shell", &mut visual);
    host.update(&mut visual, |root, _, cx| {
        root.chat_surfaces.open.open(PanelViewId::Info);
        cx.notify();
    })
    .unwrap();
    switch("direct-agent", &mut visual);
    let route = host
        .update(&mut visual, |root, _, cx| {
            root.file_link_route("direct-agent", cx)
        })
        .unwrap();
    visual.update(|_, cx| {
        route.open(
            FileLinkTarget {
                path: folder.join("lib.rs"),
                line: Some(2),
                column: None,
            },
            cx,
        )
    });
    switch("busy-shell", &mut visual);
    let subject = PanelSubject::Session("direct-agent".into());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !host
        .read_with(&visual, |root, cx| {
            root.chat_files
                .view(&subject)
                .and_then(|view| view.read(cx).active_tab())
                == Some("lib.rs".into())
        })
        .unwrap()
    {
        assert!(std::time::Instant::now() < deadline, "link did not finish");
        std::thread::sleep(std::time::Duration::from_millis(20));
        visual.run_until_parked();
    }
    assert_eq!(
        chat_surfaces(&host, &mut visual),
        surfaces_of(&[PanelViewId::Info], PanelViewId::Info)
    );
    host.update(&mut visual, |root, _, cx| {
        assert!(root.chat_panel_is_open(cx));
        assert!(root
            .chat_panel_layout(root.active_chat_detail.as_ref(), cx)
            .1
            .is_none());
        root.set_chat_panel_open(false, cx);
        cx.notify();
    })
    .unwrap();
    switch("direct-agent", &mut visual);
    assert_eq!(
        chat_surfaces(&host, &mut visual),
        surfaces_of(&[PanelViewId::Info, PanelViewId::Files], PanelViewId::Files)
    );
    host.update(&mut visual, |root, _, cx| {
        assert!(root.chat_panel_is_open(cx));
        assert!(root
            .chat_panel_layout(root.active_chat_detail.as_ref(), cx)
            .1
            .is_some());
        assert!(
            root.chat_panel_visibility.started_at.is_none(),
            "switching tabs does not animate"
        );
    })
    .unwrap();
    switch("busy-shell", &mut visual);
    host.update(&mut visual, |root, _, cx| {
        assert!(!root.chat_panel_is_open(cx))
    })
    .unwrap();

    // A newly launched chat inherits the last collapse choice and cannot reset A.
    store.update(&mut visual, |store, cx| {
            let layout = PaneLayout::single(Some("new-agent"), &[]);
            let conn = store.test_core.db.get().unwrap();
            conn.execute("INSERT INTO sessions (id, status, agent_runtime, agent_command) VALUES ('new-agent', 'stopped', 'codex', 'codex')", []).unwrap();
            runner_daemon::repo::node::create_tab(&conn, None, "new", 0, &layout.serialize().unwrap()).unwrap();
            drop(conn);
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
    visual.run_until_parked();
    host.update(&mut visual, |root, window, cx| {
        root.finish_local_chat_launch(
            Some("new-agent".into()),
            Ok(crate::surfaces::front_page::client::ChatLaunched {
                session: "new-agent".into(),
                layout: None,
            }),
            window,
            cx,
        );
        assert!(!root.chat_panel_is_open(cx));
        assert_eq!(root.chat_surfaces.open, Default::default());
    })
    .unwrap();
    switch("direct-agent", &mut visual);
    assert_eq!(
        chat_surfaces(&host, &mut visual),
        surfaces_of(&[PanelViewId::Info, PanelViewId::Files], PanelViewId::Files)
    );
    // Settings closes disabled surfaces in both the shown and saved tabs.
    switch("busy-shell", &mut visual);
    host.update(&mut visual, |root, _, cx| {
        root.chat_surfaces.open.open(PanelViewId::Files);
        root.update_app_settings(cx, false, |settings| {
            settings.files_view = false;
            true
        });
    })
    .unwrap();
    visual.run_until_parked();
    switch("direct-agent", &mut visual);
    assert_eq!(
        chat_surfaces(&host, &mut visual),
        surfaces_of(&[PanelViewId::Info], PanelViewId::Info)
    );
    switch("busy-shell", &mut visual);
    assert_eq!(
        chat_surfaces(&host, &mut visual),
        surfaces_of(&[PanelViewId::Info], PanelViewId::Info)
    );
}

#[test]
fn settings_remove_the_chat_panel_and_its_toggle() {
    let (_temp, _cx, _store, host, mut visual) = panel_root("direct-agent");
    let set = |visual: &mut gpui::VisualTestContext, update: fn(&mut AppSettings)| {
        host.update(visual, |root, _, cx| {
            root.update_app_settings(cx, false, |settings| {
                update(settings);
                true
            });
        })
        .unwrap();
        visual.run_until_parked();
        host.read_with(visual, |root, _| root.chat_panel_visibility.target)
            .unwrap()
    };
    assert!(visual.debug_bounds("CHAT_SIDE_PANEL").is_some());

    // Show the side panel off: the open panel closes and no toggle
    // offers it back, collapsed or not.
    assert_eq!(
        set(&mut visual, |settings| settings.show_side_panel = false),
        0.
    );
    assert!(visual.debug_bounds("CHAT_PANEL_TOGGLE").is_none());
    host.update(&mut visual, |root, _, cx| {
        root.set_chat_panel_open(false, cx)
    })
    .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("CHAT_PANEL_TOGGLE").is_none());
    set(&mut visual, |settings| settings.show_side_panel = true);
    assert!(visual.debug_bounds("CHAT_PANEL_TOGGLE").is_some());

    // Every view off: the chat has no panel and no toggle.
    set(&mut visual, |settings| {
        settings.info_view = false;
        settings.files_view = false;
    });
    assert!(visual.debug_bounds("CHAT_PANEL_TOGGLE").is_none());
    host.update(&mut visual, |root, _, cx| {
        root.set_chat_panel_open(true, cx)
    })
    .unwrap();
    visual.run_until_parked();
    assert_eq!(
        host.read_with(&visual, |root, _| root.chat_panel_visibility.target)
            .unwrap(),
        0.
    );

    // One surface back: the panel opens on its permanent tab, with no
    // × and no `+`.
    assert_eq!(set(&mut visual, |settings| settings.files_view = true), 1.);
    assert!(visual.debug_bounds("PANEL_TAB Files").is_some());
    assert!(visual.debug_bounds("PANEL_TAB_CLOSE Files").is_none());
    assert!(visual.debug_bounds("PANEL_ADD_SURFACE").is_none());
    assert!(visual.debug_bounds("PANEL_TAB Info").is_none());

    // Info back: Settings closed its tab, so the chat shows the list.
    set(&mut visual, |settings| settings.info_view = true);
    assert!(visual.debug_bounds("PANEL_EMPTY").is_some());
    assert!(visual.debug_bounds("PANEL_ADD_SURFACE").is_some());
}

#[test]
fn cached_chat_header_spans_the_chat_column_with_or_without_the_side_panel() {
    use crate::{AppRoute, NativeRoot};
    use gpui::{TestAppContext, VisualTestContext};

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for panel_open in [false, true] {
        for zoom in [1., 1.5] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            store.update(&mut cx, |store, cx| {
                crate::app_store::seed_mixed_working_sessions(store);
                store
                    .test_core
                    .db
                    .get()
                    .unwrap()
                    .execute(
                        "UPDATE sessions SET status = 'stopped' WHERE mission_id IS NULL",
                        [],
                    )
                    .unwrap();
                store.refresh(crate::app_store::StoreRefreshKind::All, cx);
                store.settings.chat_panel_open = panel_open;
                store.settings.app_zoom = zoom;
            });
            let host = cx.add_window(|window, cx| {
                let mut root = NativeRoot::new(
                    "chat-header-width".into(),
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
                root
            });
            let mut visual = VisualTestContext::from_window(host.into(), &cx);
            for width in [1000., 1600.] {
                visual.simulate_resize(size(px(width), px(900.)));
                visual.run_until_parked();
                let column = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
                let header = visual.debug_bounds("PANE_HEADER:direct-agent").unwrap();
                assert!((header.left() - column.left() - px(zoom)).abs() <= px(1.));
                assert!(
                    (header.right() - column.right() + px(zoom)).abs() <= px(1.),
                    "panel={panel_open}, zoom={zoom}, width={width}"
                );
                if panel_open {
                    let panel = visual.debug_bounds("CHAT_SIDE_PANEL").unwrap();
                    let gutter = visual.debug_bounds("CHAT_PANEL_GUTTER").unwrap();
                    assert!((panel.left() - column.right() - px(6. * zoom)).abs() <= px(0.5));
                    assert!((gutter.center().x - panel.left()).abs() <= px(0.5));
                    assert_eq!(gutter.size.width, px(6. * zoom));
                }
            }
        }
    }
}

fn panel_role() -> runner_core::protocol::model::Role {
    runner_core::protocol::model::Role {
        id: "role-architect".into(),
        handle: "architect".into(),
        display_name: "Architect".into(),
        runtime: "claude-code".into(),
        command: "claude".into(),
        args: Vec::new(),
        working_dir: None,
        system_prompt: None,
        env: Default::default(),
        model: Some("fable".into()),
        effort: Some("xhigh".into()),
        codex_speed: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

struct ChatPanel {
    detail: DirectSessionEntry,
    role: Option<runner_core::protocol::model::Role>,
    width: f32,
    copy: gpui::Entity<runner_app::ui::CopyValueButton>,
}

impl gpui::Render for ChatPanel {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::prelude::*;
        gpui::div()
            .w(gpui::rems(self.width / 16.))
            .p_5()
            .child(super::chat_panel_content(
                &self.detail,
                self.role.as_ref(),
                self.width,
                self.copy.clone(),
                super::ChatPanelActions {
                    open_role: self.role.as_ref().map(|_| {
                        std::rc::Rc::new(|_: &mut gpui::Window, _: &mut gpui::App| {})
                            as super::ClickHandler
                    }),
                    toggle_prompt: std::rc::Rc::new(|_, _| {}),
                    expanded: false,
                    view: cx.entity_id(),
                },
                self.detail.claude_account.as_ref().map(|account| {
                    (
                        "Work".into(),
                        Some("fixture@example.test".into()),
                        account.folder.clone(),
                    )
                }),
                cx,
            ))
    }
}

fn panel_harness(
    detail: DirectSessionEntry,
    role: Option<runner_core::protocol::model::Role>,
    width: f32,
) -> (gpui::TestAppContext, gpui::VisualTestContext) {
    let mut cx = gpui::TestAppContext::single();
    let window = cx.add_window(move |_, cx| ChatPanel {
        copy: cx.new(|cx| {
            runner_app::ui::CopyValueButton::new(cx.focus_handle(), None, "Copy session_key")
        }),
        detail,
        role,
        width,
    });
    let visual = gpui::VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(size(px(500.), px(800.)));
    visual.run_until_parked();
    (cx, visual)
}

#[test]
fn claude_header_preserves_account_spacing_and_controls_in_narrow_panes() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx,|store,cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            {
                let conn = store.test_core.db.get().unwrap();
                conn.execute("UPDATE sessions SET status='stopped', agent_runtime='claude-code', agent_command='claude', title='Claude Code' WHERE id='direct-agent'",[]).unwrap();
                let accounts = serde_json::json!({"accounts":[{"id":"native","name":"Personal","folder":"/tmp/native","native":true},{"id":"work","name":"Work","folder":"/tmp/work","native":false}],"main_id":"native","active_id":"work"});
                conn.execute("INSERT INTO _app_state(key,value) VALUES('claude_accounts',?1)",[accounts.to_string()]).unwrap();
            }
            store.settings.chat_panel_open = true;
            store.settings.keymap_overrides.insert("resume-session".into(), Some(crate::keymap::KeyCombo { meta: false, ctrl: false, alt: false, shift: false, code: "F8".into(), label: None, shift_optional: false }));
            store.refresh(crate::app_store::StoreRefreshKind::All,cx);
        });
    let sidebar_width = store.read_with(&cx, |store, _| store.settings.sidebar_width);
    let panel_width = store.read_with(&cx, |store, _| store.settings.chat_panel_width);
    let host = cx.add_window(|window, cx| {
        let mut root = crate::NativeRoot::new(
            "account-header".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.apply_tab_rows(cx);
        assert!(root.tabs.activate_session("direct-agent"));
        root.route = crate::AppRoute::Chat;
        root.sync_active_chat_detail(cx);
        root.chat_surfaces
            .open
            .open(crate::surfaces::side_panel::PanelViewId::Info);
        root
    });
    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    for zoom in [1., 1.3] {
        host.update(&mut visual, |root, window, cx| {
            root.set_zoom(zoom, window, cx)
        })
        .unwrap();
        for width in [240., 360., 640.] {
            visual.simulate_resize(size(
                px((width + sidebar_width + panel_width + 20.) * zoom),
                px(900.),
            ));
            visual.run_until_parked();
            let header = visual.debug_bounds("PANE_HEADER:direct-agent").unwrap();
            let chip = visual.debug_bounds("CHAT_ACCOUNT_CHIP");
            let name = visual.debug_bounds("PANE_TITLE:Claude Code").unwrap();
            if let Some(chip) = chip {
                assert!(chip.top() >= header.top() && chip.bottom() <= header.bottom());
                assert!((chip.left() - name.right() - px(8. * zoom)).abs() < px(1.));
            }
            assert_eq!(chip.is_some(), width >= 360.);
            let indicator = visual.debug_bounds("ISLAND_STATUS_INDICATOR").unwrap();
            let button = visual.debug_bounds("PANE_LIFECYCLE:Resume chat").unwrap();
            let actions = visual.debug_bounds("PANE_HEADER_ACTIONS").unwrap();
            assert!(indicator.left() >= chip.map_or(name.right(), |chip| chip.right()));
            assert!(
                    button.right() <= actions.left(),
                    "{width}: {button:?}, {actions:?}, name={name:?}, chip={chip:?}, status={indicator:?}, identity={:?}", visual.debug_bounds("ISLAND_HEADER_IDENTITY")
                );
            assert!(actions.right() <= header.right());
            assert_eq!(visual.debug_bounds("ISLAND_STATUS_LABEL").is_some(), width >= 360., "width={width}, name={name:?}, chip={chip:?}, header={header:?}, actions={actions:?}, status={indicator:?}");
            if width == 360. {
                assert!(
                    name.size.width >= px(24. * zoom) && name.size.width < px(70. * zoom),
                    "{name:?}"
                );
            }
            assert!(visual
                .debug_bounds("CHAT_STOPPED_TITLE:Chat stopped")
                .is_some());
            assert!(visual.debug_bounds("BUTTON_SHORTCUT:F8").is_some());
        }
    }
    host.update(&mut visual, |root, _, cx| {
        root.update_app_settings(cx, false, |settings| {
            settings.chat_panel_open = false;
            settings
                .keymap_overrides
                .insert("resume-session".into(), None);
            true
        });
        root.set_chat_panel_open(false, cx);
        root.chat_panel_visibility = crate::surfaces::root::SidebarVisibilityTransition::new(false);
    })
    .unwrap();
    visual.simulate_resize(size(px((240. + sidebar_width + 14.) * 1.3), px(900.)));
    visual.run_until_parked();
    let actions = visual.debug_bounds("PANE_HEADER_ACTIONS").unwrap();
    let lifecycle = visual.debug_bounds("PANE_LIFECYCLE:Resume chat").unwrap();
    assert!(lifecycle.right() <= actions.left(), "lifecycle={lifecycle:?}, actions={actions:?}, identity={:?}, chip={:?}, name={:?}, status={:?}", visual.debug_bounds("ISLAND_HEADER_IDENTITY"), visual.debug_bounds("CHAT_ACCOUNT_CHIP"), visual.debug_bounds("PANE_TITLE:Claude Code"), visual.debug_bounds("ISLAND_STATUS_INDICATOR"));
    assert!(visual.debug_bounds("ISLAND_STATUS_LABEL").is_none());
    assert!(visual.debug_bounds("BUTTON_SHORTCUT:F8").is_none());
}

#[test]
fn account_is_beside_runtime_and_config_is_in_the_metadata_rows() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let mut detail = direct_session("claude-code", true, true);
    detail.claude_account = Some(runner_core::protocol::ClaudeSessionAccount {
        account_id: Some("work".into()),
        folder: "/tmp/work".into(),
        native: false,
        pinned: false,
        origin_folder: "/tmp/native".into(),
        permission_mode: "default".into(),
    });
    for role in [None, Some(panel_role())] {
        let (_cx, mut visual) = panel_harness(detail.clone(), role, 320.);
        let runtime = visual.debug_bounds("SETUP_ROW RUNTIME").unwrap();
        let account = visual.debug_bounds("SETUP_ROW Account").unwrap();
        assert!((runtime.top() - account.top()).abs() <= px(1.));
        assert!(account.left() > runtime.left());
        let config = visual.debug_bounds("CHAT_PANEL_META /tmp/work").unwrap();
        assert!(config.top() > account.bottom());
    }
}

#[test]
fn panel_prompt_only_shows_nonblank_text_and_only_long_prompts_have_a_toggle() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for prompt in ["", " \n ", "# Short\n\nPrompt", &"line\n".repeat(60)] {
        let mut role = panel_role();
        role.system_prompt = Some(prompt.into());
        let (_cx, mut visual) = panel_harness(
            direct_session("codex", true, true),
            Some(role),
            crate::app_settings::CHAT_PANEL_MIN,
        );
        let link = visual.debug_bounds("CHAT_PANEL_OPEN_ROLE").unwrap();
        let label = visual.debug_bounds("CHAT_PANEL_LABEL Role").unwrap();
        assert!((label.center().y - link.center().y).abs() <= gpui::px(1.));
        assert!(link.right() <= gpui::px(crate::app_settings::CHAT_PANEL_MIN));
        if !prompt.trim().is_empty() {
            let label = visual
                .debug_bounds("CHAT_PANEL_LABEL System prompt")
                .unwrap();
            let meta = visual.debug_bounds("CHAT_PANEL_PROMPT_META").unwrap();
            assert!((label.center().y - meta.center().y).abs() <= gpui::px(1.));
            assert!(meta.right() <= gpui::px(crate::app_settings::CHAT_PANEL_MIN));
        }
        assert_eq!(
            visual.debug_bounds("CHAT_PANEL_PROMPT_TEXT").is_some(),
            !prompt.trim().is_empty()
        );
        assert_eq!(
            visual.debug_bounds("CHAT_PANEL_PROMPT_TOGGLE").is_some(),
            prompt.lines().count() > crate::surfaces::profile_page::PREVIEW_LINES
        );
    }
}

#[test]
fn role_chat_panel_renders_avatar_runtime_model_and_effort() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let mut detail = direct_session("claude-code", true, true);
    detail.agent_model = Some("fable".into());
    detail.agent_effort = Some("xhigh".into());
    let (_cx, mut visual) = panel_harness(detail, Some(panel_role()), 320.);
    let avatar = visual.debug_bounds("CHAT_PANEL_AVATAR architect").unwrap();
    assert_eq!(avatar.size, size(px(40.), px(40.)));
    for text in [
        "COLUMN_TEXT Architect",
        "COLUMN_TEXT @architect",
        "COLUMN_TEXT Claude Code",
        "COLUMN_TEXT fable",
        "COLUMN_TEXT xhigh",
    ] {
        assert!(
            visual.debug_bounds(text).is_some(),
            "missing rendered {text}"
        );
    }
    assert!(visual.debug_bounds("SETUP_ROW RUNTIME").is_some());
    let model = visual.debug_bounds("SETUP_ROW MODEL").unwrap();
    let effort = visual.debug_bounds("SETUP_ROW EFFORT").unwrap();
    assert_eq!(model.top(), effort.top());
    assert_eq!(model.size.width, effort.size.width);
}

#[test]
fn runtime_chat_panel_renders_mark_and_defaults_without_runtime_row() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for variant in [
        crate::theme::ThemeVariant::Carbon,
        crate::theme::ThemeVariant::RunnerLight,
    ] {
        crate::theme::set_active_variant(variant);
        for (model, effort, default_row) in [
            (None, None, "SETUP_ROW EFFORT"),
            (None, Some("xhigh"), "SETUP_ROW MODEL"),
            (Some("gpt-5.6-sol"), None, "SETUP_ROW EFFORT"),
        ] {
            let mut detail = direct_session("codex", true, true);
            detail.agent_model = model.map(str::to_owned);
            detail.agent_effort = effort.map(str::to_owned);
            let (_cx, mut visual) = panel_harness(detail, None, 320.);
            assert!(visual.debug_bounds("CHAT_PANEL_OPEN_ROLE").is_none());
            assert!(visual.debug_bounds("CHAT_PANEL_PROMPT_TEXT").is_none());
            let mark = visual.debug_bounds("CHAT_PANEL_MARK codex").unwrap();
            assert_eq!(mark.size, size(px(40.), px(40.)));
            assert!(visual.debug_bounds("COLUMN_TEXT Codex").is_some());
            assert!(visual.debug_bounds("SETUP_ROW RUNTIME").is_none());
            let model = visual.debug_bounds("SETUP_ROW MODEL").unwrap();
            let effort = visual.debug_bounds("SETUP_ROW EFFORT").unwrap();
            let row = visual.debug_bounds(default_row).unwrap();
            let default = visual.debug_bounds("COLUMN_TEXT default").unwrap();
            assert_eq!(model.top(), effort.top());
            assert_eq!(model.size, effort.size);
            assert!(default.top() >= row.top() && default.bottom() <= row.bottom());
        }
    }
}

#[test]
fn role_chat_panel_renders_effective_runtime_and_command_override() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let mut detail = direct_session("codex", true, true);
    detail.agent_command = "codex-override".into();
    detail.agent_model = Some("gpt-5.6-sol".into());
    detail.agent_effort = Some("high".into());
    let (_cx, mut visual) = panel_harness(detail, Some(panel_role()), 320.);
    assert!(visual.debug_bounds("COLUMN_TEXT Codex").is_some());
    assert!(visual.debug_bounds("COLUMN_TEXT Claude Code").is_none());
    assert!(visual
        .debug_bounds("CHAT_PANEL_META codex-override")
        .is_some());
    assert!(visual.debug_bounds("CHAT_PANEL_META claude").is_none());
    assert!(visual.debug_bounds("COLUMN_TEXT gpt-5.6-sol").is_some());
    assert!(visual.debug_bounds("COLUMN_TEXT high").is_some());
}

#[test]
fn role_chat_panel_renders_chat_cwd_with_legacy_role_fallback() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for (chat_cwd, role_cwd, expected) in [
        (Some("/chat"), Some("/role"), "CHAT_PANEL_META /chat"),
        (Some("/chat"), None, "CHAT_PANEL_META /chat"),
        (None, Some("/role"), "CHAT_PANEL_META /role"),
    ] {
        let mut detail = direct_session("claude-code", true, true);
        detail.cwd = chat_cwd.map(str::to_owned);
        let mut role = panel_role();
        role.working_dir = role_cwd.map(str::to_owned);
        let (_cx, mut visual) = panel_harness(detail, Some(role), 320.);
        assert!(
            visual.debug_bounds(expected).is_some(),
            "missing rendered {expected}"
        );
        if chat_cwd.is_some() {
            assert!(visual.debug_bounds("CHAT_PANEL_META /role").is_none());
        }
    }
}

#[test]
fn chat_panel_identity_and_setup_fit_at_minimum_width() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let mut role = panel_role();
    role.display_name = "A very long architect display name that must truncate".into();
    role.handle = "a-very-long-architect-handle".into();
    let mut detail = direct_session("antigravity", false, false);
    detail.agent_model = Some("a-very-long-model-name-that-must-truncate".into());
    detail.agent_effort = Some("xhigh".into());
    let (_cx, mut visual) = panel_harness(detail, Some(role), crate::app_settings::CHAT_PANEL_MIN);
    for text in [
        "COLUMN_TEXT A very long architect display name that must truncate",
        "COLUMN_TEXT @a-very-long-architect-handle",
        "COLUMN_TEXT Antigravity CLI",
        "COLUMN_TEXT a-very-long-model-name-that-must-truncate",
        "COLUMN_TEXT xhigh",
    ] {
        let bounds = visual
            .debug_bounds(text)
            .unwrap_or_else(|| panic!("missing rendered {text}"));
        assert!(bounds.size.width > px(0.), "{text}: {bounds:?}");
        assert!(
            bounds.left() >= px(35.) && bounds.right() <= px(165.),
            "{text}: {bounds:?}"
        );
        assert!(bounds.size.height <= px(24.), "{text} wrapped: {bounds:?}");
    }
}

#[test]
fn split_tab_shows_one_archiving_pill_only_when_every_pane_archives() {
    let single = PaneLayout::single(Some("a"), &["a".into()]);
    assert!(!split_tab_archiving(&single, |_| true));

    let mut split = PaneLayout::single(Some("a"), &["a".into()]);
    let second = split.split("p1", SplitOrientation::Row).unwrap();
    assert!(split_tab_archiving(&split, |_| true));

    split.assign_session(&second, "b").unwrap();
    assert!(split_tab_archiving(&split, |_| true));
    assert!(!split_tab_archiving(&split, |session_id| session_id == "a"));
    assert!(!split_tab_archiving(&split, |_| false));
}

#[test]
fn adjacent_pane_index_wraps_in_both_directions() {
    assert_eq!(adjacent_pane_index(0, 3, -1), Some(2));
    assert_eq!(adjacent_pane_index(2, 3, 1), Some(0));
    assert_eq!(adjacent_pane_index(1, 3, 1), Some(2));
    assert_eq!(adjacent_pane_index(0, 1, 1), None);
    assert_eq!(adjacent_pane_index(3, 3, -1), None);
}

#[test]
fn drop_side_uses_normalized_edges_and_breaks_diagonal_ties_left_then_up() {
    let pane = Bounds::new(point(px(100.), px(200.)), size(px(800.), px(400.)));
    for (x, y, side) in [
        (0.1, 0.5, DropSide::Left),
        (0.9, 0.5, DropSide::Right),
        (0.5, 0.1, DropSide::Up),
        (0.5, 0.9, DropSide::Down),
        (0.375, 0.5, DropSide::Left),
        (0.25, 0.25, DropSide::Left),
        (0.25, 0.75, DropSide::Left),
        (0.75, 0.25, DropSide::Up),
        (0.75, 0.75, DropSide::Right),
        (0.5, 0.5, DropSide::Left),
        (0., 0., DropSide::Left),
        (1., 0., DropSide::Up),
        (0., 1., DropSide::Left),
        (1., 1., DropSide::Right),
    ] {
        assert_eq!(
            drop_side(pane, pane.origin + point(px(800. * x), px(400. * y))),
            side,
            "({x}, {y})"
        );
    }
}

#[test]
fn drop_preview_is_the_destination_half_in_pane_coordinates() {
    let pane = Bounds::new(point(px(100.), px(200.)), size(px(800.), px(400.)));
    for (side, x, y, width, height) in [
        (DropSide::Left, 100., 200., 400., 400.),
        (DropSide::Right, 500., 200., 400., 400.),
        (DropSide::Up, 100., 200., 800., 200.),
        (DropSide::Down, 100., 400., 800., 200.),
    ] {
        assert_eq!(
            drop_preview(pane, side),
            Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
        );
    }
}

#[test]
fn drop_allowed_respects_both_split_axes_at_each_zoom() {
    for zoom in [1., 1.5] {
        for side in [DropSide::Left, DropSide::Right] {
            assert!(drop_allowed(
                Some(size(px(480. * zoom), px(160. * zoom))),
                side,
                zoom
            ));
            assert!(!drop_allowed(
                Some(size(px(480. * zoom - 1.), px(160. * zoom))),
                side,
                zoom
            ));
        }
        for side in [DropSide::Up, DropSide::Down] {
            assert!(drop_allowed(
                Some(size(px(240. * zoom), px(320. * zoom))),
                side,
                zoom
            ));
            assert!(!drop_allowed(
                Some(size(px(240. * zoom), px(320. * zoom - 1.))),
                side,
                zoom
            ));
        }
    }
}

#[test]
fn header_fork_state_uses_capability_key_and_focused_pane_kind() {
    for runtime in ["claude-code", "codex", "pi"] {
        let entry = direct_session(runtime, true, true);
        assert_eq!(
            header_fork_state(Some(&entry), false),
            HeaderForkState::Enabled
        );
    }

    for runtime in ["trae", "copilot", "antigravity"] {
        let entry = direct_session(runtime, false, false);
        assert_eq!(
            header_fork_state(Some(&entry), false),
            HeaderForkState::Disabled(None)
        );
    }
    let waiting = direct_session("codex", true, false);
    assert_eq!(
        header_fork_state(Some(&waiting), false),
        HeaderForkState::Disabled(Some("No session key captured yet"))
    );
    let shell = direct_session("shell", false, false);
    assert_eq!(
        header_fork_state(Some(&shell), false),
        HeaderForkState::Hidden
    );
    assert_eq!(header_fork_state(None, false), HeaderForkState::Hidden);
    assert_eq!(
        header_fork_state(Some(&waiting), true),
        HeaderForkState::Hidden
    );
}

#[test]
fn starting_overlay_names_fork_materialization_before_normal_startup() {
    assert_eq!(
        starting_overlay_label("claude-code", true),
        Some("Forking chat…")
    );
    assert_eq!(starting_overlay_label("codex", false), None);
    assert_eq!(
        starting_overlay_label("shell", false),
        Some("Starting terminal…")
    );
}

#[test]
fn pane_identity_branches_for_chat_terminal_and_empty_panes() {
    assert_eq!(pane_identity_icon(Some("codex")).asset_path(), "openai.svg");
    assert_eq!(
        pane_identity_icon(Some("claude-code")).asset_path(),
        "claude.svg"
    );
    assert_eq!(pane_identity_icon(Some("trae")).asset_path(), "trae.svg");
    assert_eq!(
        pane_identity_icon(Some("copilot")).asset_path(),
        "copilot.svg"
    );
    assert_eq!(pane_identity_icon(Some("pi")).asset_path(), "pi.svg");
    assert_eq!(
        pane_identity_icon(Some("antigravity")).asset_path(),
        "antigravity-icon.png"
    );
    assert_eq!(
        pane_identity_icon(Some("unknown")).asset_path(),
        "message-square.svg"
    );

    assert_eq!(
        pane_identity_icon(Some("shell")).asset_path(),
        "square-terminal.svg"
    );
    assert_eq!(side_panel_surface(true), PanelSurface::Terminal);
    assert_eq!(side_panel_surface(false), PanelSurface::Chat);
    let collapsed = AppSettings {
        chat_panel_open: false,
        ..AppSettings::default()
    };
    assert!(side_panel_available(&collapsed, PanelSurface::Chat));
    assert!(side_panel_available(&collapsed, PanelSurface::Terminal));

    // Settings → Side panel → Show the side panel off: no panel and no
    // toggle anywhere, the open state and the switches kept.
    let hidden = AppSettings {
        show_side_panel: false,
        ..AppSettings::default()
    };
    for surface in [PanelSurface::Chat, PanelSurface::Terminal] {
        assert!(!side_panel_available(&hidden, surface));
    }
    assert!(hidden.chat_panel_open && hidden.info_view && hidden.files_view);
    assert!(!hidden.chat_panel_shown() && !hidden.mission_rail_shown());

    // Every view a surface has turned off: that surface has no panel.
    let files_off = AppSettings {
        files_view: false,
        ..AppSettings::default()
    };
    assert!(side_panel_available(&files_off, PanelSurface::Chat));
    assert!(!side_panel_available(&files_off, PanelSurface::Terminal));
    let all_off = AppSettings {
        info_view: false,
        ..files_off
    };
    assert!(!side_panel_available(&all_off, PanelSurface::Chat));

    assert_eq!(pane_identity_icon(None).asset_path(), "plus.svg");
    assert_eq!(pane_close_behavior(None), PaneCloseBehavior::LayoutOnly);
}

#[test]
fn only_unfocused_panes_in_a_split_are_faded() {
    assert_eq!(UNFOCUSED_PANE_OPACITY, 0.7);
    assert_eq!(pane_body_opacity(true, false), UNFOCUSED_PANE_OPACITY);
    assert_eq!(pane_body_opacity(true, true), 1.);
    assert_eq!(pane_body_opacity(false, false), 1.);
}

#[test]
fn title_menu_lists_rename_fork_and_pin_and_never_stop_or_archive() {
    use super::PaneMenuAction::{Fork, Pin, Rename};
    let every = |_: &[&str]| true;
    assert_eq!(
        pane_menu_actions("codex", false, true, every),
        [Rename, Fork, Pin]
    );
    assert_eq!(
        pane_menu_actions("shell", false, true, every),
        [Rename, Pin]
    );
    assert_eq!(
        pane_menu_actions("codex", true, true, every),
        [Rename, Pin],
        "a secondary pane leaves forking to the window that controls it"
    );
    assert_eq!(
        pane_menu_actions("codex", false, false, every),
        [Rename, Fork]
    );

    let only = |offered: &'static [&'static str]| {
        move |features: &[&str]| features.iter().all(|feature| offered.contains(feature))
    };
    assert_eq!(
        pane_menu_actions("codex", false, true, only(&["rename"])),
        [Rename]
    );
    assert_eq!(pane_menu_actions("codex", false, true, only(&["fork"])), []);
    assert_eq!(
        pane_menu_actions("codex", false, true, only(&["catalog", "fork", "pin"])),
        [Fork, Pin]
    );
    assert_eq!(
        pane_menu_actions("shell", false, true, only(&["catalog", "fork"])),
        []
    );

    let labels = |items: &[crate::UiMenuItem]| {
        items
            .iter()
            .map(|item| item.label.to_string())
            .collect::<Vec<_>>()
    };
    let chat = pane_menu_items(&[Rename, Fork, Pin], false, false, false, None);
    assert_eq!(labels(&chat), ["Rename…", "Fork chat", "Pin"]);
    assert_eq!(
        chat.iter()
            .map(|item| item.icon.as_deref())
            .collect::<Vec<_>>(),
        [Some("pencil.svg"), Some("git-fork.svg"), Some("pin.svg")]
    );
    assert!(chat.iter().all(|item| !item.disabled
        && !item.destructive
        && item.shortcut.is_none()
        && !item.separator_before));

    let pinned = pane_menu_items(&[Rename, Pin], true, false, false, None);
    assert_eq!(labels(&pinned), ["Rename…", "Unpin"]);
    assert_eq!(pinned[1].icon.as_deref(), Some("pin-off.svg"));

    let waiting = pane_menu_items(
        &[Rename, Fork, Pin],
        false,
        false,
        true,
        Some("No session key captured yet"),
    );
    assert!(!waiting[0].disabled && waiting[1].disabled && !waiting[2].disabled);
    assert_eq!(
        waiting[1].tooltip.as_deref(),
        Some("No session key captured yet")
    );

    let secondary = pane_menu_items(&[Rename, Pin], false, true, false, None);
    assert!(secondary.iter().all(|item| item.disabled));
}

#[test]
fn maximize_holds_the_focused_pane_and_ends_on_navigation_split_close_or_single_pane() {
    use std::collections::HashMap;
    let mut layout = PaneLayout::single(Some("chat"), &[]);
    let left = layout.focused_pane_id.clone();
    let mut maximized = HashMap::new();
    assert!(!toggle_maximized(&mut maximized, &layout, &left));
    assert!(
        maximized.is_empty(),
        "a single pane has nothing to maximize"
    );

    let right = layout.split(&left, SplitOrientation::Row).unwrap();
    let runner_app::pane_layout::PaneNode::Split(split) = &layout.root else {
        unreachable!("the tab was just split");
    };
    let split_id = split.id.clone();
    assert!(layout.set_split_sizes(&split_id, [70., 30.]));
    let before = layout.serialize().unwrap();
    assert!(layout.focus_pane(&left));
    assert!(toggle_maximized(&mut maximized, &layout, &left));
    assert_eq!(maximized_leaf(&maximized, &layout).unwrap().id, left);
    assert_eq!(
        layout.serialize().unwrap(),
        before,
        "maximize is view state and never reaches the persisted layout"
    );

    assert!(!toggle_maximized(&mut maximized, &layout, &left));
    assert!(maximized_leaf(&maximized, &layout).is_none());
    assert!(maximized.is_empty());

    assert!(toggle_maximized(&mut maximized, &layout, &left));
    assert!(layout.focus_pane(&right));
    assert!(
        maximized_leaf(&maximized, &layout).is_none(),
        "focusing a hidden pane restores the split"
    );
    prune_maximized(&mut maximized, &layout);
    assert!(maximized.is_empty());
    assert!(layout.focus_pane(&left));
    assert!(
        maximized_leaf(&maximized, &layout).is_none(),
        "returning to the pane does not maximize it again"
    );

    assert!(toggle_maximized(&mut maximized, &layout, &left));
    assert!(layout.close_pane(&left));
    assert!(maximized_leaf(&maximized, &layout).is_none());
    prune_maximized(&mut maximized, &layout);
    assert!(
        maximized.is_empty(),
        "a tab reduced to one pane drops the state"
    );

    let mut layout = PaneLayout::single(Some("chat"), &[]);
    let first = layout.focused_pane_id.clone();
    let second = layout.split(&first, SplitOrientation::Row).unwrap();
    let third = layout.split(&second, SplitOrientation::Column).unwrap();
    assert!(layout.focus_pane(&second));
    assert!(toggle_maximized(&mut maximized, &layout, &second));
    assert!(layout.close_pane(&second));
    prune_maximized(&mut maximized, &layout);
    assert!(
        maximized.is_empty(),
        "closing the maximized pane leaves the rest unmaximized"
    );
    assert_eq!(layout.root.leaves().len(), 2);

    assert!(layout.focus_pane(&third));
    assert!(toggle_maximized(&mut maximized, &layout, &third));
    let mut other = PaneLayout::single(Some("other"), &[]);
    other.id = "other-tab".into();
    let other_left = other.focused_pane_id.clone();
    other.split(&other_left, SplitOrientation::Row).unwrap();
    prune_maximized(&mut maximized, &other);
    assert_eq!(
        maximized_leaf(&maximized, &layout).unwrap().id,
        third,
        "each tab keeps its own maximized pane"
    );
    assert!(maximized_leaf(&maximized, &other).is_none());
}

#[test]
fn pane_close_archives_chats_closes_terminals_and_drops_empty_panes() {
    assert_eq!(pane_close_behavior(None), PaneCloseBehavior::LayoutOnly);
    assert_eq!(
        pane_close_behavior(Some("shell")),
        PaneCloseBehavior::CloseTerminal
    );
    assert_eq!(
        pane_close_behavior(Some("codex")),
        PaneCloseBehavior::ArchiveChat
    );
    assert_eq!(
        pane_close_behavior(Some("claude-code")),
        PaneCloseBehavior::ArchiveChat
    );
}

#[test]
fn archive_chat_confirm_body_warns_only_while_the_agent_runs() {
    assert_eq!(
            archive_chat_confirm_body("@coder", true),
            "@coder is still running. Archiving stops it. You can restore the chat from Settings → Archived."
        );
    assert_eq!(
        archive_chat_confirm_body("@coder", false),
        "You can restore @coder from Settings → Archived."
    );
    assert_eq!(
        archive_chat_confirm_body("this chat", false),
        "You can restore this chat from Settings → Archived."
    );
}

#[test]
fn start_surface_badges_follow_default_rebound_and_unbound_bindings() {
    let mut overrides = keymap::KeymapOverrides::new();
    for (binding, mac, windows) in [
        ("new-chat", "⌘N", "Ctrl+N"),
        ("new-terminal", "⌘T", "Ctrl+Shift+T"),
        ("new-mission", "⇧⌘M", "Ctrl+Shift+M"),
    ] {
        let expected = if cfg!(any(windows, target_os = "linux")) {
            windows
        } else {
            mac
        };
        assert_eq!(
            super::start_action_shortcut(binding, &overrides).as_deref(),
            Some(expected)
        );
        let mut rebound = keymap::entry(binding).unwrap().default.clone();
        rebound.meta = false;
        rebound.ctrl = true;
        rebound.shift = false;
        rebound.code = "KeyJ".into();
        rebound.label = None;
        overrides.insert(binding.into(), Some(rebound));
        let expected = if cfg!(any(windows, target_os = "linux")) {
            "Ctrl+J"
        } else {
            "⌃J"
        };
        assert_eq!(
            super::start_action_shortcut(binding, &overrides).as_deref(),
            Some(expected)
        );
        overrides.insert(binding.into(), None);
        assert_eq!(super::start_action_shortcut(binding, &overrides), None);
    }
}

#[test]
fn start_surface_is_shown_for_no_tab_or_a_single_empty_pane() {
    assert!(super::start_surface_visible(None));
    let mut layout = PaneLayout::single(None, &[]);
    assert!(super::start_surface_visible(Some(&layout)));
    layout.split("p1", SplitOrientation::Row).unwrap();
    assert!(!super::start_surface_visible(Some(&layout)));
    let filled = PaneLayout::single(Some("chat"), &[]);
    assert!(!super::start_surface_visible(Some(&filled)));
}

#[cfg(unix)]
#[test]
fn start_surface_and_split_pane_variants_render_in_their_own_contexts() {
    use crate::{AppRoute, NativeRoot};
    use gpui::{TestAppContext, VisualTestContext};
    use runner_app::pane_layout::TabSet;
    use runner_core::protocol::node::{NodeRow, NodeType};

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for variant in [
        "no-tab",
        "no-tab-error",
        "single-empty",
        "single-empty-error",
        "single-empty-drawer",
        "split-empty",
        "filled",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute("UPDATE sessions SET status = 'stopped'", [])
                .unwrap();
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        let host = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "surface-variants".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            let mut layout =
                PaneLayout::single((variant == "filled").then_some("direct-agent"), &[]);
            if variant == "split-empty" {
                layout.assign_session("p1", "direct-agent").unwrap();
                layout.split("p1", SplitOrientation::Row).unwrap();
            }
            if variant == "single-empty-drawer" {
                layout.add_drawer_shell("busy-shell".into());
            }
            let rows = if variant.starts_with("no-tab") {
                Vec::new()
            } else {
                vec![NodeRow {
                    id: layout.id.clone(),
                    parent_id: None,
                    position: 0,
                    node_type: NodeType::Tab,
                    name: None,
                    ref_id: None,
                    layout: Some(layout.serialize().unwrap()),
                    pinned_position: None,
                    last_completed_at: None,
                    last_viewed_at: None,
                    created_at: String::new(),
                }]
            };
            root.tabs = TabSet::from_rows(&rows);
            root.route = AppRoute::Chat;
            if variant.ends_with("error") {
                root.chat_error = Some("Shell launch failed".into());
            }
            root
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1440.), px(900.)));
        visual.run_until_parked();
        assert_eq!(
            visual.debug_bounds("PANE_DRAG_GRIP:p1").is_some(),
            variant == "split-empty",
            "{variant}"
        );
        assert_eq!(
            visual.debug_bounds("START_SURFACE").is_some(),
            variant.starts_with("no-tab") || variant.starts_with("single-empty"),
            "{variant}"
        );
        assert_eq!(
            visual.debug_bounds("EMPTY_PANE_START").is_some(),
            variant == "split-empty",
            "{variant}"
        );
        assert_eq!(
            visual.debug_bounds("CHAT_ERROR_BANNER").is_some(),
            variant.ends_with("error"),
            "{variant}"
        );
        assert!(
            visual.debug_bounds("CHAT_TERMINAL_DRAWER").is_none(),
            "{variant}"
        );
        if let Some(surface) = visual.debug_bounds("START_SURFACE") {
            assert!(surface.size.height > px(0.), "{variant}");
            assert!(
                visual.debug_bounds("CHAT_TAB_HEADER").is_none(),
                "{variant}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn start_surface_column_stays_440_pixels_wide_and_centred() {
    use crate::{AppRoute, NativeRoot};
    use gpui::{TestAppContext, VisualTestContext};
    use runner_app::pane_layout::TabSet;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "start-surface-width".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.tabs = TabSet::default();
        root.route = AppRoute::Chat;
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    for width in [900., 1440.] {
        visual.simulate_resize(size(px(width), px(900.)));
        visual.run_until_parked();
        let surface = visual.debug_bounds("START_SURFACE").unwrap();
        let column = visual.debug_bounds("START_SURFACE_COLUMN").unwrap();
        assert_eq!(column.size.width, px(440.), "window width {width}");
        assert_eq!(
            column.center().x,
            surface.center().x,
            "window width {width}"
        );
    }
}

#[test]
fn terminal_drawer_tooltip_tracks_rebound_and_unbound_shortcuts() {
    let mut overrides = keymap::KeymapOverrides::new();
    #[cfg(target_os = "macos")]
    assert_eq!(
        terminal_drawer_tooltip(false, &overrides),
        "Show terminal drawer · ⌥F12"
    );
    #[cfg(any(windows, target_os = "linux"))]
    assert_eq!(
        terminal_drawer_tooltip(false, &overrides),
        "Show terminal drawer · Alt+F12"
    );
    #[cfg(target_os = "macos")]
    assert_eq!(
        terminal_drawer_tooltip(true, &overrides),
        "Hide terminal drawer · ⌥F12"
    );
    #[cfg(any(windows, target_os = "linux"))]
    assert_eq!(
        terminal_drawer_tooltip(true, &overrides),
        "Hide terminal drawer · Alt+F12"
    );

    let mut rebound = keymap::entry("toggle-terminal-drawer")
        .unwrap()
        .default
        .clone();
    rebound.alt = false;
    rebound.ctrl = true;
    rebound.code = "Backquote".into();
    overrides.insert("toggle-terminal-drawer".into(), Some(rebound));
    #[cfg(target_os = "macos")]
    assert_eq!(
        terminal_drawer_tooltip(false, &overrides),
        "Show terminal drawer · ⌃`"
    );
    #[cfg(any(windows, target_os = "linux"))]
    assert_eq!(
        terminal_drawer_tooltip(false, &overrides),
        "Show terminal drawer · Ctrl+`"
    );

    overrides.insert("toggle-terminal-drawer".into(), None);
    assert_eq!(
        terminal_drawer_tooltip(false, &overrides),
        "Show terminal drawer"
    );
}

#[test]
fn a_split_needs_room_for_both_halves_and_the_floor_scales_with_zoom() {
    let wide = size(
        px(2. * MIN_SPLIT_PANE_WIDTH),
        px(2. * MIN_SPLIT_PANE_HEIGHT),
    );
    assert!(split_allowed(Some(wide), SplitOrientation::Row, 1.));
    assert!(split_allowed(Some(wide), SplitOrientation::Column, 1.));

    let narrow = size(
        px(2. * MIN_SPLIT_PANE_WIDTH - 1.),
        px(2. * MIN_SPLIT_PANE_HEIGHT),
    );
    assert!(!split_allowed(Some(narrow), SplitOrientation::Row, 1.));
    assert!(split_allowed(Some(narrow), SplitOrientation::Column, 1.));

    let short = size(
        px(2. * MIN_SPLIT_PANE_WIDTH),
        px(2. * MIN_SPLIT_PANE_HEIGHT - 1.),
    );
    assert!(split_allowed(Some(short), SplitOrientation::Row, 1.));
    assert!(!split_allowed(Some(short), SplitOrientation::Column, 1.));

    // The same pixels are too small once the app is zoomed in.
    assert!(!split_allowed(Some(wide), SplitOrientation::Row, 1.5));
    assert!(!split_allowed(Some(wide), SplitOrientation::Column, 1.5));
    let zoomed = size(
        px(3. * MIN_SPLIT_PANE_WIDTH),
        px(3. * MIN_SPLIT_PANE_HEIGHT),
    );
    assert!(split_allowed(Some(zoomed), SplitOrientation::Row, 1.5));
    assert!(split_allowed(Some(zoomed), SplitOrientation::Column, 1.5));

    // An unmeasured pane is not disabled on a size we do not have.
    assert!(split_allowed(None, SplitOrientation::Row, 1.));
}

// `split_pane` is the single gate both routes pass through — the menu
// item and `⌘D` / `⇧⌘D` alike — so what it decides is what is pinned here.
#[test]
fn chat_and_terminal_splits_meet_the_same_floor_on_every_route() {
    for zoom in [1., 1.5] {
        let roomy = size(
            px(2. * MIN_SPLIT_PANE_WIDTH * zoom),
            px(2. * MIN_SPLIT_PANE_HEIGHT * zoom),
        );
        let narrow = size(roomy.width - px(1.), roomy.height);
        let short = size(roomy.width, roomy.height - px(1.));
        for orientation in [SplitOrientation::Row, SplitOrientation::Column] {
            assert_eq!(
                split_decision(Some(roomy), orientation, zoom),
                SplitDecision::Allowed
            );
            assert_eq!(
                split_decision(None, orientation, zoom),
                SplitDecision::Allowed
            );
        }
        assert_eq!(
            split_decision(Some(narrow), SplitOrientation::Row, zoom),
            SplitDecision::Blocked(crate::i18n::text(TOO_SMALL_TO_SPLIT))
        );
        assert_eq!(
            split_decision(Some(narrow), SplitOrientation::Column, zoom),
            SplitDecision::Allowed
        );
        assert_eq!(
            split_decision(Some(short), SplitOrientation::Column, zoom),
            SplitDecision::Blocked(crate::i18n::text(TOO_SMALL_TO_SPLIT))
        );
        assert_eq!(
            split_decision(Some(short), SplitOrientation::Row, zoom),
            SplitDecision::Allowed
        );
    }
}

#[test]
fn a_disabled_menu_item_carries_the_reason_the_action_path_would_report() {
    let narrow = size(px(300.), px(800.));
    let items = split_menu_items(Some(narrow), 1., &keymap::KeymapOverrides::new());
    let SplitDecision::Blocked(reason) = split_decision(Some(narrow), SplitOrientation::Row, 1.)
    else {
        panic!("a 300 px pane cannot split to the right");
    };

    assert!(items[0].disabled);
    assert_eq!(
        items[0].tooltip.as_ref().map(|tip| tip.as_ref()),
        Some(reason)
    );
}

#[test]
fn the_split_menu_offers_two_directions_with_live_pills_and_says_why_it_is_off() {
    let mut overrides = keymap::KeymapOverrides::new();
    let roomy = size(px(1200.), px(800.));
    let items = split_menu_items(Some(roomy), 1., &overrides);
    assert_eq!(
        items
            .iter()
            .map(|item| item.label.as_ref())
            .collect::<Vec<_>>(),
        ["Split Right", "Split Down"]
    );
    assert!(items.iter().all(|item| !item.disabled));
    assert!(items.iter().all(|item| item.tooltip.is_none()));
    #[cfg(target_os = "macos")]
    assert_eq!(
        items
            .iter()
            .map(|item| item.shortcut.as_ref().map(|pill| pill.as_ref()))
            .collect::<Vec<_>>(),
        [Some("⌘D"), Some("⇧⌘D")]
    );

    overrides.insert("split-pane-right".into(), None);
    let unbound = split_menu_items(Some(roomy), 1., &overrides);
    assert_eq!(unbound[0].shortcut, None);
    assert!(unbound[1].shortcut.is_some());

    let cramped = size(px(300.), px(800.));
    let gated = split_menu_items(Some(cramped), 1., &keymap::KeymapOverrides::new());
    assert!(gated[0].disabled);
    assert_eq!(
        gated[0].tooltip.as_ref().map(|tip| tip.as_ref()),
        Some(crate::i18n::text(TOO_SMALL_TO_SPLIT))
    );
    assert!(!gated[1].disabled);
    assert_eq!(gated[1].tooltip, None);
}

#[test]
fn pane_rename_keys_submit_or_revert_inline_edits() {
    assert_eq!(pane_rename_key("enter"), Some(PaneRenameKey::Submit));
    assert_eq!(pane_rename_key("escape"), Some(PaneRenameKey::Cancel));
    assert_eq!(pane_rename_key("tab"), None);
}

#[test]
fn dropped_files_focus_the_pane_they_land_on() {
    use crate::{AppRoute, NativeRoot};
    use gpui::{ExternalPaths, FileDropEvent, TestAppContext, VisualTestContext};
    use std::sync::Arc;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let target_pane = store.update(&mut cx, |store, cx| {
        crate::app_store::seed_mixed_working_sessions(store);
        let core = store.test_core.clone();
        let events: Arc<dyn runner_daemon::session::manager::SessionEvents> =
            Arc::new(core.session_events());
        for id in ["busy-shell", "direct-agent"] {
            core.sessions
                .prepare_unlisted_terminal(id, (80, 24), &core.db, &events)
                .unwrap();
            store.bridge.attach(id).unwrap();
        }
        let mut layout = PaneLayout::single(Some("busy-shell"), &[]);
        let target_pane = layout.split("p1", SplitOrientation::Row).unwrap();
        layout.assign_session(&target_pane, "direct-agent").unwrap();
        assert!(layout.focus_pane("p1"));
        let conn = core.db.get().unwrap();
        runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
            .unwrap();
        store.replace_nodes(runner_daemon::repo::node::list(&conn).unwrap(), cx);
        target_pane
    });
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "file-drop-test".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.apply_tab_rows(cx);
        assert!(root.tabs.activate_session("busy-shell"));
        root.route = AppRoute::Chat;
        root.sync_active_chat_detail(cx);
        root.ensure_active_tab_attached(window, cx).unwrap();
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1440.), px(900.)));
    visual.run_until_parked();
    host.update(&mut cx, |root, _, _| {
        assert_eq!(root.tabs.active().unwrap().focused_pane_id, "p1");
    })
    .unwrap();

    let target = visual
        .debug_bounds("TERMINAL_FILE_DROP direct-agent")
        .expect("an interactive terminal is a drop target");
    visual.simulate_event(FileDropEvent::Entered {
        position: target.center(),
        paths: ExternalPaths(["/tmp/two words.png"].into_iter().map(Into::into).collect()),
    });
    visual.simulate_event(FileDropEvent::Submit {
        position: target.center(),
    });
    visual.run_until_parked();

    host.update(&mut cx, |root, window, _| {
        assert_eq!(root.tabs.active().unwrap().focused_pane_id, target_pane);
        assert!(root.attached["direct-agent"]
            .terminal_focus
            .is_focused(window));
        // No process backs the test session, so the daemon refuses the
        // write; the refusal names the terminal the paths were sent to.
        assert_eq!(
            root.chat_error,
            Some("session not found: direct-agent".into())
        );
    })
    .unwrap();
}
