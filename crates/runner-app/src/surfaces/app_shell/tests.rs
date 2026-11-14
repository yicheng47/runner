use super::*;

#[test]
fn theme_guard_isolates_unscoped_root_theme_and_material_updates() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{TestAppContext, VisualTestContext};
    use std::sync::{mpsc, Arc, Barrier};
    use std::time::Duration;

    let ready = Arc::new(Barrier::new(2));
    let (updated, updates) = mpsc::channel();
    let guarded = std::thread::spawn({
        let ready = ready.clone();
        move || {
            let _theme = ThemeGuard::new();
            theme::set_active_variant(theme::ThemeVariant::Carbon);
            theme::set_glass(true);
            ready.wait();
            updates.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(
                (theme::active_variant(), theme::is_glass()),
                (theme::ThemeVariant::Carbon, true)
            );
        }
    });
    ready.wait();

    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        store.settings.app_theme = theme::ThemeIntent::Light;
        store.settings.light_app_theme = theme::LightTheme::RunnerLight;
        store.settings.window_material = runner_app::appearance::WindowMaterial::Solid;
    });
    let host = cx.add_window(|window, cx| {
        NativeRoot::new(
            "unscoped-theme-writer".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    let visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    updated.send(()).unwrap();
    guarded.join().unwrap();
}

#[cfg(unix)]
#[test]
fn work_card_insets_every_route_and_contains_chat_and_mission_columns() {
    use crate::theme_snapshot::{assert_fill, ThemeGuard};
    use gpui::{size, TestAppContext, VisualTestContext};
    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, cx| {
        crate::app_store::seed_mixed_working_sessions(store);
        let conn = store.test_core.db.get().unwrap();
        let mut layout = runner_app::pane_layout::PaneLayout::single(
            Some("direct-agent"),
            &["direct-agent".into()],
        );
        let focused = layout.focused_pane_id.clone();
        layout
            .split(&focused, runner_app::pane_layout::SplitOrientation::Row)
            .unwrap();
        let right = layout.focused_pane_id.clone();
        layout
            .split(&right, runner_app::pane_layout::SplitOrientation::Column)
            .unwrap();
        layout.focus_pane(&focused);
        runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
            .unwrap();
        store.replace_nodes(runner_daemon::repo::node::list(&conn).unwrap(), cx);
    });
    for path in [
        "/chats/direct-agent",
        "/missions/test-mission",
        "/roles",
        "/crews",
        "/settings",
    ] {
        let host = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                format!("card-{path}"),
                temp.path().join("logs"),
                Some(path.into()),
                None,
                store.clone(),
                window,
                cx,
            );
            root.set_sidebar_collapsed(false, false, cx);
            root.sidebar_visibility = SidebarVisibilityTransition::new(true);
            root
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1440.), px(900.)));
        visual.run_until_parked();
        let (card_selector, chrome_selector) = if path == "/settings" {
            ("WORK_CARD", "SETTINGS_CHROME")
        } else {
            ("APP_CONTENT_COLUMN", "APP_CHROME")
        };
        let card = visual.debug_bounds(card_selector).unwrap();
        let chrome = visual.debug_bounds(chrome_selector).unwrap();
        if path.starts_with("/chats/") {
            assert_fill(&mut visual, "CHAT_PANEL_HEADER", None);
        } else if path.starts_with("/missions/") {
            assert_fill(&mut visual, "MISSION_HEADER_ROW", None);
            assert_fill(&mut visual, "MISSION_RAIL_HEADER", None);
        }
        if path.starts_with("/chats/") {
            let first = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
            let second = visual.debug_bounds("PANE_ISLAND:p3").unwrap();
            let third = visual.debug_bounds("PANE_ISLAND:p5").unwrap();
            let panel = visual.debug_bounds("CHAT_SIDE_PANEL").unwrap();
            assert_eq!(third.top() - second.bottom(), px(6.));
            assert_eq!(second.left() - first.right(), px(6.));
            assert_eq!(panel.left() - second.right(), px(6.));
        }
        let top_inset = if cfg!(target_os = "linux") { 0. } else { 8. };
        assert_eq!(card.top(), chrome.top() + px(top_inset), "{path}");
        assert_eq!(card.right(), chrome.right() - px(8.), "{path}");
        assert_eq!(card.bottom(), chrome.bottom() - px(8.), "{path}");
        assert_eq!(
            card.left(),
            chrome.left()
                + px(store.read_with(&visual, |store, _| store.settings.sidebar_width))
                + px(
                    if path.starts_with("/chats/") || path.starts_with("/missions/") {
                        6.
                    } else {
                        0.
                    }
                ),
            "{path}"
        );
        for selector in if path.starts_with("/chats/") {
            &["PANE_ISLAND:p1", "CHAT_SIDE_PANEL"][..]
        } else if path.starts_with("/missions/") {
            &["MISSION_HEADER", "MISSION_RAIL"][..]
        } else {
            &[][..]
        } {
            let child = visual.debug_bounds(selector).unwrap();
            assert!(
                child.left() >= card.left() && child.right() <= card.right(),
                "{path}: {selector} {child:?} in {card:?}"
            );
            let inset = if path.starts_with("/chats/") || *selector == "MISSION_RAIL" {
                0.
            } else {
                1.
            };
            assert_eq!(child.top(), card.top() + px(inset), "{path}: {selector}");
            assert!(child.bottom() <= card.bottom(), "{path}: {selector}");
            if matches!(*selector, "CHAT_SIDE_PANEL" | "MISSION_RAIL") {
                assert_eq!(
                    child.bottom(),
                    card.bottom() - px(inset),
                    "{path}: {selector}"
                );
            }
        }
        if path != "/settings" {
            host.update(&mut visual, |root, _, cx| {
                root.set_sidebar_collapsed(true, false, cx);
            })
            .unwrap();
            visual.run_until_parked();
            // Finish the sidebar's bounded transition before checking its settled layout.
            host.update(&mut visual, |root, _, cx| {
                root.sidebar_visibility = SidebarVisibilityTransition::new(false);
                cx.notify();
            })
            .unwrap();
            visual.run_until_parked();
            assert_eq!(
                visual.debug_bounds("APP_CONTENT_COLUMN").unwrap().left(),
                px(8.)
            );
        }
        host.update(&mut visual, |_, window, _| window.remove_window())
            .unwrap();
    }
}

