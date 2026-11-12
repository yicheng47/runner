use runner_app::updater::{UpdateInfo, UpdateStep};

use super::*;

fn info() -> UpdateInfo {
    UpdateInfo::new("0.19.5")
}

fn downloading(received: u64, total: u64) -> UpdateState {
    UpdateState::Downloading {
        info: info(),
        received,
        total,
    }
}

fn available(size: u64, manual_only: bool) -> UpdateState {
    UpdateState::Available {
        info: info(),
        size,
        manual_only,
    }
}

fn ready() -> UpdateState {
    UpdateState::Ready {
        info: info(),
        size: 40_400_000,
    }
}

fn failed() -> UpdateState {
    UpdateState::Failed {
        step: UpdateStep::Download,
        message: "The network connection was lost.".into(),
        info: Some(info()),
    }
}

#[test]
fn tab_cycles_through_the_actions_in_both_directions() {
    assert_eq!(next_action(None, 3, false), 0);
    assert_eq!(next_action(None, 3, true), 2);
    assert_eq!(next_action(Some(0), 3, false), 1);
    assert_eq!(next_action(Some(2), 3, false), 0);
    assert_eq!(next_action(Some(0), 3, true), 2);
    assert_eq!(next_action(Some(1), 3, true), 0);
}

#[test]
fn ring_fraction_is_clamped_and_unknown_sizes_have_none() {
    assert_eq!(ring_fraction(0, 100), Some(0.));
    assert_eq!(ring_fraction(45, 100), Some(0.45));
    assert_eq!(ring_fraction(500, 100), Some(1.));
    assert_eq!(ring_fraction(45, 0), None);
}

#[test]
fn long_arcs_split_so_svg_can_draw_them() {
    assert_eq!(arc_ends(0., 0.25), [0.25]);
    assert_eq!(arc_ends(0.5, 0.5), [1.0]);
    assert_eq!(arc_ends(0., 0.8), [0.4, 0.8]);
    assert_eq!(arc_ends(0., 1.), [0.5, 1.]);
    assert_eq!(arc_ends(0., 7.), [0.5, 1.]);
}

#[test]
fn ring_starts_at_the_top_and_runs_clockwise() {
    let center = point(px(10.), px(10.));
    let top = ring_point(center, 5., 0.);
    assert!((f32::from(top.x) - 10.).abs() < 0.01 && (f32::from(top.y) - 5.).abs() < 0.01);
    let right = ring_point(center, 5., 0.25);
    assert!((f32::from(right.x) - 15.).abs() < 0.01 && (f32::from(right.y) - 10.).abs() < 0.01);
}

#[test]
fn each_state_has_its_icon_tooltip_and_click() {
    let cases = [
        (
            downloading(18_180_000, 40_400_000),
            UpdateIcon::Ring(0.45),
            "Downloading Runner 0.19.5 · 45%",
            None,
        ),
        (
            downloading(18_180_000, 0),
            UpdateIcon::Arc,
            "Downloading Runner 0.19.5",
            None,
        ),
        (
            UpdateState::Verifying { info: info() },
            UpdateIcon::Arc,
            "Verifying Runner 0.19.5",
            None,
        ),
        (
            ready(),
            UpdateIcon::Restart,
            "Runner 0.19.5 is ready to install",
            Some(IconClick::OpenPopover),
        ),
        (
            available(40_400_000, false),
            UpdateIcon::Download,
            "Download Runner 0.19.5 · 40.4 MB",
            Some(IconClick::StartDownload),
        ),
        (
            available(0, false),
            UpdateIcon::Download,
            "Download Runner 0.19.5",
            Some(IconClick::StartDownload),
        ),
        (
            available(40_400_000, true),
            UpdateIcon::Download,
            "Runner 0.19.5 is available",
            Some(IconClick::OpenSettings),
        ),
        (
            UpdateState::Installing { info: info() },
            UpdateIcon::Arc,
            "Installing Runner 0.19.5",
            None,
        ),
        (
            failed(),
            UpdateIcon::Warning,
            "Runner 0.19.5 couldn't be installed",
            Some(IconClick::OpenSettings),
        ),
    ];
    for (state, icon, tooltip, click) in cases {
        assert_eq!(update_icon(&state), Some(icon), "{state:?}");
        assert_eq!(update_tooltip(&state).as_deref(), Some(tooltip));
        assert_eq!(icon_click(&state), click, "{state:?}");
    }
}