#[test]
fn window_navigation_stays_fixed_when_sidebar_animates() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{size, Modifiers, TestAppContext, VisualTestContext};
    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, cx| {
        crate::app_store::seed_mixed_working_sessions(store);
        let conn = store.test_core.db.get().unwrap();
        let layout = runner_app::pane_layout::PaneLayout::single(
            Some("direct-agent"),
            &["direct-agent".into()],
        );
        runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
            .unwrap();
        store.replace_nodes(runner_daemon::repo::node::list(&conn).unwrap(), cx);
    });
    for path in [
        "/chats/direct-agent",
        "/missions/test-mission",
        "/roles",
        "/crews",
        "/settings",
    ] {
        let host = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                format!("navigation-{path}"),
                temp.path().join("logs"),
                Some(path.into()),
                None,
                store.clone(),
                window,
                cx,
            );
            root.set_sidebar_collapsed(false, false, cx);
            root.sidebar_visibility = SidebarVisibilityTransition::new(true);
            root
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1440.), px(900.)));
        for zoom in [1., 1.5] {
            host.update(&mut visual, |root, window, cx| {
                root.set_zoom(zoom, window, cx)
            })
            .unwrap();
            visual.run_until_parked();
            let selectors = [
                "WINDOW_SIDEBAR_TOGGLE",
                "WINDOW_PREVIOUS_PAGE",
                "WINDOW_NEXT_PAGE",
            ];
            let original = selectors.map(|selector| visual.debug_bounds(selector).unwrap());
            let controls_center = if cfg!(any(windows, target_os = "linux")) {
                16.
            } else {
                30.
            } * zoom;
            for bounds in original {
                assert_eq!(
                    bounds.center().y,
                    px(controls_center),
                    "{path} at {zoom}: {bounds:?}"
                );
                assert_eq!(
                    bounds.size,
                    size(px(28. * zoom), px(28. * zoom)),
                    "{path} at {zoom}"
                );
            }
            if cfg!(any(windows, target_os = "linux")) {
                let caption = visual.debug_bounds("WINDOW_CAPTION_BAR").unwrap();
                let buttons = visual.debug_bounds("WINDOW_CAPTION_CONTROLS").unwrap();
                assert_eq!(caption.size.height, px(32. * zoom));
                assert_eq!(buttons.center().y, original[0].center().y);
                assert!(original[0].top() >= caption.top());
                assert!(original[0].bottom() <= caption.bottom());
                let (card_selector, chrome_selector) = if path == "/settings" {
                    ("WORK_CARD", "SETTINGS_CHROME")
                } else {
                    ("APP_CONTENT_COLUMN", "APP_CHROME")
                };
                assert_eq!(
                    visual.debug_bounds(card_selector).unwrap().top(),
                    visual.debug_bounds(chrome_selector).unwrap().top(),
                    "{path} at {zoom}: card starts below the titlebar"
                );
            }
            let header_selector = if path.starts_with("/chats/") {
                Some("PANE_HEADER:direct-agent")
            } else if path.starts_with("/missions/") {
                Some("MISSION_HEADER_ROW")
            } else {
                None
            };
            if let Some(selector) = header_selector {
                let header = visual.debug_bounds(selector).unwrap();
                assert!(
                    (header.center().y - original[0].center().y
                        + px(if path.starts_with("/chats/") {
                            2. * zoom
                        } else {
                            0.
                        })
                        - px(if cfg!(any(windows, target_os = "linux")) {
                            38. * zoom
                        } else {
                            0.
                        }))
                    .abs()
                        <= px(1.),
                    "{path} at {zoom}: {header:?} vs {:?}",
                    original[0]
                );
            }
            visual.simulate_click(original[0].center(), Modifiers::default());
            visual.run_until_parked();
            host.read_with(&visual, |root, _| {
                assert_eq!(root.sidebar_collapsed, path != "/settings")
            })
            .unwrap();
            for (selector, before) in selectors.into_iter().zip(original) {
                assert_eq!(
                    visual.debug_bounds(selector).unwrap(),
                    before,
                    "{path} at {zoom}: collapse moved {selector}"
                );
            }
            if path != "/settings" {
                host.update(&mut visual, |root, _, cx| {
                    root.sidebar_visibility = SidebarVisibilityTransition::new(false);
                    cx.notify();
                })
                .unwrap();
                visual.run_until_parked();
                for (selector, before) in selectors.into_iter().zip(original) {
                    assert_eq!(
                        visual.debug_bounds(selector).unwrap(),
                        before,
                        "{path} at {zoom}: settled collapse moved {selector}"
                    );
                }
                if cfg!(any(windows, target_os = "linux")) {
                    assert!(visual.debug_bounds("WINDOW_NAVIGATION_SPACE").is_none());
                } else if header_selector.is_some() {
                    let reserved = visual.debug_bounds("WINDOW_NAVIGATION_SPACE").unwrap();
                    assert!(
                        (reserved.left() - original[0].left()).abs() <= px(1.),
                        "{path} at {zoom}: {reserved:?} vs {:?}",
                        original[0]
                    );
                    assert!(
                        reserved.right() >= original[2].right(),
                        "{path} at {zoom}: title overlaps controls"
                    );
                }
                visual.simulate_click(original[0].center(), Modifiers::default());
                visual.run_until_parked();
                host.read_with(&visual, |root, _| assert!(!root.sidebar_collapsed))
                    .unwrap();
                for (selector, before) in selectors.into_iter().zip(original) {
                    assert_eq!(
                        visual.debug_bounds(selector).unwrap(),
                        before,
                        "{path} at {zoom}: expand moved {selector}"
                    );
                }
                host.update(&mut visual, |root, _, cx| {
                    root.sidebar_visibility = SidebarVisibilityTransition::new(true);
                    cx.notify();
                })
                .unwrap();
                visual.run_until_parked();
            }
        }
        host.update(&mut visual, |_, window, _| window.remove_window())
            .unwrap();
    }
}

struct RetryService {
    core: runner_daemon::daemon::InProcessTransport,
    down: std::sync::atomic::AtomicBool,
    error: &'static str,
    fail_once: std::sync::atomic::AtomicBool,
    attempts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl runner_core::protocol::Transport for RetryService {
    fn call(
        &self,
        request: runner_core::protocol::Request,
    ) -> Result<runner_core::protocol::Response, runner_core::protocol::ClientError> {
        if self.down.load(std::sync::atomic::Ordering::Acquire) {
            return Err(runner_core::protocol::ClientError::msg(self.error));
        }
        self.core.call(request)
    }
    fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
        self.core.subscribe()
    }
    fn reconnect(&self) -> Result<(), runner_core::protocol::ClientError> {
        use std::sync::atomic::Ordering;
        self.attempts.fetch_add(1, Ordering::AcqRel);
        if self.fail_once.swap(false, Ordering::AcqRel) {
            return Err(runner_core::protocol::ClientError::msg("test start failed"));
        }
        self.down.store(false, Ordering::Release);
        self.core
            .0
            .events
            .emit("daemon/reconnected", &serde_json::Value::Null);
        Ok(())
    }
}