#[test]
fn up_to_date_and_a_failed_check_with_no_version_have_no_icon() {
    for state in [
        UpdateState::UpToDate { checking: false },
        UpdateState::UpToDate { checking: true },
        UpdateState::Failed {
            step: UpdateStep::Check,
            message: "offline".into(),
            info: None,
        },
    ] {
        assert_eq!(update_icon(&state), None);
        assert_eq!(update_tooltip(&state), None);
        assert_eq!(icon_click(&state), None);
        assert_eq!(ready_popover(&state, 3), None);
    }
}

#[test]
fn only_ready_has_a_popover_and_it_carries_the_working_note() {
    for state in [
        downloading(1, 2),
        UpdateState::Verifying { info: info() },
        available(1, false),
        available(1, true),
        UpdateState::Installing { info: info() },
        failed(),
    ] {
        assert_eq!(ready_popover(&state, 2), None, "{state:?}");
    }
    assert_eq!(
        ready_popover(&ready(), 0),
        Some(ReadyPopover {
            title: "Runner 0.19.5 is ready".into(),
            working: None,
        })
    );
    assert_eq!(
        ready_popover(&ready(), 2).unwrap().working.as_deref(),
        Some("2 agents working")
    );
}

mod in_a_window {
    use gpui::{
        px, size, InputEvent as _, Modifiers, TestAppContext, VisualTestContext, WindowHandle,
    };

    use super::*;
    use crate::theme_snapshot::ThemeGuard;

    struct Opened {
        handle: WindowHandle<NativeRoot>,
        visual: VisualTestContext,
        store: Entity<AppStore>,
        _temp: tempfile::TempDir,
        _cx: TestAppContext,
    }

    fn open_window(
        cx: &mut TestAppContext,
        store: &Entity<AppStore>,
        logs: std::path::PathBuf,
    ) -> WindowHandle<NativeRoot> {
        cx.add_window(|window, cx| {
            NativeRoot::new(
                "update-ui".into(),
                logs,
                None,
                None,
                store.clone(),
                window,
                cx,
            )
        })
    }

    fn opened() -> Opened {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let handle = open_window(&mut cx, &store, temp.path().join("logs"));
        let visual = VisualTestContext::from_window(handle.into(), &cx);
        visual.simulate_resize(size(px(1200.), px(900.)));
        visual.run_until_parked();
        Opened {
            handle,
            visual,
            store,
            _temp: temp,
            _cx: cx,
        }
    }

    fn set_state(cx: &mut VisualTestContext, state: UpdateState) {
        cx.update(|_, cx| {
            global_updater(cx).update(cx, |updater, cx| updater.set_state_for_test(state, cx))
        });
        cx.run_until_parked();
    }

    fn set_preview(visual: &mut VisualTestContext, state: UpdateState) {
        visual.update(|_, cx| {
            global_updater(cx).update(cx, |updater, cx| updater.set_preview_for_test(state, cx))
        });
        visual.run_until_parked();
    }

    fn state(visual: &mut VisualTestContext) -> UpdateState {
        visual.update(|_, cx| global_updater(cx).read(cx).state().clone())
    }

    fn is_open(handle: WindowHandle<NativeRoot>, cx: &VisualTestContext) -> bool {
        handle
            .read_with(cx, |root, _| root.update_popover.open)
            .unwrap()
    }

    fn route(handle: WindowHandle<NativeRoot>, cx: &VisualTestContext) -> AppRoute {
        handle.read_with(cx, |root, _| root.route.clone()).unwrap()
    }