#[test]
fn capped_banner_survives_failed_requests_and_try_again_until_reconnect() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let events = store.read_with(&cx, |store, _| store.test_core.events.clone());
    let window = cx.add_window(|window, cx| {
        NativeRoot::new(
            "retry-notice".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    cx.run_until_parked();
    let attempts = Arc::new(AtomicUsize::new(0));
    store.update(&mut cx, |store, _| {
        store.client = runner_core::protocol::DaemonClient::new(Arc::new(RetryService {
            core: runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
            down: AtomicBool::new(true),
            error: "runnerd connection closed",
            fail_once: AtomicBool::new(true),
            attempts: attempts.clone(),
        }));
        store.live_session_count = 6;
        store.stopped_session_count = 6;
    });
    events.emit(
        "daemon/disconnected",
        &serde_json::json!({"restart_limit":true}),
    );
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    store.read_with(&cx, |store, _| {
        assert_eq!(store.live_session_count, 0);
        assert_eq!(store.stopped_session_count, 6);
    });
    let notice = visual.debug_bounds("DAEMON_CRASH_BANNER").unwrap();
    visual.simulate_click(
        point(notice.left() + px(2.), notice.top() + px(2.)),
        Modifiers::default(),
    );
    cx.run_until_parked();
    assert!(visual.debug_bounds("GLOBAL_TOAST").is_none());
    window
        .update(&mut cx, |root, _, cx| {
            let error = root.core(cx).role_list().unwrap_err().to_string();
            root.error = Some(error.clone().into());
            root.chat_error = Some(error.into());
            root.show_toast("Saved", ToastTone::Success, cx);
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    window
        .read_with(&cx, |root, _| assert!(root.toasts.active().is_none()))
        .unwrap();
    for attempt in 1..=2 {
        assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_some());
        assert!(visual.debug_bounds("DAEMON_CRASH_MESSAGE").is_some());
        assert!(visual.debug_bounds("DAEMON_OPEN_LOG").is_some());
        assert!(visual.debug_bounds("GLOBAL_TOAST").is_none());
        let retry = visual.debug_bounds("DAEMON_TRY_AGAIN").unwrap();
        visual.simulate_click(retry.center(), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(attempts.load(Ordering::Acquire), attempt);
    }
    assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
    store.read_with(&cx, |store, _| {
        assert!(!store.daemon_disconnected && store.daemon_notice.is_none())
    });
    window
        .read_with(&cx, |root, _| {
            assert!(root.chat_error.is_none() && !root.daemon_retrying)
        })
        .unwrap();
}

#[test]
fn failed_automatic_restart_offers_persistent_banner_until_reconnect() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let events = store.read_with(&cx, |store, _| store.test_core.events.clone());
    let window = cx.add_window(|window, cx| {
        NativeRoot::new(
            "failed-automatic-restart".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    cx.run_until_parked();
    let attempts = Arc::new(AtomicUsize::new(0));
    store.update(&mut cx, |store, _| {
        store.client = runner_core::protocol::DaemonClient::new(Arc::new(RetryService {
            core: runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
            down: AtomicBool::new(true),
            error: "runnerd connection closed",
            fail_once: AtomicBool::new(true),
            attempts: attempts.clone(),
        }));
    });
    events.emit("daemon/disconnected", &serde_json::Value::Null);
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
    events.emit(
        "daemon/disconnected",
        &serde_json::json!({"restart_limit":true}),
    );
    cx.run_until_parked();
    store.update(&mut cx, |store, cx| {
        assert!(store.client.role_list().is_err());
        store.refresh_sessions(cx);
    });
    cx.run_until_parked();
    for attempt in 1..=2 {
        assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_some());
        assert!(visual.debug_bounds("DAEMON_OPEN_LOG").is_some());
        assert!(visual.debug_bounds("GLOBAL_TOAST").is_none());
        let retry = visual.debug_bounds("DAEMON_TRY_AGAIN").unwrap();
        visual.simulate_click(retry.center(), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(attempts.load(Ordering::Acquire), attempt);
    }
    assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
    assert!(visual.debug_bounds("GLOBAL_TOAST").is_none());
    store.read_with(&cx, |store, _| {
        assert!(!store.daemon_disconnected);
        assert!(store.daemon_notice.is_none() && store.error.is_none());
    });
}

#[test]
fn capped_service_has_one_notice_on_cached_and_unloaded_entity_lists() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let events = store.read_with(&cx, |store, _| store.test_core.events.clone());
    let mut windows = Vec::new();
    let attempts = Arc::new(AtomicUsize::new(0));
    for cached in [true, false] {
        if !cached {
            store.update(&mut cx, |store, _| {
                store.client = runner_core::protocol::DaemonClient::new(Arc::new(RetryService {
                    core: runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
                    down: AtomicBool::new(true),
                    error: runner_core::protocol::managed::RESTART_LIMIT_NOTICE,
                    fail_once: AtomicBool::new(true),
                    attempts: attempts.clone(),
                }));
            });
            events.emit(
                "daemon/disconnected",
                &serde_json::json!({"restart_limit":true}),
            );
            cx.run_until_parked();
        }
        for path in ["/roles", "/crews"] {
            windows.push(cx.add_window(|window, cx| {
                NativeRoot::new(
                    format!("list-notice-{cached}-{path}").replace('/', ""),
                    temp.path().join("logs"),
                    Some(path.into()),
                    None,
                    store.clone(),
                    window,
                    cx,
                )
            }));
        }
        cx.run_until_parked();
    }
    for window in &windows {
        window
            .update(&mut cx, |root, _, cx| {
                if root.route == AppRoute::Roles {
                    root.load_role_page(cx);
                } else {
                    root.load_crew_page(cx);
                }
            })
            .unwrap();
    }
    cx.run_until_parked();
    let mut visuals = windows
        .iter()
        .map(|window| VisualTestContext::from_window((*window).into(), &cx))
        .collect::<Vec<_>>();
    for attempt in 1..=2 {
        for visual in &mut visuals {
            assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_some());
            assert!(visual.debug_bounds("DAEMON_OPEN_LOG").is_some());
            assert!(visual.debug_bounds("PAGINATED_LIST_ERROR").is_none());
            assert!(visual.debug_bounds("PAGINATED_LIST_LOAD_ERROR").is_none());
            assert!(visual.debug_bounds("GLOBAL_TOAST").is_none());
        }
        let retry = visuals[0].debug_bounds("DAEMON_TRY_AGAIN").unwrap();
        visuals[0].simulate_click(retry.center(), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(attempts.load(Ordering::Acquire), attempt);
    }
    for visual in &mut visuals {
        assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
        assert!(visual.debug_bounds("PAGINATED_LIST_ERROR").is_none());
        assert!(visual.debug_bounds("PAGINATED_LIST_LOAD_ERROR").is_none());
    }
    store.update(&mut cx, |store, _| {
        store.client = runner_core::protocol::DaemonClient::new(Arc::new(RetryService {
            core: runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
            down: AtomicBool::new(true),
            error: "Example page error",
            fail_once: AtomicBool::new(false),
            attempts,
        }));
    });
    for window in &windows {
        window
            .update(&mut cx, |root, _, cx| {
                if root.route == AppRoute::Roles {
                    root.load_role_page(cx);
                } else {
                    root.load_crew_page(cx);
                }
            })
            .unwrap();
    }
    cx.run_until_parked();
    for visual in &mut visuals {
        let banner = visual.debug_bounds("PAGINATED_LIST_ERROR").unwrap();
        let content = visual.debug_bounds("APP_CONTENT_COLUMN").unwrap();
        assert_eq!(banner.size.height, px(41.));
        assert_eq!(banner.left(), content.left() + px(1.));
        assert_eq!(banner.right(), content.right() - px(1.));
        assert_eq!(banner.top(), content.top() + px(45.));
        assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
    }
}

#[test]
fn capped_banner_stays_in_each_windows_main_content() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{size, TestAppContext, VisualTestContext};
    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, cx| {
        crate::app_store::seed_mixed_working_sessions(store);
        let conn = store.test_core.db.get().unwrap();
        runner_daemon::repo::node::create_tab(
            &conn,
            None,
            "",
            0,
            &runner_app::pane_layout::PaneLayout::single(
                Some("direct-agent"),
                &["direct-agent".into()],
            )
            .serialize()
            .unwrap(),
        )
        .unwrap();
        let nodes = runner_daemon::repo::node::list(&conn).unwrap();
        store.replace_nodes(nodes, cx);
    });
    let events = store.read_with(&cx, |store, _| store.test_core.events.clone());
    let chat = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "main".into(),
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
    let other = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "banner-other".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = AppRoute::Roles;
        root
    });
    let mut chat_visual = VisualTestContext::from_window(chat.into(), &cx);
    let mut other_visual = VisualTestContext::from_window(other.into(), &cx);
    chat_visual.simulate_resize(size(px(1440.), px(900.)));
    other_visual.simulate_resize(size(px(1440.), px(900.)));
    events.emit(
        "daemon/disconnected",
        &serde_json::json!({"restart_limit":true}),
    );
    cx.run_until_parked();
    let banner = chat_visual.debug_bounds("DAEMON_CRASH_BANNER").unwrap();
    let area = chat_visual.debug_bounds("APP_CONTENT_COLUMN").unwrap();
    let sidebar = chat_visual.debug_bounds("APP_SIDEBAR").unwrap();
    let panel = chat_visual.debug_bounds("CHAT_SIDE_PANEL").unwrap();
    assert_eq!(banner.top(), area.top());
    assert_eq!(banner.left(), sidebar.right() + px(6.));
    assert_eq!(banner.right(), panel.left() - px(6.));
    assert_eq!(banner.size.height, px(41.));
    let text = chat_visual.debug_bounds("DAEMON_CRASH_MESSAGE").unwrap();
    let log = chat_visual.debug_bounds("DAEMON_OPEN_LOG").unwrap();
    let retry = chat_visual.debug_bounds("DAEMON_TRY_AGAIN").unwrap();
    assert!(text.size.height < banner.size.height);
    assert!(text.right() <= log.left() && log.right() <= retry.left());
    assert!(retry.right() <= banner.right());
    assert!(chat_visual.debug_bounds("NOTICE_BANNER_ICON").is_some());
    assert!(chat_visual.debug_bounds("GLOBAL_TOAST").is_none());
    let content = other_visual.debug_bounds("APP_CONTENT_COLUMN").unwrap();
    let other_banner = other_visual.debug_bounds("DAEMON_CRASH_BANNER").unwrap();
    assert_eq!(other_banner.left(), content.left() + px(1.));
    assert_eq!(other_banner.right(), content.right() - px(1.));
    for route in [
        AppRoute::Mission("test-mission".into()),
        AppRoute::Settings,
        AppRoute::ArchivedChat,
    ] {
        other
            .update(&mut cx, |root, _, cx| {
                root.route = route.clone();
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let banner = other_visual.debug_bounds("DAEMON_CRASH_BANNER").unwrap();
        if matches!(route, AppRoute::Mission(_)) {
            let center = other_visual.debug_bounds("MISSION_CONTENT_COLUMN").unwrap();
            let header = other_visual.debug_bounds("MISSION_HEADER").unwrap();
            assert_eq!(banner.top(), header.bottom());
            assert_eq!(banner.left(), center.left());
            assert_eq!(banner.right(), center.right());
        } else {
            assert_eq!(banner.left(), content.left() + px(1.));
            assert_eq!(banner.right(), content.right() - px(1.));
            if route == AppRoute::Settings {
                let track = other_visual
                    .debug_bounds("SETTINGS_CONTENT_SCROLLBAR_TRACK")
                    .unwrap();
                assert!(track.top() >= banner.bottom());
                assert!(track.bottom() <= px(900.));
            }
        }
    }
    events.emit("daemon/reconnected", &serde_json::Value::Null);
    cx.run_until_parked();
    assert!(chat_visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
    assert!(other_visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
}

#[test]
fn automatic_crash_recovery_shows_a_short_error_toast_without_actions() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{TestAppContext, VisualTestContext};
    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let events = store.read_with(&cx, |store, _| store.test_core.events.clone());
    let window = cx.add_window(|window, cx| {
        NativeRoot::new(
            "crash-toast".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    cx.run_until_parked();
    store.update(&mut cx, |store, _| {
        store.live_session_count = 6;
        store.stopped_session_count = 6;
    });
    events.emit("daemon/disconnected", &serde_json::Value::Null);
    events.emit("daemon/reconnected", &serde_json::Value::Null);
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    assert!(visual.debug_bounds("GLOBAL_TOAST").is_some());
    assert!(
        visual
            .debug_bounds("GLOBAL_TOAST_MESSAGE")
            .unwrap()
            .size
            .height
            <= px(22.)
    );
    assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
    assert!(visual.debug_bounds("DAEMON_OPEN_LOG").is_none());
    assert!(visual.debug_bounds("DAEMON_TRY_AGAIN").is_none());
    window
        .read_with(&cx, |root, _| {
            let toast = root.toasts.active().unwrap();
            assert_eq!(
                toast.message,
                "Background service restarted · 6 sessions stopped"
            );
            assert_eq!(toast.tone, ToastTone::Error);
            assert_eq!(
                toast.duration_ms,
                Some(crate::toast::DEFAULT_TOAST_DURATION_MS)
            );
        })
        .unwrap();
}

fn full_weekly_usage_snapshot() -> UsageSnapshot {
    UsageSnapshot {
        runtimes: [Runtime::Codex, Runtime::ClaudeCode]
            .into_iter()
            .map(|runtime| {
                (
                    runtime,
                    runner_core::protocol::usage::RuntimeUsage {
                        value: Some(AgentUsage {
                            credits: None,
                            windows: vec![UsageWindow {
                                name: crate::runtime_ui::runtime_ui(runtime).usage_week.into(),
                                used_percent: 100.,
                                resets_at: None,
                            }],
                            updated_at: chrono::Utc::now(),
                        }),
                        ..Default::default()
                    },
                )
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn sidebar_footer_fits_weekly_values_and_settings_in_one_row() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{size, TestAppContext, VisualTestContext};

    let _theme = ThemeGuard::new();
    for variant in [
        theme::ThemeVariant::Carbon,
        theme::ThemeVariant::RunnerLight,
    ] {
        for width in [
            app_settings::SIDEBAR_MIN,
            app_settings::SIDEBAR_DEFAULT,
            app_settings::SIDEBAR_MAX,
        ] {
            for zoom in [0.6, 1., 1.5, 2.] {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                store.update(&mut cx, |store, _| {
                    store.settings.app_theme = if variant.is_light() {
                        theme::ThemeIntent::Light
                    } else {
                        theme::ThemeIntent::Dark
                    };
                    store.settings.light_app_theme = theme::LightTheme::RunnerLight;
                    store.settings.dark_app_theme = theme::DarkTheme::Runner;
                    store.settings.sidebar_width = width;
                    store.settings.app_zoom = zoom;
                    store.usage = full_weekly_usage_snapshot();
                });
                let host = cx.add_window(|window, cx| {
                    let mut root = NativeRoot::new(
                        "sidebar-footer".into(),
                        temp.path().join("logs"),
                        None,
                        None,
                        store.clone(),
                        window,
                        cx,
                    );
                    root.usage_installed = vec![Runtime::Codex, Runtime::ClaudeCode];
                    root
                });
                let mut visual = VisualTestContext::from_window(host.into(), &cx);
                visual.simulate_resize(size(px(1200.), px(900.)));
                visual.run_until_parked();
                assert_eq!(theme::active_variant(), variant);

                let footer = visual.debug_bounds("SIDEBAR_FOOTER").unwrap();
                let usage = visual.debug_bounds("SIDEBAR_USAGE").unwrap();
                crate::theme_snapshot::assert_fill_color(&mut visual, "SIDEBAR_USAGE", None);
                crate::theme_snapshot::assert_border(
                    &mut visual,
                    "SIDEBAR_USAGE",
                    gpui::transparent_black(),
                );
                let gear = visual.debug_bounds("SIDEBAR_SETTINGS").unwrap();
                let host_button = visual.debug_bounds("SIDEBAR_HOST_BUTTON").unwrap();
                assert!((gear.left() - host_button.right() - px(8. * zoom)).abs() < px(1.));
                let row = visual.debug_bounds("SIDEBAR_FOOTER_ROW").unwrap();
                assert!((usage.size.height - px(32. * zoom)).abs() <= px(1.));
                assert!((gear.size.width - px(32. * zoom)).abs() <= px(1.));
                assert_eq!(usage.size.height, gear.size.height);
                assert_eq!(usage.top(), gear.top());
                assert!(usage.top() >= row.top() && usage.bottom() <= row.bottom());
                assert!(gear.top() >= row.top() && gear.bottom() <= row.bottom());
                assert!(usage.left() >= footer.left());
                assert!(usage.right() <= gear.left());
                assert!(gear.right() <= footer.right());
                assert!(usage.size.width <= px(176. * zoom + 1.));
                if width >= app_settings::SIDEBAR_DEFAULT {
                    assert!(gear.left() - usage.right() >= px(8. * zoom - 1.));
                }
                let entries = [
                    "SIDEBAR_USAGE_ENTRY_codex",
                    "SIDEBAR_USAGE_ENTRY_claude-code",
                ]
                .map(|id| visual.debug_bounds(id).unwrap());
                assert!(
                    (entries[0].left() - usage.left() - px(6. * zoom + 1.)).abs() <= px(0.5),
                    "{width} {zoom}: {usage:?} {entries:?}"
                );
                assert!(
                    (entries[1].left() - entries[0].right() - px(8. * zoom)).abs() <= px(0.5),
                    "{width} {zoom}: {entries:?}"
                );
                for pair in entries.windows(2) {
                    assert!(
                        pair[0].right() <= pair[1].left(),
                        "{width} {zoom}: {pair:?}"
                    );
                }
                let last = visual
                    .debug_bounds("SIDEBAR_USAGE_VALUE_claude-code")
                    .unwrap();
                assert!(
                    last.right() <= usage.right() - px(6. * zoom - 0.5),
                    "{width} {zoom}: {usage:?} {last:?}"
                );
                visual.simulate_mouse_move(usage.center(), None, gpui::Modifiers::default());
                visual.run_until_parked();
                assert_eq!(visual.debug_bounds("SIDEBAR_USAGE"), Some(usage));
                assert_eq!(visual.debug_bounds("SIDEBAR_SETTINGS"), Some(gear));
                visual.simulate_mouse_move(gear.center(), None, gpui::Modifiers::default());
                visual.run_until_parked();
                assert_eq!(visual.debug_bounds("SIDEBAR_USAGE"), Some(usage));
                assert_eq!(visual.debug_bounds("SIDEBAR_SETTINGS"), Some(gear));
            }
        }
    }
}

#[test]
fn sidebar_footer_opens_usage_and_settings_and_hides_usage_in_collapsed_preview() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{size, Modifiers, TestAppContext, VisualTestContext};

    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "sidebar-footer-actions".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.usage_installed = vec![Runtime::Codex];
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    visual.run_until_parked();
    let usage = visual.debug_bounds("SIDEBAR_USAGE").unwrap();
    visual.simulate_click(usage.center(), Modifiers::default());
    visual.run_until_parked();
    assert!(host.read_with(&visual, |root, _| root.usage_open).unwrap());
    assert!(visual.debug_bounds("USAGE_POPOVER_PANEL").is_some());
    visual.simulate_click(usage.center(), Modifiers::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("USAGE_POPOVER_PANEL").is_none());
    let gear = visual.debug_bounds("SIDEBAR_SETTINGS").unwrap();
    visual.simulate_click(gear.center(), Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        host.read_with(&visual, |root, _| root.route.clone())
            .unwrap(),
        AppRoute::Settings
    );
    host.update(&mut visual, |root, _, cx| {
        root.route = AppRoute::Roles;
        root.sidebar_collapsed = true;
        root.sidebar_preview_open = true;
        cx.notify();
    })
    .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SIDEBAR_USAGE").is_none());
    assert!(visual.debug_bounds("SIDEBAR_SETTINGS").is_some());
    host.update(&mut visual, |root, _, cx| {
        root.sidebar_collapsed = false;
        root.sidebar_preview_open = false;
        root.usage_installed.clear();
        cx.notify();
    })
    .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SIDEBAR_USAGE").is_none());
    assert!(visual.debug_bounds("SIDEBAR_SETTINGS").is_some());
}

struct SidebarHeaderProbe {
    root: Entity<NativeRoot>,
    width: f32,
    zoom: f32,
    available: bool,
}

impl Render for SidebarHeaderProbe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(px(16. * self.zoom));
        div()
            .w(px(self.width * self.zoom))
            .child(self.root.update(cx, |root, cx| {
                let update = self
                    .available
                    .then(|| runner_app::updater::UpdateState::Available {
                        info: runner_app::updater::UpdateInfo::new("0.99.0"),
                        size: 0,
                        manual_only: false,
                    });
                root.render_sidebar_brand(update, window, cx)
            }))
    }
}

#[test]
fn remote_sidebar_hides_an_available_local_update() {
    use crate::surfaces::remote_client::tests::{choose_host, enable_front_page, fixture};
    use gpui::{TestAppContext, VisualTestContext};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let host = cx.add_window(|window, cx| SidebarHeaderProbe {
        root: cx.new(|cx| {
            NativeRoot::new(
                "remote-update".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            )
        }),
        width: app_settings::SIDEBAR_DEFAULT,
        zoom: 1.,
        available: true,
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.run_until_parked();
    assert!(visual.debug_bounds("SIDEBAR_UPDATE").is_some());
    host.update(&mut visual, |probe, window, cx| {
        probe
            .root
            .update(cx, |root, cx| choose_host(root, window, cx));
    })
    .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SIDEBAR_UPDATE").is_none());
    assert!(visual.debug_bounds("SIDEBAR_BRAND_MARK").is_some());
}

#[test]
fn available_app_update_sits_right_after_runner_in_the_sidebar_header() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{size, TestAppContext, VisualTestContext};

    let _theme = ThemeGuard::new();
    for width in [app_settings::SIDEBAR_MIN, app_settings::SIDEBAR_DEFAULT] {
        for zoom in [1., 1.5] {
            for available in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                store.update(&mut cx, |store, _| store.settings.app_zoom = zoom);
                let host = cx.add_window(|window, cx| SidebarHeaderProbe {
                    root: cx.new(|cx| {
                        NativeRoot::new(
                            "sidebar-update".into(),
                            temp.path().join("logs"),
                            None,
                            None,
                            store.clone(),
                            window,
                            cx,
                        )
                    }),
                    width,
                    zoom,
                    available,
                });
                let mut visual = VisualTestContext::from_window(host.into(), &cx);
                visual.simulate_resize(size(px(800.), px(300.)));
                visual.run_until_parked();
                let mark = visual.debug_bounds("SIDEBAR_BRAND_MARK").unwrap();
                assert_eq!(mark.size, size(px(20. * zoom), px(20. * zoom)));
                let update = visual.debug_bounds("SIDEBAR_UPDATE");
                assert_eq!(update.is_some(), available);
                if let Some(update) = update {
                    let header = visual.debug_bounds("SIDEBAR_HEADER").unwrap();
                    let name = visual.debug_bounds("SIDEBAR_NAME").unwrap();
                    assert_eq!(update.left() - name.right(), px(8. * zoom));
                    assert!(update.top() >= header.top());
                    assert!(update.bottom() <= header.bottom());
                    assert!(update.right() <= header.right());
                }
            }
        }
    }
}

struct UsagePopoverProbe {
    zoom: f32,
}

impl Render for UsagePopoverProbe {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(px(16. * self.zoom));
        let now = chrono::Utc::now();
        let sections = [(Runtime::ClaudeCode, 4), (Runtime::Codex, 4)]
            .into_iter()
            .map(|(runtime, count)| {
                let usage = AgentUsage {
                    credits: None,
                    windows: (0..count)
                        .map(|index| UsageWindow {
                            name: format!("Window {index}"),
                            used_percent: 25.,
                            resets_at: Some(now),
                        })
                        .collect(),
                    updated_at: now,
                };
                usage_section(runtime, Some(&usage), None, false, now)
            })
            .collect();
        div().w(px(340. * self.zoom)).child(usage_popover_panel(
            window.bounds().size.height - px(16.),
            div()
                .h(px(24.))
                .debug_selector(|| "USAGE_POPOVER_HEADER".into())
                .child("Usage")
                .into_any_element(),
            sections,
        ))
    }
}

#[test]
fn usage_popover_scrolls_all_provider_rows_in_short_window() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{size, ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext};

    let _theme = ThemeGuard::new();
    theme::set_active_variant(theme::ThemeVariant::Carbon);
    for zoom in [1., 1.5] {
        let mut cx = TestAppContext::single();
        let host = cx.add_window(move |_, _| UsagePopoverProbe { zoom });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(800.), px(600.)));
        visual.run_until_parked();

        if zoom == 1. {
            let panel = visual.debug_bounds("USAGE_POPOVER_PANEL").unwrap();
            assert!(panel.size.height < px(500.), "{panel:?}");
        }
        visual.simulate_resize(size(px(800.), px(300.)));
        visual.run_until_parked();

        let panel = visual.debug_bounds("USAGE_POPOVER_PANEL").unwrap();
        let body = visual.debug_bounds("USAGE_POPOVER_BODY").unwrap();
        let header = visual.debug_bounds("USAGE_POPOVER_HEADER").unwrap();
        let last = visual.debug_bounds("USAGE_SECTION_codex").unwrap();
        assert!(panel.size.height <= px(284.), "{zoom}: {panel:?}");
        assert!(header.bottom() <= body.top(), "{zoom}: {header:?} {body:?}");
        assert!(last.bottom() > body.bottom(), "{zoom}: {last:?} {body:?}");

        visual.simulate_event(ScrollWheelEvent {
            position: body.center(),
            delta: ScrollDelta::Lines(point(0., -100.)),
            ..Default::default()
        });
        visual.run_until_parked();
        let scrolled = visual.debug_bounds("USAGE_SECTION_codex").unwrap();
        assert!(scrolled.top() < last.top(), "{zoom}: {last:?} {scrolled:?}");
        assert!(
            scrolled.bottom() <= body.bottom(),
            "{zoom}: {scrolled:?} {body:?}"
        );
    }
}

#[test]
fn update_dot_counts_only_enabled_agents() {
    let mut settings = app_settings::AppSettings::default();
    assert!(!update_dot_visible(&[], &settings));
    assert!(update_dot_visible(&[Runtime::Codex], &settings));
    settings.disabled_agents.insert(Runtime::Codex.key().into());
    assert!(!update_dot_visible(&[Runtime::Codex], &settings));
    assert!(update_dot_visible(
        &[Runtime::Codex, Runtime::ClaudeCode],
        &settings
    ));
    let copilot_default =
        runner_core::protocol::runtime_metadata::runtime_default_enabled(Runtime::Copilot);
    assert_eq!(
        update_dot_visible(&[Runtime::Copilot], &settings),
        copilot_default
    );
    settings
        .enabled_agents
        .insert(Runtime::Copilot.key().into());
    assert!(update_dot_visible(&[Runtime::Copilot], &settings));
}

#[test]
fn usage_thresholds_and_reset_labels() {
    use chrono::TimeDelta;
    assert_eq!(usage_tone([79.9]), UsageTone::Normal);
    assert_eq!(usage_tone([5., 80.]), UsageTone::Warning);
    assert_eq!(usage_tone([80., 100.]), UsageTone::Danger);
    let now = chrono::Utc::now();
    assert_eq!(reset_label(None, now), "reset unknown");
    assert_eq!(
        reset_label(Some(now + TimeDelta::minutes(190)), now),
        "resets 3h 10m"
    );
    assert_eq!(
        reset_label(Some(now + TimeDelta::hours(164)), now),
        "resets 6d 20h"
    );
    assert_eq!(
        reset_label(Some(now + TimeDelta::seconds(30)), now),
        "resets <1m"
    );
}

#[test]
fn sidebar_usage_uses_weekly_windows() {
    let snapshot = UsageSnapshot {
        runtimes: [(
            Runtime::ClaudeCode,
            runner_core::protocol::usage::RuntimeUsage {
                value: Some(AgentUsage {
                    credits: None,
                    windows: vec![
                        UsageWindow {
                            name: "5 hours".into(),
                            used_percent: 91.,
                            resets_at: None,
                        },
                        UsageWindow {
                            name: "Week".into(),
                            used_percent: 26.,
                            resets_at: None,
                        },
                    ],
                    updated_at: chrono::Utc::now(),
                }),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
        ..UsageSnapshot::default()
    };
    assert_eq!(
        weekly_usage_percent(Runtime::ClaudeCode, snapshot.get(Runtime::ClaudeCode)),
        Some(26.)
    );
    let antigravity_usage = AgentUsage {
        credits: None,
        windows: vec![
            UsageWindow {
                name: "Gemini Models · 5 hours used".into(),
                used_percent: 95.,
                resets_at: None,
            },
            UsageWindow {
                name: "Gemini Models · Week used".into(),
                used_percent: 42.,
                resets_at: None,
            },
            UsageWindow {
                name: "Claude and GPT models · 5 hours used".into(),
                used_percent: 18.,
                resets_at: None,
            },
            UsageWindow {
                name: "Claude and GPT models · Week used".into(),
                used_percent: 73.,
                resets_at: None,
            },
        ],
        updated_at: chrono::Utc::now(),
    };
    assert_eq!(
        weekly_usage_percent(Runtime::Antigravity, Some(&antigravity_usage)),
        Some(73.)
    );
    let antigravity_five_hour_only = AgentUsage {
        credits: None,
        windows: vec![UsageWindow {
            name: "Gemini Models · 5 hours used".into(),
            used_percent: 91.,
            resets_at: None,
        }],
        updated_at: chrono::Utc::now(),
    };
    assert_eq!(
        weekly_usage_percent(Runtime::Antigravity, Some(&antigravity_five_hour_only)),
        None
    );
    let antigravity_claude_only = AgentUsage {
        credits: None,
        windows: vec![
            UsageWindow {
                name: "Claude and GPT models · 5 hours used".into(),
                used_percent: 18.,
                resets_at: None,
            },
            UsageWindow {
                name: "Claude and GPT models · Week used".into(),
                used_percent: 73.,
                resets_at: None,
            },
        ],
        updated_at: chrono::Utc::now(),
    };
    assert_eq!(
        weekly_usage_percent(Runtime::Antigravity, Some(&antigravity_claude_only)),
        Some(73.)
    );
    for window_name in [
        "5 hours",
        "5h",
        "300 min",
        "Gemini Models · 5 hours used",
        "Claude and GPT models · 5 hours used",
    ] {
        assert!(is_five_hour(window_name), "{window_name}");
    }
    for window_name in [
        "Week",
        "Opus · week",
        "Gemini Models · Week used",
        "Claude and GPT models · Week used",
    ] {
        assert!(!is_five_hour(window_name), "{window_name}");
    }
    assert_eq!(weekly_usage_percent(Runtime::Codex, None), None);
    let muse_usage = AgentUsage {
        credits: None,
        windows: vec![UsageWindow {
            name: "Week".into(),
            used_percent: 43.,
            resets_at: None,
        }],
        updated_at: chrono::Utc::now(),
    };
    assert_eq!(weekly_usage_percent(Runtime::Muse, Some(&muse_usage)), Some(43.));
    assert_eq!(unavailable_line(None, Runtime::Codex), "Checking…");
    assert_eq!(
        unavailable_line(Some(UnavailableReason::SignIn), Runtime::Muse),
        "Sign in to Muse to see usage."
    );
    assert_eq!(
        unavailable_line(Some(UnavailableReason::SignIn), Runtime::Antigravity),
        "Sign in to Antigravity CLI to see usage"
    );
    assert_eq!(
        unavailable_line(
            Some(UnavailableReason::AntigravityNoAnswer),
            Runtime::Antigravity
        ),
        "Antigravity CLI didn't answer."
    );
    #[cfg(debug_assertions)]
    crate::i18n::testing::with_locale(crate::i18n::Locale::SimplifiedChinese, || {
        assert_eq!(
            weekly_usage_percent(Runtime::ClaudeCode, snapshot.get(Runtime::ClaudeCode)),
            Some(26.)
        );
        assert_eq!(
            weekly_usage_percent(Runtime::Codex, snapshot.get(Runtime::ClaudeCode)),
            Some(26.)
        );
        assert_eq!(
            weekly_usage_percent(Runtime::Antigravity, Some(&antigravity_usage)),
            Some(73.)
        );
        assert_eq!(usage_window_name("Week"), "本周");
        assert_eq!(usage_window_name("5 hours"), "5 小时");
        assert_eq!(usage_window_name("Opus · week"), "Opus · 本周");
        assert_eq!(usage_window_name("60 min"), "60 分钟");
        assert_eq!(
            usage_window_name("Gemini Models · Week used"),
            "Gemini 模型 · 本周已用"
        );
        assert_eq!(
            usage_window_name("Claude and GPT models · 5 hours used"),
            "Claude 和 GPT 模型 · 5 小时已用"
        );
        assert_eq!(
            usage_window_name("Custom model · custom window"),
            "Custom model · custom window"
        );
    });
}

#[test]
fn usage_runtimes_are_installed_enabled_and_usage_capable_in_runtime_order() {
    let all = [Runtime::Antigravity, Runtime::ClaudeCode, Runtime::Codex];
    // An installed, enabled runtime shows without ever having run a session.
    assert_eq!(
        visible_usage_runtimes(&all, &all),
        [Runtime::Codex, Runtime::ClaudeCode, Runtime::Antigravity]
    );
    assert_eq!(
        visible_usage_runtimes(&[Runtime::ClaudeCode], &all),
        [Runtime::ClaudeCode]
    );
    // Uninstalled and disabled runtimes do not.
    assert_eq!(
        visible_usage_runtimes(&[Runtime::Codex, Runtime::Antigravity], &all),
        [Runtime::Codex, Runtime::Antigravity]
    );
    assert_eq!(
        visible_usage_runtimes(&all, &[Runtime::ClaudeCode, Runtime::Codex]),
        [Runtime::Codex, Runtime::ClaudeCode]
    );
    // A runtime without usage never shows, even if installed and enabled.
    let with_copilot = [Runtime::Copilot, Runtime::Pi, Runtime::Codex];
    assert_eq!(
        visible_usage_runtimes(&with_copilot, &with_copilot),
        [Runtime::Codex]
    );
    assert!(visible_usage_runtimes(&[], &all).is_empty());
    assert!(visible_usage_runtimes(&all, &[]).is_empty());
    assert!(visible_usage_runtimes(&[Runtime::Copilot], &[Runtime::Copilot]).is_empty());
}

#[test]
fn usage_popover_keeps_provider_order_for_installed_and_enabled_subsets() {
    let order = [
        Runtime::ClaudeCode,
        Runtime::Codex,
        Runtime::Antigravity,
        Runtime::Muse,
    ];
    assert_eq!(usage_popover_runtimes(&Runtime::ALL, &Runtime::ALL), order);
    for installed_mask in 0..8 {
        for enabled_mask in 0..8 {
            let subset = |mask| {
                order
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, runtime)| (mask & (1 << index) != 0).then_some(runtime))
                    .rev()
                    .collect::<Vec<_>>()
            };
            let installed = subset(installed_mask);
            let enabled = subset(enabled_mask);
            let expected: Vec<_> = order
                .into_iter()
                .filter(|runtime| installed.contains(runtime) && enabled.contains(runtime))
                .collect();
            assert_eq!(
                usage_popover_runtimes(&installed, &enabled),
                expected,
                "installed: {installed:?}, enabled: {enabled:?}"
            );
        }
    }
}

#[test]
fn sidebar_resize_strip_centres_on_the_island_border_at_each_zoom() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{px, size, TestAppContext, VisualTestContext};
    use runner_daemon::{db, session, shell_path};
    use std::sync::{Arc, RwLock};

    let _theme = ThemeGuard::new();
    theme::set_active_variant(theme::ThemeVariant::Carbon);
    let temp = tempfile::tempdir().unwrap();
    let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
    let runtime_discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
    let core = crate::test_support::core(
        Arc::new(db::open_fixture_pool(&temp.path().join("runner.db")).unwrap()),
        temp.path().to_owned(),
        session::SessionManager::new(
            runtime_shell_env.clone(),
            runtime_discovery.clone(),
            Arc::new(session::pty_runtime::PtyRuntime::new()),
        ),
        runtime_shell_env,
        runtime_discovery,
    );
    let mut cx = TestAppContext::single();
    let store = cx.new(|cx| {
        AppStore::new(
            core.clone(),
            None,
            None,
            temp.path().join("settings.json"),
            AppSettings::default(),
            None,
            cx,
        )
    });
    cx.update(|cx| {
        cx.set_global(crate::GlobalAppStore(store.clone()));
        cx.set_global(crate::WindowLayoutCheckpoint::default());
        #[cfg(target_os = "macos")]
        let updater = cx.new(|cx| crate::Updater::new(false, cx));
        #[cfg(any(windows, target_os = "linux"))]
        let updater = cx.new(|cx| crate::Updater::new(false, temp.path().join("updates"), cx));
        cx.set_global(crate::GlobalUpdater(updater));
    });
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "sidebar-resize".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = AppRoute::Chat;
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    visual.run_until_parked();
    for zoom in [1., 1.5] {
        host.update(&mut visual, |root, window, cx| {
            root.set_zoom(zoom, window, cx)
        })
        .unwrap();
        visual.run_until_parked();
        let sidebar = visual.debug_bounds("APP_SIDEBAR").unwrap();
        let island_area = visual.debug_bounds("APP_CONTENT_COLUMN").unwrap();
        let gutter = visual.debug_bounds("SIDEBAR_RESIZE_GUTTER").unwrap();
        assert_eq!(island_area.left() - sidebar.right(), px(6. * zoom));
        assert_eq!(gutter.size.width, px(6. * zoom));
        assert_eq!(gutter.left() + gutter.size.width / 2., island_area.left());
        for offset in [-2.9, 0., 2.9] {
            let position = gpui::point(island_area.left() + px(offset * zoom), sidebar.center().y);
            assert!(
                gutter.contains(&position),
                "border point {position:?} outside {gutter:?}"
            );
        }
        let grab = gpui::point(gutter.right() - px(1.), sidebar.center().y);
        let target = gpui::point(island_area.left() + px(40. * zoom), grab.y);
        visual.simulate_mouse_down(grab, gpui::MouseButton::Left, Default::default());
        visual.simulate_mouse_move(
            grab + gpui::point(px(12.), px(0.)),
            Some(gpui::MouseButton::Left),
            Default::default(),
        );
        visual.simulate_mouse_move(target, Some(gpui::MouseButton::Left), Default::default());
        visual.run_until_parked();
        let dragged = visual.debug_bounds("APP_CONTENT_COLUMN").unwrap();
        assert!(
            (f32::from(dragged.left()) - f32::from(target.x)).abs() < 0.5,
            "border {:?} not under pointer {:?}",
            dragged.left(),
            target.x
        );
        visual.simulate_mouse_up(target, gpui::MouseButton::Left, Default::default());
        visual.run_until_parked();
    }
    host.update(&mut visual, |root, _, cx| {
        root.set_sidebar_collapsed(true, false, cx);
        root.sidebar_visibility = SidebarVisibilityTransition::new(false);
        cx.notify();
    })
    .unwrap();
    visual.run_until_parked();
    assert_eq!(
        visual.debug_bounds("APP_CONTENT_COLUMN").unwrap().left(),
        px(8. * 1.5)
    );
    assert!(visual.debug_bounds("SIDEBAR_RESIZE_GUTTER").is_none());
}

#[test]
fn archived_mission_only_leaves_its_open_route() {
    assert_eq!(
        route_after_mission_archived(
            &AppRoute::Mission("mission-1".into()),
            "mission-1",
            Some(false),
            true,
        ),
        Some(AppRoute::Chat)
    );
    for route in [
        AppRoute::Mission("mission-2".into()),
        AppRoute::Roles,
        AppRoute::Settings,
        AppRoute::Chat,
    ] {
        assert_eq!(
            route_after_mission_archived(&route, "mission-1", Some(false), true),
            None
        );
    }
    let open_archived = AppRoute::Mission("mission-1".into());
    assert_eq!(
        route_after_mission_archived(&open_archived, "mission-1", Some(true), true),
        None
    );
    assert_eq!(
        route_after_mission_archived(&open_archived, "mission-1", None, true),
        None
    );
    assert_eq!(
        route_after_mission_archived(&open_archived, "mission-1", Some(false), false),
        None
    );
}

#[test]
fn credit_values_round_to_whole_credits_and_distinguish_missing_from_zero() {
    use runner_core::protocol::UsageCredits;
    assert_eq!(credit_value(None), ("Unavailable".into(), false));
    for (balance, has_credits, unlimited, expected, available) in [
        (Some("1250"), true, false, "1,250", true),
        (Some("1250.750012345678901234"), true, false, "1,251", true),
        (
            Some("9007199254740993.00"),
            true,
            false,
            "9,007,199,254,740,993",
            true,
        ),
        (Some("54135.9439300000"), true, false, "54,136", true),
        (Some("1250.49"), true, false, "1,250", true),
        (Some("1250.499999999999999999"), true, false, "1,250", true),
        (Some("1250.5"), true, false, "1,251", true),
        (Some("999.5"), true, false, "1,000", true),
        (Some("999999.9999"), true, false, "1,000,000", true),
        (
            Some("9007199254740993.5"),
            true,
            false,
            "9,007,199,254,740,994",
            true,
        ),
        (
            Some("999999999999999999999999999999999999999.5"),
            true,
            false,
            "1,000,000,000,000,000,000,000,000,000,000,000,000,000",
            true,
        ),
        (Some("-1.5"), true, false, "-2", true),
        (Some("-0.49"), true, false, "0", true),
        (Some("+000999.5"), true, false, "+1,000", true),
        (Some("0"), true, false, "0", true),
        (Some("0.0000"), true, false, "0", true),
        (None, false, false, "0", true),
        (Some("1250"), false, true, "Unlimited", true),
        (None, true, false, "Unavailable", false),
        (Some("NaN"), false, false, "Unavailable", false),
        (Some("inf"), true, false, "Unavailable", false),
        (Some(""), false, false, "Unavailable", false),
        (Some("12.3e4"), true, false, "123,000", true),
        (Some("1.235e2"), true, false, "124", true),
        (Some("4.999999999999999999e-1"), true, false, "0", true),
        (Some("5e-1"), true, false, "1", true),
        (Some("5e-2"), true, false, "0", true),
        (Some("1e+3"), true, false, "1,000", true),
        (Some("1e"), true, false, "Unavailable", false),
        (Some("1.2.3"), true, false, "Unavailable", false),
    ] {
        let credits = UsageCredits {
            balance: balance.map(str::to_owned),
            has_credits,
            unlimited,
        };
        assert_eq!(credit_value(Some(&credits)), (expected.into(), available));
        let usage = AgentUsage {
            windows: vec![],
            credits: Some(credits),
            updated_at: chrono::Utc::now(),
        };
        assert_eq!(weekly_usage_percent(Runtime::Codex, Some(&usage)), None);
    }
}

struct CreditSectionProbe {
    runtime: Runtime,
    usage: Option<AgentUsage>,
    error: Option<UnavailableReason>,
    zoom: f32,
}

impl Render for CreditSectionProbe {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(px(16. * self.zoom));
        div().w(px(340. * self.zoom)).p_4().child(usage_section(
            self.runtime,
            self.usage.as_ref(),
            self.error,
            true,
            chrono::Utc::now(),
        ))
    }
}

#[test]
fn shared_credit_row_is_codex_only_and_survives_empty_filtered_windows() {
    use gpui::{size, TestAppContext, VisualTestContext};
    use runner_core::protocol::UsageCredits;
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for variant in [
        theme::ThemeVariant::Carbon,
        theme::ThemeVariant::RunnerLight,
    ] {
        theme::set_active_variant(variant);
        for zoom in [1., 1.5] {
            for runtime in [Runtime::Codex, Runtime::ClaudeCode] {
                for state in 0..4 {
                    let mut cx = TestAppContext::single();
                    let usage = (state != 0).then(|| AgentUsage {
                        windows: vec![UsageWindow {
                            name: "5 hours".into(),
                            used_percent: 12.,
                            resets_at: None,
                        }]
                        .into_iter()
                        .filter(|window| !is_five_hour(&window.name))
                        .collect(),
                        credits: (state != 1).then(|| UsageCredits {
                            balance: Some("1250.75001234567890".into()),
                            has_credits: true,
                            unlimited: false,
                        }),
                        updated_at: chrono::Utc::now() - chrono::TimeDelta::minutes(25),
                    });
                    let host = cx.add_window(move |_, _| CreditSectionProbe {
                        runtime,
                        usage,
                        error: (state == 3).then_some(UnavailableReason::CodexNoAnswer),
                        zoom,
                    });
                    let mut visual = VisualTestContext::from_window(host.into(), &cx);
                    visual.simulate_resize(size(px(800.), px(600.)));
                    visual.run_until_parked();
                    let row = visual.debug_bounds("USAGE_CREDITS");
                    assert_eq!(row.is_some(), runtime == Runtime::Codex && state != 0);
                    if let Some(row) = row {
                        let section = visual.debug_bounds("USAGE_SECTION_codex").unwrap();
                        assert!(row.size.height > px(0.));
                        assert!(row.right() <= section.right());
                        assert!(row.bottom() <= section.bottom());
                    }
                }
            }
        }
    }
}

#[test]
fn sidebar_usage_entry_selection_accounts_for_eight_px_gap_and_overflow() {
    let two_digit = [
        (Runtime::Codex, Some(24.), "24%".to_owned()),
        (Runtime::ClaudeCode, Some(65.), "65%".to_owned()),
        (Runtime::Antigravity, Some(41.), "41%".to_owned()),
    ];
    for (max_width, zoom) in [(144., 1.), (216., 1.5)] {
        let (kept, hidden) = usage::select_sidebar_usage_entries(&two_digit, max_width, zoom);
        assert_eq!(kept, vec![0, 1, 2], "{max_width}@{zoom}");
        assert_eq!(hidden, 0, "{max_width}@{zoom}");
    }
    // 12 pad + 3*38 entries + 2*8 gaps = 142 px, so 140 px overflows to 2 + +1.
    assert_eq!(
        usage::select_sidebar_usage_entries(&two_digit, 140., 1.),
        (vec![1, 2], 1)
    );

    let three_digit = [
        (Runtime::Codex, Some(100.), "100%".to_owned()),
        (Runtime::ClaudeCode, Some(100.), "100%".to_owned()),
        (Runtime::Antigravity, Some(100.), "100%".to_owned()),
    ];
    assert_eq!(
        usage::select_sidebar_usage_entries(&three_digit, 144., 1.),
        (vec![0, 1], 1)
    );

    let narrow = [
        (Runtime::Codex, Some(64.), "64%".to_owned()),
        (Runtime::ClaudeCode, Some(22.), "22%".to_owned()),
        (Runtime::Antigravity, Some(91.), "91%".to_owned()),
    ];
    assert_eq!(
        usage::select_sidebar_usage_entries(&narrow, 104., 1.),
        (vec![2], 2)
    );
}

#[test]
fn sidebar_footer_overflow_collapses_lowest_usage_badges_across_three_runtimes() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{size, TestAppContext, VisualTestContext};

    let _theme = ThemeGuard::new();
    theme::set_active_variant(theme::ThemeVariant::Carbon);

    let snapshot_with =
        |codex_pct: f64, claude_pct: f64, gemini_pct: f64, cg_pct: f64| UsageSnapshot {
            runtimes: [
                (
                    Runtime::Codex,
                    runner_core::protocol::usage::RuntimeUsage {
                        value: Some(AgentUsage {
                            credits: None,
                            windows: vec![UsageWindow {
                                name: "Week".into(),
                                used_percent: codex_pct,
                                resets_at: None,
                            }],
                            updated_at: chrono::Utc::now(),
                        }),
                        ..Default::default()
                    },
                ),
                (
                    Runtime::ClaudeCode,
                    runner_core::protocol::usage::RuntimeUsage {
                        value: Some(AgentUsage {
                            credits: None,
                            windows: vec![UsageWindow {
                                name: "Week".into(),
                                used_percent: claude_pct,
                                resets_at: None,
                            }],
                            updated_at: chrono::Utc::now(),
                        }),
                        ..Default::default()
                    },
                ),
                (
                    Runtime::Antigravity,
                    runner_core::protocol::usage::RuntimeUsage {
                        value: Some(AgentUsage {
                            credits: None,
                            windows: vec![
                                UsageWindow {
                                    name: "Gemini Models · Week used".into(),
                                    used_percent: gemini_pct,
                                    resets_at: None,
                                },
                                UsageWindow {
                                    name: "Claude and GPT models · Week used".into(),
                                    used_percent: cg_pct,
                                    resets_at: None,
                                },
                            ],
                            updated_at: chrono::Utc::now(),
                        }),
                        ..Default::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };

    for zoom in [1., 1.5] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, _| {
            store.settings.sidebar_width = app_settings::SIDEBAR_DEFAULT;
            store.settings.app_zoom = zoom;
            store.usage = snapshot_with(24., 65., 41., 73.);
        });
        let host = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "sidebar-footer-three".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.usage_installed = vec![Runtime::Codex, Runtime::ClaudeCode, Runtime::Antigravity];
            root
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1200.), px(900.)));
        visual.run_until_parked();
        store.update(&mut visual, |store, cx| {
            store.usage = snapshot_with(24., 65., 41., 73.);
            cx.notify();
        });
        visual.run_until_parked();

        // Moderate 2-digit values fit all 3 runtimes on 240 px sidebar without overflow.
        for selector in [
            "SIDEBAR_USAGE_ENTRY_codex",
            "SIDEBAR_USAGE_ENTRY_claude-code",
            "SIDEBAR_USAGE_ENTRY_antigravity",
        ] {
            assert!(
                visual.debug_bounds(selector).is_some(),
                "{zoom}: {selector}"
            );
        }
        assert!(visual.debug_bounds("SIDEBAR_USAGE_OVERFLOW").is_none());
        let codex = visual.debug_bounds("SIDEBAR_USAGE_ENTRY_codex").unwrap();
        let claude = visual
            .debug_bounds("SIDEBAR_USAGE_ENTRY_claude-code")
            .unwrap();
        let antigravity = visual
            .debug_bounds("SIDEBAR_USAGE_ENTRY_antigravity")
            .unwrap();
        assert!((claude.left() - codex.right() - px(8. * zoom)).abs() <= px(0.5));
        assert!((antigravity.left() - claude.right() - px(8. * zoom)).abs() <= px(0.5));
        let usage = visual.debug_bounds("SIDEBAR_USAGE").unwrap();
        let host_btn = visual.debug_bounds("SIDEBAR_HOST_BUTTON").unwrap();
        let gear = visual.debug_bounds("SIDEBAR_SETTINGS").unwrap();
        assert!((gear.left() - host_btn.right() - px(8. * zoom)).abs() <= px(0.5));
        assert!(usage.right() <= host_btn.left() - px(8. * zoom - 1.));

        // Narrow 200 px sidebar collapses lowest usage percentages first and preserves highest.
        store.update(&mut visual, |store, cx| {
            store.settings.sidebar_width = app_settings::SIDEBAR_MIN;
            store.usage = snapshot_with(64., 22., 91., 40.);
            cx.notify();
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("SIDEBAR_USAGE_OVERFLOW").is_some());
        assert!(visual
            .debug_bounds("SIDEBAR_USAGE_ENTRY_antigravity")
            .is_some());
        assert!(visual
            .debug_bounds("SIDEBAR_USAGE_ENTRY_claude-code")
            .is_none());
        let usage = visual.debug_bounds("SIDEBAR_USAGE").unwrap();
        let host_btn = visual.debug_bounds("SIDEBAR_HOST_BUTTON").unwrap();
        assert!(usage.right() <= host_btn.left() - px(8. * zoom - 1.));

        // Three 100% badges on default 240 px sidebar deterministically collapse with +N overflow.
        store.update(&mut visual, |store, cx| {
            store.settings.sidebar_width = app_settings::SIDEBAR_DEFAULT;
            store.usage = snapshot_with(100., 100., 100., 100.);
            cx.notify();
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("SIDEBAR_USAGE_OVERFLOW").is_some());
        assert!(visual.debug_bounds("SIDEBAR_USAGE_ENTRY_codex").is_some());
        assert!(visual
            .debug_bounds("SIDEBAR_USAGE_ENTRY_antigravity")
            .is_none());
        let usage = visual.debug_bounds("SIDEBAR_USAGE").unwrap();
        let host_btn = visual.debug_bounds("SIDEBAR_HOST_BUTTON").unwrap();
        assert!(usage.right() <= host_btn.left() - px(8. * zoom - 1.));
    }
}

#[test]
fn muse_weekly_usage_renders_in_the_sidebar_footer() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{size, TestAppContext, VisualTestContext};

    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        store.usage.runtimes.insert(
            Runtime::Muse,
            runner_core::protocol::usage::RuntimeUsage {
                value: Some(AgentUsage {
                    credits: None,
                    windows: vec![UsageWindow {
                        name: "Week".into(),
                        used_percent: 43.,
                        resets_at: None,
                    }],
                    updated_at: chrono::Utc::now(),
                }),
                ..Default::default()
            },
        );
    });
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "muse-sidebar-usage".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.usage_installed = vec![Runtime::Muse];
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    visual.run_until_parked();

    assert!(visual.debug_bounds("SIDEBAR_USAGE_ENTRY_muse").is_some());
    assert!(visual.debug_bounds("SIDEBAR_USAGE_VALUE_muse").is_some());
}