    fn focused(handle: WindowHandle<NativeRoot>, visual: &mut VisualTestContext) -> FocusHandle {
        handle
            .update(visual, |_, window, cx| {
                window.focused(cx).expect("the window focuses something")
            })
            .unwrap()
    }

    fn is_focused(
        handle: WindowHandle<NativeRoot>,
        visual: &mut VisualTestContext,
        focus: &FocusHandle,
    ) -> bool {
        handle
            .update(visual, |_, window, _| focus.is_focused(window))
            .unwrap()
    }

    fn click_icon(visual: &mut VisualTestContext) {
        let icon = visual.debug_bounds("SIDEBAR_UPDATE").unwrap();
        visual.simulate_click(icon.center(), Modifiers::default());
        visual.run_until_parked();
    }

    fn click(visual: &mut VisualTestContext, selector: &'static str) {
        let bounds = visual.debug_bounds(selector).unwrap();
        visual.simulate_click(bounds.center(), Modifiers::default());
        visual.run_until_parked();
    }

    /// A whole key press: `simulate_keystrokes` alone sends only the key-down, and GPUI clicks
    /// the focused element on the key-up.
    fn press(visual: &mut VisualTestContext, key: &str) {
        visual.simulate_keystrokes(key);
        visual.run_until_parked();
        let keystroke = gpui::Keystroke::parse(key).unwrap();
        visual.update(|window, cx| {
            window.dispatch_event(gpui::KeyUpEvent { keystroke }.to_platform_input(), cx);
        });
        visual.run_until_parked();
    }

    #[test]
    fn clicking_the_available_icon_starts_one_download_and_changes_the_icon_at_once() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_preview(&mut visual, available(40_400_000, false));
        let before = route(handle, &visual);
        click_icon(&mut visual);
        assert!(matches!(
            state(&mut visual),
            UpdateState::Downloading { received: 0, .. }
        ));
        assert!(!is_open(handle, &visual));
        assert_eq!(route(handle, &visual), before);
    }

    #[test]
    fn one_key_press_on_the_icon_starts_the_download_and_doesnt_go_on_to_settings() {
        let _theme = ThemeGuard::new();
        for key in ["enter", "space"] {
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_preview(&mut visual, available(40_400_000, false));
            let before = route(handle, &visual);
            handle
                .update(&mut visual, |root, window, cx| {
                    root.update_popover.icon_focus.clone().focus(window, cx)
                })
                .unwrap();
            visual.run_until_parked();
            press(&mut visual, key);
            assert!(
                matches!(state(&mut visual), UpdateState::Downloading { .. }),
                "{key}"
            );
            assert_eq!(route(handle, &visual), before, "{key}");
        }
    }

    #[test]
    fn a_double_click_at_available_starts_one_download_and_stays_where_it_is() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        let before = route(handle, &visual);
        let focus = focused(handle, &mut visual);
        set_preview(&mut visual, available(40_400_000, false));

        click_icon(&mut visual);
        // The first click's download has moved on, so a second one would show from zero.
        visual.executor().advance_clock(Duration::from_millis(250));
        visual.run_until_parked();
        let progress = state(&mut visual);
        assert!(matches!(
            progress,
            UpdateState::Downloading { received, .. } if received > 0
        ));

        let position = visual.debug_bounds("SIDEBAR_UPDATE").unwrap().center();
        visual.simulate_event(gpui::MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count: 2,
            first_mouse: false,
        });
        visual.simulate_event(gpui::MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count: 2,
        });
        visual.run_until_parked();

        assert_eq!(state(&mut visual), progress);
        assert_eq!(route(handle, &visual), before);
        assert!(!is_open(handle, &visual));
        assert!(is_focused(handle, &mut visual, &focus));
    }

    #[test]
    fn clicking_the_icon_does_nothing_while_an_update_is_under_way() {
        let _theme = ThemeGuard::new();
        for update in [
            downloading(18_180_000, 40_400_000),
            downloading(18_180_000, 0),
            UpdateState::Verifying { info: info() },
            UpdateState::Installing { info: info() },
        ] {
            let Opened {
                handle, mut visual, ..
            } = opened();
            let before = route(handle, &visual);
            // Whatever the window itself rendered and focused, as a terminal would be.
            let focus = focused(handle, &mut visual);
            set_state(&mut visual, update.clone());

            click_icon(&mut visual);

            assert_eq!(route(handle, &visual), before, "{update:?}");
            assert_eq!(state(&mut visual), update);
            assert!(!is_open(handle, &visual), "{update:?}");
            assert!(is_focused(handle, &mut visual, &focus), "{update:?}");
            assert!(visual.debug_bounds("UPDATE_HERO_STATUS").is_none());
            assert!(visual.debug_bounds("UPDATE_POPOVER").is_none());
            // It stays an ordinary icon, with its tooltip.
            assert!(visual.debug_bounds("SIDEBAR_UPDATE").is_some());
        }
    }

    #[test]
    fn failed_and_manual_download_open_settings_from_the_icon() {
        let _theme = ThemeGuard::new();
        for update in [failed(), available(1, true)] {
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_state(&mut visual, update.clone());
            click_icon(&mut visual);
            assert_eq!(route(handle, &visual), AppRoute::Settings, "{update:?}");
            assert!(!is_open(handle, &visual), "{update:?}");
            assert_eq!(state(&mut visual), update);
            assert!(visual.debug_bounds("UPDATE_HERO_STATUS").is_some());
            assert!(visual.debug_bounds("UPDATE_POPOVER").is_none());
        }
    }

    #[test]
    fn the_ready_icon_opens_the_popover_and_escape_returns_focus() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_state(&mut visual, ready());
        let previous = focused(handle, &mut visual);
        click_icon(&mut visual);
        assert!(is_open(handle, &visual));
        let icon = visual.debug_bounds("SIDEBAR_UPDATE").unwrap();
        let popover = visual.debug_bounds("UPDATE_POPOVER").unwrap();
        assert!(popover.top() >= icon.bottom());
        assert_eq!(popover.size.width, px(POPOVER_WIDTH));
        assert!(!is_focused(handle, &mut visual, &previous));

        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        assert!(!is_open(handle, &visual));
        assert!(visual.debug_bounds("UPDATE_POPOVER").is_none());
        assert!(is_focused(handle, &mut visual, &previous));
    }

    #[test]
    fn a_click_outside_closes_the_popover_and_later_keeps_the_icon() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_state(&mut visual, ready());
        click_icon(&mut visual);
        assert!(is_open(handle, &visual));
        visual.simulate_click(gpui::point(px(900.), px(600.)), Modifiers::default());
        visual.run_until_parked();
        assert!(!is_open(handle, &visual));

        click_icon(&mut visual);
        click(&mut visual, "update-popover-later");
        assert!(!is_open(handle, &visual));
        assert!(visual.debug_bounds("SIDEBAR_UPDATE").is_some());
        assert!(matches!(state(&mut visual), UpdateState::Ready { .. }));
    }

    #[test]
    fn restart_closes_the_popover_and_the_icon_shows_installing() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_preview(&mut visual, ready());
        let previous = focused(handle, &mut visual);
        click_icon(&mut visual);
        click(&mut visual, "UPDATE_POPOVER_RESTART");
        assert!(!is_open(handle, &visual));
        assert!(matches!(state(&mut visual), UpdateState::Installing { .. }));
        assert!(visual.debug_bounds("SIDEBAR_UPDATE").is_some());
        assert!(is_focused(handle, &mut visual, &previous));
    }

    #[test]
    fn the_popover_closes_when_the_state_leaves_ready_and_returns_focus() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_state(&mut visual, ready());
        let previous = focused(handle, &mut visual);
        click_icon(&mut visual);
        visual.simulate_keystrokes("tab");
        visual.run_until_parked();
        set_state(&mut visual, failed());
        assert!(!is_open(handle, &visual));
        assert!(is_focused(handle, &mut visual, &previous));
    }

    #[test]
    fn a_download_started_from_the_icon_opens_ready_only_in_that_window_without_focus() {
        let _theme = ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let started = open_window(&mut cx, &store, temp.path().join("logs"));
        let other = open_window(&mut cx, &store, temp.path().join("logs"));
        let mut visual = VisualTestContext::from_window(started.into(), &cx);
        visual.simulate_resize(size(px(1200.), px(900.)));
        visual.run_until_parked();
        let focus = focused(started, &mut visual);

        set_preview(&mut visual, available(40_400_000, false));
        click_icon(&mut visual);
        assert!(matches!(
            state(&mut visual),
            UpdateState::Downloading { .. }
        ));
        set_state(&mut visual, UpdateState::Verifying { info: info() });
        assert!(!is_open(started, &visual));
        set_state(&mut visual, ready());
        assert!(is_open(started, &visual));
        assert!(!is_open(other, &visual));
        assert!(visual.debug_bounds("UPDATE_POPOVER").is_some());
        assert!(is_focused(started, &mut visual, &focus));

        // Only once per download.
        started
            .update(&mut visual, |root, window, cx| {
                root.close_update_popover(window, cx)
            })
            .unwrap();
        set_state(&mut visual, available(1, false));
        set_state(&mut visual, ready());
        assert!(!is_open(started, &visual));
    }

    #[test]
    fn an_automatic_download_only_changes_the_icon() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_state(&mut visual, downloading(1, 2));
        set_state(&mut visual, ready());
        assert!(!is_open(handle, &visual));
        assert!(visual.debug_bounds("SIDEBAR_UPDATE").is_some());
    }

    #[test]
    fn a_download_started_from_the_hero_opens_ready_once_the_person_has_left_settings() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_preview(&mut visual, available(40_400_000, false));
        let before = route(handle, &visual);
        handle
            .update(&mut visual, |root, window, cx| {
                root.enter_settings_route(Some("updates"), window, cx)
            })
            .unwrap();
        visual.run_until_parked();
        click(&mut visual, "UPDATE_HERO_Download");
        assert!(matches!(
            state(&mut visual),
            UpdateState::Downloading { .. }
        ));
        handle
            .update(&mut visual, |root, _, cx| root.set_route(before, cx))
            .unwrap();
        visual.run_until_parked();
        set_state(&mut visual, ready());
        assert!(is_open(handle, &visual));

        // Still on Settings, the hero shows Ready and nothing opens.
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_preview(&mut visual, available(40_400_000, false));
        handle
            .update(&mut visual, |root, window, cx| {
                root.enter_settings_route(Some("updates"), window, cx)
            })
            .unwrap();
        visual.run_until_parked();
        click(&mut visual, "UPDATE_HERO_Download");
        set_state(&mut visual, ready());
        assert!(!is_open(handle, &visual));
        assert!(visual.debug_bounds("UPDATE_HERO_Restart").is_some());
    }

    #[test]
    fn a_cancelled_or_failed_download_does_not_open_ready_later() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        for end in [available(1, false), failed()] {
            set_state(&mut visual, downloading(1, 2));
            handle
                .update(&mut visual, |root, _, cx| {
                    root.mark_update_download_started(cx)
                })
                .unwrap();
            set_state(&mut visual, end);
            set_state(&mut visual, ready());
            assert!(!is_open(handle, &visual));
        }
    }

    #[test]
    fn the_hero_carries_every_action_that_left_the_popover() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle,
            mut visual,
            store,
            ..
        } = opened();
        set_preview(&mut visual, available(40_400_000, false));
        handle
            .update(&mut visual, |root, window, cx| {
                root.enter_settings_route(Some("updates"), window, cx)
            })
            .unwrap();
        visual.run_until_parked();

        click(&mut visual, "UPDATE_HERO_Download");
        assert!(matches!(
            state(&mut visual),
            UpdateState::Downloading { .. }
        ));
        click(&mut visual, "UPDATE_HERO_Cancel");
        assert!(matches!(state(&mut visual), UpdateState::Available { .. }));
        assert_eq!(
            visual.update(|_, cx| store.read(cx).settings.skipped_update_version.clone()),
            None
        );

        set_preview(&mut visual, failed());
        assert!(visual.debug_bounds("UPDATE_HERO_OpenRelease").is_some());
        click(&mut visual, "UPDATE_HERO_Retry");
        assert!(matches!(
            state(&mut visual),
            UpdateState::Downloading { .. }
        ));

        set_preview(&mut visual, available(40_400_000, true));
        click(&mut visual, "UPDATE_HERO_OpenRelease");
        assert_eq!(visual.cx.opened_url().as_deref(), Some(release_page_url()));
        assert!(visual.debug_bounds("UPDATE_HERO_Download").is_none());
    }

    #[test]
    fn one_whole_key_press_runs_each_popover_action_once() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_preview(&mut visual, ready());
        click_icon(&mut visual);

        // What's new opens the page and leaves the popover; Later closes it.
        visual.simulate_keystrokes("tab tab");
        visual.run_until_parked();
        press(&mut visual, "enter");
        assert_eq!(visual.cx.opened_url().as_deref(), Some(release_page_url()));
        assert!(is_open(handle, &visual));
        visual.simulate_keystrokes("tab");
        visual.run_until_parked();
        press(&mut visual, "enter");
        assert!(!is_open(handle, &visual));
        assert!(matches!(state(&mut visual), UpdateState::Ready { .. }));

        click_icon(&mut visual);
        visual.simulate_keystrokes("tab");
        visual.run_until_parked();
        press(&mut visual, "enter");
        assert!(!is_open(handle, &visual));
        assert!(matches!(state(&mut visual), UpdateState::Installing { .. }));
    }

    #[test]
    fn the_keyboard_opens_the_popover_from_the_palette_and_tabs_through_it() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_state(&mut visual, ready());
        let previous = focused(handle, &mut visual);
        handle
            .update(&mut visual, |root, window, cx| {
                root.command_palette
                    .update(cx, |palette, cx| palette.open(window, cx))
            })
            .unwrap();
        visual.run_until_parked();
        visual.simulate_input("show runner");
        visual.run_until_parked();
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        assert!(is_open(handle, &visual));
        assert!(visual.debug_bounds("UPDATE_POPOVER").is_some());
        assert!(!is_focused(handle, &mut visual, &previous));

        // Restart to update, What's new, then Later, which closes it and returns focus.
        visual.simulate_keystrokes("tab tab tab");
        visual.run_until_parked();
        press(&mut visual, "enter");
        assert!(!is_open(handle, &visual));
        assert!(visual.cx.opened_url().is_none());
        assert!(is_focused(handle, &mut visual, &previous));
    }

    #[test]
    fn the_palette_command_opens_settings_in_every_state_but_ready() {
        let _theme = ThemeGuard::new();
        for update in [
            available(1, false),
            available(1, true),
            downloading(1, 2),
            UpdateState::Verifying { info: info() },
            UpdateState::Installing { info: info() },
            failed(),
        ] {
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_state(&mut visual, update.clone());
            handle
                .update(&mut visual, |root, window, cx| {
                    root.show_update_from_keyboard(window, cx);
                    assert_eq!(root.route, AppRoute::Settings, "{update:?}");
                    assert!(!root.update_popover.open, "{update:?}");
                })
                .unwrap();
            visual.run_until_parked();
            // It never starts the download, unlike the icon.
            assert_eq!(state(&mut visual), update);
            assert!(visual.debug_bounds("UPDATE_HERO_STATUS").is_some());
        }
    }

    #[test]
    fn tab_wraps_through_the_actions_in_both_directions() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_state(&mut visual, ready());
        click_icon(&mut visual);

        let on = |visual: &mut VisualTestContext, index: usize| {
            handle
                .update(visual, |root, window, _| {
                    root.update_popover.action_focus[index].is_focused(window)
                })
                .unwrap()
        };
        visual.simulate_keystrokes("tab");
        visual.run_until_parked();
        assert!(on(&mut visual, 0));
        visual.simulate_keystrokes("shift-tab");
        visual.run_until_parked();
        assert!(on(&mut visual, 2));
        visual.simulate_keystrokes("tab");
        visual.run_until_parked();
        assert!(on(&mut visual, 0));
        visual.simulate_keystrokes("tab tab");
        visual.run_until_parked();
        assert!(on(&mut visual, 2));
        assert!(is_open(handle, &visual));
    }

    #[test]
    fn clicking_later_on_a_popover_that_opened_by_itself_keeps_terminal_focus() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_state(&mut visual, downloading(1, 2));
        handle
            .update(&mut visual, |root, _, cx| {
                root.mark_update_download_started(cx)
            })
            .unwrap();
        let focus = focused(handle, &mut visual);
        set_state(&mut visual, ready());
        assert!(is_open(handle, &visual));
        click(&mut visual, "update-popover-later");
        assert!(!is_open(handle, &visual));
        assert!(is_focused(handle, &mut visual, &focus));
    }

    #[test]
    fn a_stale_palette_command_opens_settings_when_the_state_has_moved_on() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_state(&mut visual, ready());
        handle
            .update(&mut visual, |root, window, cx| {
                root.command_palette
                    .update(cx, |palette, cx| palette.open(window, cx))
            })
            .unwrap();
        visual.run_until_parked();
        visual.simulate_input("show runner");
        visual.run_until_parked();
        set_state(&mut visual, UpdateState::UpToDate { checking: false });
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        handle
            .read_with(&visual, |root, _| {
                assert!(!root.update_popover.open);
                assert_eq!(root.route, AppRoute::Settings);
            })
            .unwrap();
    }

    #[test]
    fn without_an_icon_the_palette_command_opens_settings_updates() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_state(&mut visual, ready());
        handle
            .update(&mut visual, |root, window, cx| {
                root.enter_settings_route(None, window, cx);
                root.show_update_from_keyboard(window, cx);
                assert_eq!(root.route, AppRoute::Settings);
                assert!(!root.update_popover.open);
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("UPDATE_HERO_STATUS").is_some());
    }

    #[test]
    fn escape_closes_a_popover_that_opened_by_itself_and_leaves_focus_alone() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle, mut visual, ..
        } = opened();
        set_state(&mut visual, downloading(1, 2));
        // Whatever the window itself rendered and focused, as a terminal would be.
        let focus = handle
            .update(&mut visual, |root, window, cx| {
                root.mark_update_download_started(cx);
                window.focused(cx).expect("the window focuses something")
            })
            .unwrap();
        set_state(&mut visual, ready());
        assert!(is_open(handle, &visual));

        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        assert!(!is_open(handle, &visual));
        assert!(is_focused(handle, &mut visual, &focus));
    }

    struct EscapeProbe {
        root: Entity<NativeRoot>,
        focus: FocusHandle,
        keys_received: usize,
    }

    impl gpui::Render for EscapeProbe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let root = self.root.clone();
            div()
                .size_full()
                .capture_key_down(move |_, window, cx| {
                    root.update(cx, |root, cx| {
                        root.close_unfocused_update_popover(window, cx)
                    });
                })
                .child(div().size_full().track_focus(&self.focus).on_key_down(
                    cx.listener(|this, _: &KeyDownEvent, _, _| this.keys_received += 1),
                ))
        }
    }

    #[test]
    fn that_escape_still_reaches_whatever_has_focus() {
        let _theme = ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let host = cx.add_window(|window, cx| {
            let focus = cx.focus_handle();
            EscapeProbe {
                root: cx.new(|cx| {
                    NativeRoot::new(
                        "update-ui-escape".into(),
                        temp.path().join("logs"),
                        None,
                        None,
                        store.clone(),
                        window,
                        cx,
                    )
                }),
                focus,
                keys_received: 0,
            }
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.run_until_parked();
        host.update(&mut visual, |probe, window, cx| {
            probe.focus.focus(window, cx);
            probe.root.update(cx, |root, cx| {
                root.open_ready_update_popover(window, cx);
                assert!(root.update_popover.open);
            });
        })
        .unwrap();

        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        host.read_with(&visual, |probe, cx| {
            assert!(!probe.root.read(cx).update_popover.open);
            assert_eq!(probe.keys_received, 1);
        })
        .unwrap();
    }

    #[test]
    fn the_popover_fits_small_windows_and_wraps_a_long_version_at_each_zoom() {
        let _theme = ThemeGuard::new();
        for zoom in [1., 1.3, 1.5] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            store.update(&mut cx, |store, _| store.settings.app_zoom = zoom);
            let handle = open_window(&mut cx, &store, temp.path().join("logs"));
            let mut visual = VisualTestContext::from_window(handle.into(), &cx);
            let viewport = size(px(700.), px(500.));
            visual.simulate_resize(viewport);
            visual.update(|_, cx| {
                store.update(cx, |store, _| {
                    crate::app_store::seed_mixed_working_sessions(store)
                })
            });
            let long = UpdateInfo::new("preview-nightly.abc1234abc1234abc1234.20261104.0100");
            set_state(
                &mut visual,
                UpdateState::Ready {
                    info: long,
                    size: 40_400_000,
                },
            );
            click_icon(&mut visual);
            let popover = visual.debug_bounds("UPDATE_POPOVER").unwrap();
            let title = visual.debug_bounds("UPDATE_POPOVER_TITLE").unwrap();
            let restart = visual.debug_bounds("UPDATE_POPOVER_RESTART").unwrap();
            assert!((popover.size.width - px(POPOVER_WIDTH * zoom)).abs() < px(1.));
            assert!(popover.right() <= viewport.width);
            assert!(popover.bottom() <= viewport.height);
            assert!(title.left() >= popover.left() && title.right() <= popover.right());
            assert!(restart.bottom() <= popover.bottom());
        }
    }

    #[test]
    fn the_ready_popover_shows_working_agents_and_clears_when_they_finish() {
        let _theme = ThemeGuard::new();
        let Opened {
            mut visual, store, ..
        } = opened();
        visual.update(|_, cx| {
            store.update(cx, |store, _| {
                crate::app_store::seed_mixed_working_sessions(store)
            })
        });
        set_state(&mut visual, ready());
        click_icon(&mut visual);
        assert!(visual.debug_bounds("UPDATE_WORKING_NOTE").is_some());
        assert!(visual.debug_bounds("UPDATE_WORKING_DOT").is_some());
        assert_eq!(
            visual.update(|_, cx| store.read(cx).working_agent_count()),
            2
        );
        visual.update(|_, cx| {
            store.update(cx, |store, cx| {
                store.session_statuses.clear();
                cx.notify();
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("UPDATE_WORKING_NOTE").is_none());
    }

    #[test]
    fn skip_in_settings_hides_the_icon_persists_and_a_manual_check_brings_it_back() {
        let _theme = ThemeGuard::new();
        let Opened {
            handle,
            mut visual,
            store,
            ..
        } = opened();
        set_state(&mut visual, available(1, false));
        handle
            .update(&mut visual, |root, window, cx| {
                root.enter_settings_route(Some("updates"), window, cx)
            })
            .unwrap();
        visual.run_until_parked();
        click(&mut visual, "UPDATE_HERO_Skip");
        assert_eq!(
            visual.update(|_, cx| store.read(cx).settings.skipped_update_version.clone()),
            Some("0.19.5".to_owned())
        );
        assert!(visual.update(|_, cx| global_updater(cx).read(cx).is_skipped()));
        assert_eq!(
            visual.update(|_, cx| visible_update_state(global_updater(cx).read(cx))),
            None
        );

        handle
            .update(&mut visual, |root, window, cx| {
                check_for_updates_from_menu(root, window, cx);
                assert_eq!(root.route, AppRoute::Settings);
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("UPDATE_HERO_STATUS").is_some());
        assert_eq!(
            visual.update(|_, cx| store.read(cx).settings.skipped_update_version.clone()),
            None
        );
        assert!(visual.update(|_, cx| !global_updater(cx).read(cx).is_skipped()));
    }
}