#[test]
fn antigravity_usage_section_renders_grouped_pools_filters_five_hour_and_shows_calm_signed_out() {
    use gpui::{size, TestAppContext, VisualTestContext};

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let now = chrono::Utc::now();
    let all_windows = vec![
        UsageWindow {
            name: "Gemini Models · 5 hours used".into(),
            used_percent: 82.,
            resets_at: Some(now + chrono::TimeDelta::hours(2)),
        },
        UsageWindow {
            name: "Gemini Models · Week used".into(),
            used_percent: 41.,
            resets_at: Some(now + chrono::TimeDelta::days(3)),
        },
        UsageWindow {
            name: "Claude and GPT models · 5 hours used".into(),
            used_percent: 20.,
            resets_at: Some(now + chrono::TimeDelta::hours(3)),
        },
        UsageWindow {
            name: "Claude and GPT models · Week used".into(),
            used_percent: 65.,
            resets_at: Some(now + chrono::TimeDelta::days(5)),
        },
    ];
    let weekly_only: Vec<_> = all_windows
        .iter()
        .filter(|window| !is_five_hour(&window.name))
        .cloned()
        .collect();
    assert_eq!(weekly_only.len(), 2);

    for variant in [
        theme::ThemeVariant::Carbon,
        theme::ThemeVariant::RunnerLight,
    ] {
        theme::set_active_variant(variant);
        for zoom in [1., 1.5] {
            let measure = |usage: Option<AgentUsage>, error: Option<UnavailableReason>| {
                let mut cx = TestAppContext::single();
                let host = cx.add_window(move |_, _| CreditSectionProbe {
                    runtime: Runtime::Antigravity,
                    usage,
                    error,
                    zoom,
                });
                let mut visual = VisualTestContext::from_window(host.into(), &cx);
                visual.simulate_resize(size(px(800.), px(600.)));
                visual.run_until_parked();
                let bounds = visual.debug_bounds("USAGE_SECTION_antigravity").unwrap();
                assert!(bounds.size.width <= px(340. * zoom + 1.));
                bounds
            };
            let full = measure(
                Some(AgentUsage {
                    credits: None,
                    windows: all_windows.clone(),
                    updated_at: now,
                }),
                None,
            );
            let weekly = measure(
                Some(AgentUsage {
                    credits: None,
                    windows: weekly_only.clone(),
                    updated_at: now,
                }),
                None,
            );
            let signed_out = measure(None, Some(UnavailableReason::SignIn));
            assert!(weekly.size.height > px(0.));
            assert!(weekly.size.height < full.size.height);
            assert!(signed_out.size.height > px(0.));
            assert!(signed_out.size.height < weekly.size.height);
        }
    }
}
