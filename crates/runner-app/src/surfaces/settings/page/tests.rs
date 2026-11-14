use super::*;

#[test]
#[cfg(target_os = "macos")]
fn shortcut_rows_keep_registry_order_with_one_tab_row_and_display_only_system_keys() {
    let overrides = keymap::KeymapOverrides::new();
    let rows = shortcut_rows("", &overrides);
    let titles = rows.iter().map(|row| row.title()).collect::<Vec<_>>();
    assert_eq!(titles.len(), 31);
    assert_eq!(titles[0], "New window");
    assert_eq!(
        titles[28..],
        ["Copy selection", "Paste", "Toggle fullscreen"]
    );
    let after = |title: &str| titles[titles.iter().position(|t| *t == title).unwrap() + 1];
    assert_eq!(after("Reset zoom"), "Go to tab 1–9");
    assert_eq!(after("Toggle mission terminal drawer"), "Find in terminal");
    assert_eq!(after("Split pane down"), "Maximize pane");
    assert_eq!(after("Maximize pane"), "Close tab");
    assert_eq!(
        rows.iter()
            .filter(|row| row.fixed())
            .map(|row| row.title())
            .collect::<Vec<_>>(),
        [
            "New window",
            "Go to tab 1–9",
            "Close tab",
            "Close window",
            "Copy selection",
            "Paste",
            "Toggle fullscreen"
        ]
    );
    assert_eq!(ShortcutRow::TabSelection.binding_label(&overrides), "⌘1–⌘9");
    assert_eq!(
        rows[28..]
            .iter()
            .map(|row| row.binding_label(&overrides))
            .collect::<Vec<_>>(),
        ["⌘C", "⌘V", "⌃⌘F"]
    );
}

#[test]
fn shortcut_sections_follow_the_guide_and_hide_sections_without_a_match() {
    crate::i18n::testing::with_locale(crate::i18n::Locale::English, || {
        let overrides = keymap::KeymapOverrides::new();
        let sections = shortcut_sections("", &overrides, false);
        assert_eq!(
            sections
                .iter()
                .map(|(section, _)| section.title())
                .collect::<Vec<_>>(),
            [
                "Navigation and creation",
                "Panes and sessions",
                "Missions",
                "Terminal",
                "Zoom and window",
            ]
        );
        let ids = sections
            .iter()
            .map(|(_, rows)| rows.iter().map(|row| row.id()).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                &[
                    "new-window",
                    "new-chat",
                    "new-terminal",
                    "new-mission",
                    "command-palette",
                    "search-files",
                    "toggle-sidebar",
                    "switch-sidebar-view",
                    "open-settings",
                    "page-previous",
                    "page-next",
                    TAB_SHORTCUT_ID,
                ][..],
                &[
                    "pane-previous",
                    "pane-next",
                    "split-pane-right",
                    "split-pane-down",
                    "toggle-maximize-pane",
                    "close-pane",
                    "stop-session",
                    "resume-session",
                ],
                &[
                    "toggle-terminal-drawer",
                    "mission-tab-previous",
                    "mission-tab-next",
                ],
                &["find-in-terminal", "copy", "system-paste"],
                &[
                    "zoom-in",
                    "zoom-out",
                    "zoom-reset",
                    "system-close-window",
                    "system-toggle-fullscreen",
                ],
            ]
        );
        assert_eq!(
            ids.iter().map(Vec::len).sum::<usize>(),
            shortcut_rows("", &overrides).len()
        );

        let zoom = shortcut_sections("zoom", &overrides, false);
        assert_eq!(zoom.len(), 1);
        assert_eq!(zoom[0].0, ShortcutSection::ZoomAndWindow);
        assert_eq!(zoom[0].1.len(), 3);
        let paste = shortcut_sections("paste", &overrides, false);
        assert_eq!(paste.len(), 1);
        assert_eq!(paste[0].0, ShortcutSection::Terminal);
        assert_eq!(paste[0].1.len(), 1);
        assert_eq!(paste[0].1[0].id(), "system-paste");
        for row in sections.iter().flat_map(|(_, rows)| rows) {
            if let ShortcutRow::DisplayOnly(entry) = row {
                assert!(row.fixed());
                assert!(row.entry_ids().is_empty());
                assert!(!row.overridden(&overrides));
                assert!(!row.freed_for_terminal(&overrides));
                assert_eq!(
                    row.binding_label(&overrides),
                    keymap::format_combo(&entry.default)
                );
                assert!(keymap::entry(entry.id).is_none());
            }
        }

        assert!(ctrl_keys_setting_matches(""));
        assert!(ctrl_keys_setting_matches("ctrl keys"));
        assert!(!ctrl_keys_setting_matches("zoom"));
        let setting = shortcut_sections("ctrl keys", &overrides, true);
        assert_eq!(setting.len(), 1);
        assert_eq!(setting[0].0, ShortcutSection::Terminal);
        assert!(setting[0].1.is_empty());
        assert!(shortcut_sections("ctrl keys", &overrides, false).is_empty());
    });
}

#[test]
fn switch_sidebar_view_settings_row_is_editable_and_tracks_its_effective_key() {
    for locale in [
        crate::i18n::Locale::English,
        crate::i18n::Locale::SimplifiedChinese,
    ] {
        crate::i18n::testing::with_locale(locale, || {
            let mut overrides = keymap::KeymapOverrides::new();
            let entry = keymap::entry("switch-sidebar-view").unwrap();
            let rows = shortcut_rows(entry.title(), &overrides);
            assert_eq!(rows, [ShortcutRow::Entry(entry)]);
            let row = rows[0];
            assert!(!row.fixed());
            assert_eq!(ShortcutSection::of(row), ShortcutSection::Navigation);
            assert_eq!(
                row.binding_label(&overrides),
                keymap::format_combo(&entry.default)
            );
            let mut combo = entry.default.clone();
            combo.meta = false;
            combo.ctrl = true;
            overrides.insert(entry.id.into(), Some(combo.clone()));
            assert!(row.freed_for_terminal(&overrides));
            combo.shift = true;
            overrides.insert(entry.id.into(), Some(combo));
            assert!(!row.freed_for_terminal(&overrides));
            overrides.insert(entry.id.into(), None);
            assert!(!row.freed_for_terminal(&overrides));
            assert_eq!(
                row.binding_label(&overrides),
                crate::i18n::text("ui.unassigned")
            );
        });
    }
}

#[test]
fn outside_terminals_tags_follow_the_effective_binding() {
    let bare = keymap::KeyCombo {
        meta: false,
        ctrl: true,
        alt: false,
        shift: false,
        code: "KeyJ".into(),
        label: None,
        shift_optional: false,
    };
    let entry_row = |id| ShortcutRow::Entry(keymap::entry(id).unwrap());
    let overrides = keymap::KeymapOverrides::from([
        ("new-terminal".to_owned(), Some(bare.clone())),
        ("zoom-reset".to_owned(), Some(bare.clone())),
    ]);
    assert!(entry_row("new-terminal").freed_for_terminal(&overrides));
    assert!(!entry_row("zoom-reset").freed_for_terminal(&overrides));
    assert!(!entry_row("new-mission").freed_for_terminal(&overrides));
    let freed = cfg!(any(windows, target_os = "linux"));
    for id in [
        "new-chat",
        "toggle-sidebar",
        "switch-sidebar-view",
        "open-settings",
        "pane-next",
    ] {
        assert_eq!(entry_row(id).freed_for_terminal(&overrides), freed, "{id}");
    }
    assert_eq!(
        ShortcutRow::TabSelection.freed_for_terminal(&overrides),
        freed
    );
    let mut unbound = overrides.clone();
    for id in ShortcutRow::TabSelection.entry_ids() {
        unbound.insert(id.into(), None);
    }
    assert!(!ShortcutRow::TabSelection.freed_for_terminal(&unbound));
    for id in [
        "command-palette",
        "split-pane-right",
        "split-pane-down",
        "zoom-out",
    ] {
        assert!(!entry_row(id).freed_for_terminal(&overrides), "{id}");
    }
}

#[test]
fn shortcuts_page_renders_sections_setting_row_and_tags() {
    use gpui::{size, TestAppContext, VisualTestContext};

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    crate::i18n::testing::with_locale(crate::i18n::Locale::English, || {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let host = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "shortcut-sections".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.route = AppRoute::Settings;
            root.settings_page.pane = SettingsPane::Shortcuts;
            root
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1200.), px(4000.)));
        visual.run_until_parked();
        for selector in [
            "SHORTCUT_SECTION_navigation",
            "SHORTCUT_SECTION_panes",
            "SHORTCUT_SECTION_missions",
            "SHORTCUT_SECTION_terminal",
            "SHORTCUT_SECTION_zoom-and-window",
        ] {
            assert!(visual.debug_bounds(selector).is_some(), "{selector}");
        }
        let terminal = visual.debug_bounds("SHORTCUT_SECTION_terminal").unwrap();
        assert!(
            visual
                .debug_bounds("SHORTCUT_SECTION_missions")
                .unwrap()
                .bottom()
                < terminal.top()
        );
        assert!(
            terminal.bottom()
                < visual
                    .debug_bounds("SHORTCUT_SECTION_zoom-and-window")
                    .unwrap()
                    .top()
        );
        assert_eq!(
            visual.debug_bounds("SETTINGS_TERMINAL_CTRL_KEYS").is_some(),
            cfg!(any(windows, target_os = "linux"))
        );
        assert!(visual
            .debug_bounds("SHORTCUT_OUTSIDE_TERMINALS_new-chat")
            .is_none());
        for selector in [
            "SHORTCUT_LOCKED_copy",
            "SHORTCUT_LOCKED_system-paste",
            "SHORTCUT_LOCKED_system-toggle-fullscreen",
        ] {
            assert!(visual.debug_bounds(selector).is_some(), "{selector}");
        }
        let fullscreen = visual
            .debug_bounds("SHORTCUT_LOCKED_system-toggle-fullscreen")
            .unwrap();
        assert!(visual
            .debug_bounds("SHORTCUT_SECTION_zoom-and-window")
            .unwrap()
            .contains(&fullscreen.center()));
        assert!(terminal.contains(
            &visual
                .debug_bounds("SHORTCUT_LOCKED_system-paste")
                .unwrap()
                .center()
        ));
        assert!(visual.debug_bounds("SHORTCUT_LOCKED_close-pane").is_none());

        #[cfg(any(windows, target_os = "linux"))]
        {
            let setting = visual.debug_bounds("SETTINGS_TERMINAL_CTRL_KEYS").unwrap();
            assert!(terminal.contains(&setting.center()));
            host.update(&mut visual, |root, _, cx| {
                root.set_terminal_ctrl_keys(keymap::TerminalCtrlKeys::TerminalFirst, cx)
            })
            .unwrap();
            visual.run_until_parked();
            for selector in [
                "SHORTCUT_OUTSIDE_TERMINALS_new-chat",
                "SHORTCUT_OUTSIDE_TERMINALS_toggle-sidebar",
                "SHORTCUT_OUTSIDE_TERMINALS_switch-sidebar-view",
                "SHORTCUT_OUTSIDE_TERMINALS_open-settings",
                "SHORTCUT_OUTSIDE_TERMINALS_select-tab",
                "SHORTCUT_OUTSIDE_TERMINALS_pane-previous",
                "SHORTCUT_OUTSIDE_TERMINALS_mission-tab-next",
            ] {
                assert!(visual.debug_bounds(selector).is_some(), "{selector}");
            }
            for selector in [
                "SHORTCUT_OUTSIDE_TERMINALS_command-palette",
                "SHORTCUT_OUTSIDE_TERMINALS_split-pane-right",
                "SHORTCUT_OUTSIDE_TERMINALS_zoom-reset",
                "SHORTCUT_OUTSIDE_TERMINALS_new-terminal",
            ] {
                assert!(visual.debug_bounds(selector).is_none(), "{selector}");
            }
            assert_eq!(
                AppSettings::load(&temp.path().join("settings.json"))
                    .unwrap()
                    .terminal_ctrl_keys(),
                keymap::TerminalCtrlKeys::TerminalFirst
            );
            visual.update(|_, cx| {
                let keymap = cx.key_bindings();
                let keymap = keymap.borrow();
                let in_terminal = ["Root", "ChatSplit", "Terminal"]
                    .map(|context| gpui::KeyContext::parse(context).unwrap());
                assert!(keymap
                    .bindings_for_input(&[gpui::Keystroke::parse("ctrl-n").unwrap()], &in_terminal)
                    .0
                    .is_empty());
            });
        }
    });
}

#[test]
#[cfg(target_os = "macos")]
fn fixed_rows_turn_off_and_restore_without_taking_a_custom_key() {
    let tabs = ShortcutRow::TabSelection;
    let close = ShortcutRow::Entry(keymap::entry("close-pane").unwrap());
    let mut overrides = keymap::KeymapOverrides::new();
    overrides.insert("close-pane".into(), None);
    for id in tabs.entry_ids() {
        overrides.insert(id.into(), None);
    }
    assert_eq!(close.binding_label(&overrides), "Unassigned");
    assert_eq!(tabs.binding_label(&overrides), "Unassigned");
    assert!(close.overridden(&overrides) && tabs.overridden(&overrides));

    let mut stale = keymap::KeymapOverrides::new();
    stale.insert(
        "close-pane".into(),
        Some(keymap::entry("new-chat").unwrap().default.clone()),
    );
    stale.insert("select-tab-3".into(), None);
    assert_eq!(close.binding_label(&stale), "⌘W");
    assert_eq!(tabs.binding_label(&stale), "⌘1–⌘9");
    assert!(!close.overridden(&stale) && !tabs.overridden(&stale));
}

#[test]
#[cfg(target_os = "macos")]
fn shortcut_search_matches_grouped_tab_terms() {
    let overrides = keymap::KeymapOverrides::new();
    for query in ["tab 3", "⌘3", "go to tab"] {
        assert_eq!(
            shortcut_rows(query, &overrides),
            [ShortcutRow::TabSelection],
            "{query}"
        );
    }
    assert_eq!(
        shortcut_rows("resume focused", &overrides)
            .iter()
            .map(|row| row.title())
            .collect::<Vec<_>>(),
        ["Resume focused session"]
    );
    assert!(shortcut_rows("definitely missing", &overrides).is_empty());
}

#[test]
fn runner_skill_row_only_reports_exception_states() {
    let healthy = crate::app_store::RunnerSkillStatus {
        detected: true,
        foreign: None,
    };
    assert_eq!(runner_skill_status_line(true, &healthy), None);
    assert_eq!(runner_skill_status_line(false, &healthy), None);

    let missing_agent = crate::app_store::RunnerSkillStatus::default();
    assert_eq!(
        runner_skill_status_line(true, &missing_agent),
        Some((
            "No supported agent detected yet. The skill is added when one is installed.".into(),
            false,
        ))
    );
    assert_eq!(runner_skill_status_line(false, &missing_agent), None);

    let foreign = crate::app_store::RunnerSkillStatus {
        detected: false,
        foreign: Some(PathBuf::from("/tmp/home/.agents/skills/runner-dev")),
    };
    assert_eq!(
        runner_skill_status_line(false, &foreign),
        Some((
            "/tmp/home/.agents/skills/runner-dev was not installed by Runner and is left alone."
                .into(),
            true,
        ))
    );
}

#[test]
fn theme_galleries_list_six_themes_per_mode_with_unique_keys_and_matching_lightness() {
    use crate::theme::{DarkTheme, LightTheme};

    assert_eq!(LightTheme::ALL.len(), 6);
    assert_eq!(DarkTheme::ALL.len(), 6);

    let light_keys: Vec<_> = LightTheme::ALL.into_iter().map(LightTheme::key).collect();
    assert_eq!(
        light_keys,
        [
            "runner-light",
            "catppuccin-latte",
            "rose-pine-dawn",
            "tokyo-night-day",
            "gruvbox-light",
            "solarized-light",
        ]
    );
    let dark_keys: Vec<_> = DarkTheme::ALL.into_iter().map(DarkTheme::key).collect();
    assert_eq!(
        dark_keys,
        [
            "carbon",
            "catppuccin-mocha",
            "rose-pine-moon",
            "tokyo-night",
            "gruvbox-dark",
            "solarized-dark",
        ]
    );
    for light in LightTheme::ALL {
        assert_eq!(LightTheme::parse(light.key()), Some(light));
        assert!(light.variant().is_light());
        assert!(light.variant().terminal_palette().is_light());
    }
    for dark in DarkTheme::ALL {
        assert_eq!(DarkTheme::parse(dark.key()), Some(dark));
        assert!(!dark.variant().is_light());
        assert!(!dark.variant().terminal_palette().is_light());
    }
    assert_eq!(DarkTheme::Runner.label(), "Runner");
    assert_eq!(LightTheme::parse("codex"), None);
    assert_eq!(DarkTheme::parse("runner-dark"), None);
}

#[test]
fn usage_stays_solid_and_settings_gear_opens_usage_in_both_materials() {
    use crate::theme_snapshot::{assert_fill, ThemeGuard};
    use gpui::{size, Modifiers, TestAppContext, VisualTestContext};
    use runner_app::appearance::{Appearance, WindowMaterial};
    use runner_core::protocol::model::Runtime;

    let _theme = ThemeGuard::new();
    for material in [WindowMaterial::Solid, WindowMaterial::Glass] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        cx.update(|cx| {
            cx.set_global(Appearance {
                material,
                reduce_transparency: false,
            })
        });
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            store.settings.window_material = material;
            cx.notify();
        });
        let host = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "usage-settings-gear".into(),
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
        assert_fill(&mut visual, "USAGE_POPOVER_PANEL", theme::colors().panel);
        visual.update(|_, cx| assert_eq!(cx.windows().len(), 1));
        let gear = visual.debug_bounds("USAGE_AGENT_SETTINGS").unwrap();
        visual.simulate_click(gear.center(), Modifiers::default());
        visual.run_until_parked();
        host.read_with(&visual, |root, _| {
            assert!(!root.usage_open);
            assert_eq!(root.route, AppRoute::Settings);
            assert_eq!(root.settings_page.pane, SettingsPane::Usage);
        })
        .unwrap();
        assert!(visual.debug_bounds("USAGE_POPOVER_PANEL").is_none());
    }
}

#[test]
fn usage_preview_keeps_its_natural_size_and_wraps_inside_the_sidebar_card() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let host = cx.add_window(|window, cx| {
        NativeRoot::new(
            "usage-preview-size".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1440.), px(1000.)));
    host.update(&mut visual, |root, window, cx| {
        root.usage_installed = vec![
            runner_core::Runtime::ClaudeCode,
            runner_core::Runtime::Codex,
        ];
        root.enter_settings_pane(SettingsPane::Usage, window, cx);
    })
    .unwrap();
    for (width, zoom, sidebar_width, stacked) in [
        (1440., 1., 240., false),
        (1440., 1.5, 240., false),
        (800., 1., 360., true),
        (800., 1.5, 240., true),
    ] {
        visual.simulate_resize(gpui::size(
            px(width),
            px(if width == 800. { 600. } else { 1000. }),
        ));
        store.update(&mut visual, |store, cx| {
            store.update_settings(
                |s| {
                    s.app_zoom = zoom;
                    s.sidebar_width = sidebar_width;
                    true
                },
                false,
                cx,
            );
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.refresh();
            let _ = window.draw(cx);
        });
        let sample = visual.debug_bounds("USAGE_PREVIEW_SIDEBAR").unwrap();
        let preview = visual.debug_bounds("USAGE_SIDEBAR_PREVIEW").unwrap();
        let label = visual.debug_bounds("USAGE_PREVIEW_LABEL").unwrap();
        let footer = visual.debug_bounds("SIDEBAR_FOOTER_ROW").unwrap();
        let badges = visual.debug_bounds("SIDEBAR_USAGE").unwrap();
        crate::theme_snapshot::assert_fill_color(&mut visual, "SIDEBAR_USAGE", None);
        crate::theme_snapshot::assert_border(
            &mut visual,
            "SIDEBAR_USAGE",
            gpui::transparent_black(),
        );
        let host = visual.debug_bounds("USAGE_PREVIEW_HOST").unwrap();
        let gear = visual.debug_bounds("USAGE_PREVIEW_SETTINGS").unwrap();
        assert!(visual.debug_bounds("USAGE_PREVIEW_RECENT").is_none());
        assert!((sample.size.width - px(240. * zoom)).abs() < px(1.));
        assert!((sample.size.height - px(56. * zoom)).abs() < px(1.));
        assert!((footer.size.height - px(32. * zoom)).abs() < px(1.));
        assert!((footer.left() - sample.left() - px(12. * zoom)).abs() < px(1.));
        assert!((sample.bottom() - footer.bottom() - px(12. * zoom)).abs() < px(1.));
        assert!(sample.left() >= preview.left() && sample.right() <= preview.right());
        if stacked {
            assert!(sample.top() >= label.bottom() + px(24. * zoom - 1.));
        } else {
            assert!((sample.center().y - label.center().y).abs() < px(1.));
            assert!((sample.right() - preview.right() + px(20. * zoom)).abs() < px(1.));
        }
        assert!((gear.left() - host.right() - px(8. * zoom)).abs() < px(1.));
        assert!(
            badges.size.width <= px((super::super::usage::USAGE_PREVIEW_WIDTH - 96.) * zoom + 1.)
        );
        assert!(badges.left() >= footer.left() && gear.right() <= footer.right() + px(1.));
    }
}

#[test]
fn usage_preview_tracks_badges_and_keeps_the_same_footer_anchors() {
    use runner_core::Runtime;

    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "usage-preview-states".into(),
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
    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1200.), px(900.)));
    let mut anchors = None;
    for (sidebar, codex, claude, antigravity) in [
        (true, true, true, true),
        (true, true, true, false),
        (true, true, false, true),
        (true, false, true, false),
        (true, false, false, true),
        (true, false, false, false),
        (false, true, true, true),
    ] {
        store.update(&mut visual, |store, cx| {
            store.update_settings(
                |s| {
                    s.usage.sidebar = sidebar;
                    s.usage.codex = codex;
                    s.usage.claude_code = claude;
                    s.usage.antigravity = antigravity;
                    true
                },
                false,
                cx,
            );
        });
        let mut real_usage_width = None;
        for preview in [false, true] {
            host.update(&mut visual, |root, window, cx| {
                if preview {
                    root.enter_settings_pane(SettingsPane::Usage, window, cx);
                } else {
                    root.route = AppRoute::Roles;
                    cx.notify();
                }
            })
            .unwrap();
            visual.run_until_parked();
            let usage = visual.debug_bounds("SIDEBAR_USAGE");
            assert_eq!(usage.is_some(), sidebar && (codex || claude || antigravity));
            for (selector, enabled) in [
                ("SIDEBAR_USAGE_VALUE_codex", codex),
                ("SIDEBAR_USAGE_VALUE_claude-code", claude),
                ("SIDEBAR_USAGE_VALUE_antigravity", antigravity),
            ] {
                assert_eq!(visual.debug_bounds(selector).is_some(), sidebar && enabled);
            }
            if preview {
                assert_eq!(usage.map(|bounds| bounds.size.width), real_usage_width);
                if sidebar && codex && claude && antigravity {
                    let codex_entry = visual.debug_bounds("SIDEBAR_USAGE_ENTRY_codex").unwrap();
                    let claude_entry = visual
                        .debug_bounds("SIDEBAR_USAGE_ENTRY_claude-code")
                        .unwrap();
                    let antigravity_entry = visual
                        .debug_bounds("SIDEBAR_USAGE_ENTRY_antigravity")
                        .unwrap();
                    assert!((claude_entry.left() - codex_entry.right() - px(8.)).abs() <= px(0.5));
                    assert!(
                        (antigravity_entry.left() - claude_entry.right() - px(8.)).abs() <= px(0.5)
                    );
                }
                let sample = visual.debug_bounds("USAGE_PREVIEW_SIDEBAR").unwrap();
                let host_button = visual.debug_bounds("USAGE_PREVIEW_HOST").unwrap();
                let gear = visual.debug_bounds("USAGE_PREVIEW_SETTINGS").unwrap();
                let offsets = (
                    host_button.left() - sample.left(),
                    gear.left() - sample.left(),
                );
                assert_eq!(*anchors.get_or_insert(offsets), offsets);
            } else {
                real_usage_width = usage.map(|bounds| bounds.size.width);
            }
        }
    }
}

#[test]
fn usage_settings_centers_codex_row_without_subtitle_and_labels_antigravity_gemini_pool() {
    use runner_core::Runtime;

    #[cfg(debug_assertions)]
    {
        crate::i18n::testing::with_locale(crate::i18n::Locale::English, || {
            assert_eq!(
                crate::i18n::text("usage.antigravity_gemini_pool"),
                "Shows the Gemini Models weekly pool."
            );
        });
        crate::i18n::testing::with_locale(crate::i18n::Locale::SimplifiedChinese, || {
            assert_eq!(
                crate::i18n::text("usage.antigravity_gemini_pool"),
                "显示 Gemini 模型的每周模型池。"
            );
        });
    }

    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "usage-badge-rows".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.usage_installed = vec![Runtime::Codex, Runtime::ClaudeCode, Runtime::Antigravity];
        root.enter_settings_pane(SettingsPane::Usage, window, cx);
        root
    });
    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1200.), px(900.)));
    visual.run_until_parked();

    assert!(visual.debug_bounds("USAGE_BADGE_codex_SUBTITLE").is_none());
    assert!(visual
        .debug_bounds("USAGE_BADGE_claude-code_SUBTITLE")
        .is_some());
    assert!(visual
        .debug_bounds("USAGE_BADGE_antigravity_SUBTITLE")
        .is_some());

    let codex_row = visual.debug_bounds("USAGE_BADGE_ROW_codex").unwrap();
    let claude_row = visual.debug_bounds("USAGE_BADGE_ROW_claude-code").unwrap();
    let antigravity_row = visual.debug_bounds("USAGE_BADGE_ROW_antigravity").unwrap();
    assert!(codex_row.size.height < claude_row.size.height);
    assert!(codex_row.size.height < antigravity_row.size.height);

    let codex_label = visual.debug_bounds("USAGE_BADGE_codex_LABEL").unwrap();
    let codex_control = visual.debug_bounds("USAGE_BADGE_codex_CONTROL").unwrap();
    assert!((codex_label.center().y - codex_control.center().y).abs() < px(1.));
}

#[test]
fn usage_preview_buttons_never_open_pickers_or_navigate() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let host = cx.add_window(|window, cx| {
        NativeRoot::new(
            "usage-preview".into(),
            temp.path().join("logs"),
            None,
            None,
            store,
            window,
            cx,
        )
    });
    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1200.), px(900.)));
    host.update(&mut visual, |root, window, cx| {
        root.enter_settings_pane(SettingsPane::Usage, window, cx)
    })
    .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SIDEBAR_HOST_BUTTON").is_none());
    assert!(visual.debug_bounds("SIDEBAR_SETTINGS").is_none());
    let anchor = host
        .read_with(&visual, |root, _| root.remote_client.ui.anchor)
        .unwrap();
    for selector in ["USAGE_PREVIEW_HOST", "USAGE_PREVIEW_SETTINGS"] {
        let button = visual.debug_bounds(selector).unwrap();
        visual.simulate_click(button.center(), gpui::Modifiers::default());
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        host.read_with(&visual, |root, _| {
            assert_eq!(root.settings_page.pane, SettingsPane::Usage);
            assert!(!root.remote_client.ui.picker_open);
            assert_eq!(root.remote_client.ui.anchor, anchor);
        })
        .unwrap();
    }
}

#[test]
fn usage_page_account_picker_follows_refreshed_accounts_and_usage() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        store.claude_accounts = runner_core::protocol::ClaudeAccounts {
            accounts: ["personal", "work"]
                .into_iter()
                .map(|id| runner_core::protocol::ClaudeAccount {
                    id: id.into(),
                    name: id.into(),
                    folder: format!("/tmp/{id}"),
                    native: false,
                })
                .collect(),
            main_id: "personal".into(),
            active_id: "personal".into(),
            statuses: ["personal", "work"]
                .into_iter()
                .map(|id| {
                    (
                        id.into(),
                        runner_core::protocol::ClaudeAuthStatus {
                            logged_in: true,
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            ..Default::default()
        };
    });
    let accounts = store.read_with(&cx, |store, _| store.claude_accounts.clone());
    let host = cx.add_window(|window, cx| {
        NativeRoot::new(
            "usage-picker".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(gpui::size(px(1200.), px(900.)));
    host.update(&mut visual, |root, window, cx| {
        root.enter_settings_pane(SettingsPane::Usage, window, cx)
    })
    .unwrap();
    visual.run_until_parked();
    host.read_with(&visual, |root, cx| {
        assert_eq!(
            root.claude_account_picker
                .as_ref()
                .unwrap()
                .read(cx)
                .value(),
            "personal"
        )
    })
    .unwrap();
    store.update(&mut visual, |store, cx| {
        store.claude_accounts = accounts;
        store.claude_accounts.active_id = "work".into();
        store.revisions.activity = store.revisions.activity.wrapping_add(1);
        store.claude_account_usage.insert(
            "work".into(),
            runner_core::protocol::RuntimeUsage {
                value: Some(runner_core::protocol::AgentUsage {
                    credits: None,
                    updated_at: chrono::Utc::now(),
                    windows: vec![runner_core::protocol::UsageWindow {
                        name: "Week".into(),
                        used_percent: 91.,
                        resets_at: None,
                    }],
                }),
                error: None,
            },
        );
        cx.notify();
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        window.refresh();
        let _ = window.draw(cx);
    });
    host.read_with(&visual, |root, cx| {
        assert_eq!(
            root.app_store.read(cx).claude_accounts.active_id,
            "work",
            "refreshed account snapshot"
        );
        let picker = root.claude_account_picker.as_ref().unwrap().read(cx);
        assert_eq!(picker.value(), "work");
        assert_eq!(
            picker
                .selected()
                .unwrap()
                .description
                .as_ref()
                .unwrap()
                .as_ref(),
            "91%"
        );
    })
    .unwrap();
}

#[test]
fn legacy_catalog_routes_redirect_to_agent_tabs_and_reset_on_other_agent() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = gpui::TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let host = cx.add_window(|window, cx| {
        NativeRoot::new(
            "integration-routes".into(),
            temp.path().join("logs"),
            None,
            None,
            store,
            window,
            cx,
        )
    });
    let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
    for (route, tab) in [
        ("skills", settings::agents::AgentTab::Skills),
        ("mcp", settings::agents::AgentTab::Mcp),
    ] {
        host.update(&mut visual, |root, window, cx| {
            root.enter_settings_pane(SettingsPane::from_route(Some(route)), window, cx);
            assert_eq!(root.settings_page.pane, SettingsPane::Agents);
            let agents = root.settings_page.agents.as_ref().unwrap().clone();
            assert_eq!(agents.read(cx).tab, tab);
            agents.update(cx, |pane, cx| {
                pane.open_runtime(runner_core::Runtime::Codex, cx)
            });
            assert_eq!(agents.read(cx).tab, settings::agents::AgentTab::General);
        })
        .unwrap();
    }
}

#[test]
fn agent_page_keyboard_tabs_from_executable_to_browse_and_back() {
    use gpui::{point, size, KeyUpEvent, Keystroke, Modifiers, TestAppContext, VisualTestContext};

    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let host = cx.add_window(|window, cx| {
        NativeRoot::new(
            "agent-settings-keyboard".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    host.update(&mut visual, |root, window, cx| {
        root.enter_settings_pane(SettingsPane::Agents, window, cx);
    })
    .unwrap();
    visual.run_until_parked();
    let row = visual.debug_bounds("AGENT_CARD_codex").unwrap();
    visual.simulate_click(
        point(row.left() + px(24.), row.center().y),
        Modifiers::default(),
    );
    visual.run_until_parked();
    let executable = visual.debug_bounds("AGENT_EXECUTABLE_codex").unwrap();
    visual.simulate_click(
        point(executable.left() + px(24.), executable.center().y),
        Modifiers::default(),
    );
    visual.run_until_parked();
    let editor = host
        .update(&mut visual, |_, window, cx| window.focused(cx).unwrap())
        .unwrap();
    visual.simulate_keystrokes("tab");
    visual.run_until_parked();
    host.update(&mut visual, |_, window, _| {
        assert!(!editor.is_focused(window))
    })
    .unwrap();
    visual.simulate_keystrokes("space");
    visual.run_until_parked();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    visual.run_until_parked();
    visual.simulate_keystrokes("shift-tab");
    visual.run_until_parked();
    host.update(&mut visual, |_, window, _| {
        assert!(editor.is_focused(window))
    })
    .unwrap();
    let back = visual.debug_bounds("AGENTS_BREADCRUMB").unwrap();
    visual.simulate_click(back.center(), Modifiers::default());
    visual.run_until_parked();
    host.update(&mut visual, |root, window, cx| {
        root.settings_page
            .search
            .read(cx)
            .focus_handle()
            .focus(window, cx);
    })
    .unwrap();
    visual.simulate_keystrokes("shift-tab");
    visual.run_until_parked();
    if visual.debug_bounds("AGENT_TOGGLE_opencode").is_some() {
        visual.simulate_keystrokes("space");
        visual.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse("space").unwrap(),
        });
        visual.run_until_parked();
        store.read_with(&visual, |store, _| {
            assert!(store.settings.disabled_agents.contains("opencode"))
        });
        assert!(visual.debug_bounds("AGENT_CARD_opencode").is_some());
        visual.simulate_keystrokes("shift-tab");
    }
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert!(visual.debug_bounds("AGENT_PAGE_opencode").is_some());
}

#[test]
fn general_quit_menu_renders_a_right_hand_tick_for_each_choice() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use runner_app::lifecycle::QuitBehavior;
    struct QuitSelectHost(Entity<NativeRoot>);
    impl Render for QuitSelectHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .p_6()
                .child(self.0.read(cx).settings_page.quit_behavior.clone())
        }
    }
    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let host = cx.add_window(|window, cx| {
        QuitSelectHost(cx.new(|cx| {
            NativeRoot::new(
                "quit-select".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            )
        }))
    });
    cx.run_until_parked();
    let select = host
        .read_with(&cx, |host, cx| {
            host.0.read(cx).settings_page.quit_behavior.clone()
        })
        .unwrap();
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    for (index, choice) in QuitBehavior::ALL.into_iter().enumerate() {
        select.update(&mut cx, |select, cx| select.set_value(choice.key(), cx));
        cx.run_until_parked();
        let trigger = visual.debug_bounds("STYLED_SELECT_TRIGGER").unwrap();
        visual.simulate_click(trigger.center(), Modifiers::default());
        cx.run_until_parked();
        let row = visual
            .debug_bounds(Box::leak(
                format!("STYLED_SELECT_OPTION_{index}").into_boxed_str(),
            ))
            .unwrap();
        let check = visual.debug_bounds("STYLED_SELECT_CHECK").unwrap();
        assert!(check.center().x > row.center().x);
        assert!(check.left() >= row.left() && check.right() <= row.right());
        assert!((check.center().y - row.center().y).abs() <= px(1.));
        visual.simulate_click(row.center(), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(
            store.read_with(&cx, |store, _| store.settings.quit_behavior),
            choice
        );
    }
}

#[test]
fn appearance_theme_galleries_update_app_and_terminal_together_and_fit_six_cards() {
    use crate::theme::{DarkTheme, LightTheme, ThemeVariant};
    use crate::theme_snapshot::{assert_fill, ThemeGuard};
    use gpui::{size, Modifiers, TestAppContext, VisualTestContext};
    use runner_daemon::{db, session, shell_path};
    use std::sync::{Arc, RwLock};

    fn packed(rgb: alacritty_terminal::vte::ansi::Rgb) -> u32 {
        u32::from(rgb.r) << 16 | u32::from(rgb.g) << 8 | u32::from(rgb.b)
    }

    let _theme = ThemeGuard::new();
    theme::set_active_variant(ThemeVariant::CatppuccinMocha);
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
            AppSettings {
                app_theme: ThemeIntent::Dark,
                light_app_theme: LightTheme::RunnerLight,
                dark_app_theme: DarkTheme::CatppuccinMocha,
                ..AppSettings::default()
            },
            None,
            cx,
        )
    });
    cx.update(|cx| {
        cx.set_global(GlobalAppStore(store.clone()));
        cx.set_global(WindowLayoutCheckpoint::default());
        #[cfg(target_os = "macos")]
        let updater = cx.new(|cx| Updater::new(false, cx));
        #[cfg(any(windows, target_os = "linux"))]
        let updater = cx.new(|cx| Updater::new(false, temp.path().join("updates"), cx));
        cx.set_global(GlobalUpdater(updater));
    });
    let bridge = cx.update(|cx| store.read(cx).bridge.clone());
    let events: Arc<dyn runner_daemon::session::manager::SessionEvents> =
        Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal("term", (80, 24), &core.db, &events)
        .unwrap();
    let terminal = bridge.attach("term").unwrap();
    terminal.test_feed(1, b"ready");

    let host = cx.add_window(|window, cx| {
        window.resize(size(px(1200.), px(900.)));
        let mut root = NativeRoot::new(
            "theme-galleries".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.enter_settings_pane(SettingsPane::Appearance, window, cx);
        root
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(host.into(), &cx);
    window.simulate_resize(size(px(1200.), px(900.)));
    window.run_until_parked();

    assert_eq!(theme::active_variant(), ThemeVariant::CatppuccinMocha);
    assert_eq!(
        bridge.session("term").unwrap().palette(),
        runner_terminal::palette::CATPPUCCIN_MOCHA
    );
    assert!(window
        .debug_bounds("SETTINGS_LIGHT_THEME_GALLERY")
        .is_some());
    assert!(window.debug_bounds("SETTINGS_DARK_THEME_GALLERY").is_some());
    assert!(window.debug_bounds("SETTINGS_APP_PALETTE").is_none());
    assert!(window.debug_bounds("SETTINGS_TERMINAL_COLORS").is_none());
    assert_eq!(
        window.debug_bounds("SETTINGS_WINDOW_MATERIAL").is_some(),
        cfg!(target_os = "macos")
    );

    // Every preview card paints with its own theme's ground and terminal palette.
    for light in LightTheme::ALL {
        let variant = light.variant();
        let preview_sel = static_selector(format!("APPEARANCE_THEME_PREVIEW_{}", light.key()));
        let term_sel = static_selector(format!("APPEARANCE_THEME_TERMINAL_{}", light.key()));
        assert_fill(&mut window, preview_sel, theme::colors_for(variant).bg);
        assert_fill(
            &mut window,
            term_sel,
            packed(variant.terminal_palette().background),
        );
    }
    for dark in DarkTheme::ALL {
        let variant = dark.variant();
        let preview_sel = static_selector(format!("APPEARANCE_THEME_PREVIEW_{}", dark.key()));
        let term_sel = static_selector(format!("APPEARANCE_THEME_TERMINAL_{}", dark.key()));
        assert_fill(&mut window, preview_sel, theme::colors_for(variant).bg);
        assert_fill(
            &mut window,
            term_sel,
            packed(variant.terminal_palette().background),
        );
    }

    // All six cards in each gallery stay in one non-overlapping horizontal row.
    for (gallery_sel, keys) in [
        (
            "SETTINGS_LIGHT_THEME_GALLERY",
            LightTheme::ALL
                .into_iter()
                .map(LightTheme::key)
                .collect::<Vec<_>>(),
        ),
        (
            "SETTINGS_DARK_THEME_GALLERY",
            DarkTheme::ALL
                .into_iter()
                .map(DarkTheme::key)
                .collect::<Vec<_>>(),
        ),
    ] {
        let gallery = window.debug_bounds(gallery_sel).unwrap();
        let mode_tag = if gallery_sel.contains("LIGHT") {
            "LIGHT"
        } else {
            "DARK"
        };
        let mut prev_right = gallery.left();
        for key in &keys {
            let card = window
                .debug_bounds(static_selector(format!(
                    "APPEARANCE_{mode_tag}_THEME_{key}"
                )))
                .unwrap();
            assert!(card.left() >= prev_right, "{key}: {card:?} {prev_right:?}");
            assert!(
                card.right() <= gallery.right(),
                "{key}: {card:?} {gallery:?}"
            );
            let label = window
                .debug_bounds(static_selector(format!("APPEARANCE_THEME_LABEL_{key}")))
                .unwrap();
            assert!(
                label.size.height <= px(22.),
                "label wrapped unexpectedly: {key} {label:?}"
            );
            prev_right = card.right();
        }
    }

    // Only the selected light and dark themes show checks; dark gallery meta says "in use now".
    assert!(window
        .debug_bounds("APPEARANCE_THEME_CHECK_runner-light")
        .is_some());
    assert!(window
        .debug_bounds("APPEARANCE_THEME_CHECK_catppuccin-latte")
        .is_none());
    assert!(window
        .debug_bounds("APPEARANCE_THEME_CHECK_catppuccin-mocha")
        .is_some());
    assert!(window
        .debug_bounds("APPEARANCE_THEME_CHECK_tokyo-night")
        .is_none());

    // Measure label fit for the longest names when selected.
    for (selector, label_selector, check_selector, label_text) in [
        (
            "APPEARANCE_DARK_THEME_catppuccin-mocha",
            "APPEARANCE_THEME_LABEL_catppuccin-mocha",
            "APPEARANCE_THEME_CHECK_catppuccin-mocha",
            "Catppuccin Mocha",
        ),
        (
            "APPEARANCE_LIGHT_THEME_catppuccin-latte",
            "APPEARANCE_THEME_LABEL_catppuccin-latte",
            "APPEARANCE_THEME_CHECK_catppuccin-latte",
            "Catppuccin Latte",
        ),
        (
            "APPEARANCE_LIGHT_THEME_tokyo-night-day",
            "APPEARANCE_THEME_LABEL_tokyo-night-day",
            "APPEARANCE_THEME_CHECK_tokyo-night-day",
            "Tokyo Night Day",
        ),
    ] {
        let card = window.debug_bounds(selector).unwrap();
        window.simulate_click(card.center(), Modifiers::default());
        window.run_until_parked();
        let label_bounds = window.debug_bounds(label_selector).unwrap();
        let check_bounds = window.debug_bounds(check_selector).unwrap();
        assert!(
            label_bounds.right() <= check_bounds.left(),
            "{label_text}: {label_bounds:?} vs {check_bounds:?}"
        );
        let shaped_width = shaped_ui_text_width(&mut window, label_text);
        assert!(
            shaped_width <= label_bounds.size.width,
            "{label_text} truncated at desktop width: needed {shaped_width:?}, got {:?}",
            label_bounds.size.width
        );
    }

    // Picking an inactive light theme stores it without leaving Dark mode.
    let gruvbox_light = window
        .debug_bounds("APPEARANCE_LIGHT_THEME_gruvbox-light")
        .unwrap();
    window.simulate_click(gruvbox_light.center(), Modifiers::default());
    window.run_until_parked();
    assert_eq!(
        store.read_with(&window, |store, _| store.settings.light_app_theme),
        LightTheme::GruvboxLight
    );
    assert_eq!(theme::active_variant(), ThemeVariant::CatppuccinMocha);

    // Picking an active dark theme updates the active variant and terminal palette immediately.
    let tokyo_night = window
        .debug_bounds("APPEARANCE_DARK_THEME_tokyo-night")
        .unwrap();
    window.simulate_click(tokyo_night.center(), Modifiers::default());
    window.run_until_parked();
    assert_eq!(
        store.read_with(&window, |store, _| store.settings.dark_app_theme),
        DarkTheme::TokyoNight
    );
    assert_eq!(theme::active_variant(), ThemeVariant::TokyoNight);
    assert_eq!(
        bridge.session("term").unwrap().palette(),
        runner_terminal::palette::TOKYO_NIGHT
    );

    // Switching mode to Light activates the stored Light theme and its terminal palette.
    store.update(&mut window, |store, cx| {
        store.update_settings(
            |settings| {
                settings.app_theme = ThemeIntent::Light;
                true
            },
            false,
            cx,
        );
    });
    window.run_until_parked();
    assert_eq!(theme::active_variant(), ThemeVariant::GruvboxLight);
    assert_eq!(
        bridge.session("term").unwrap().palette(),
        runner_terminal::palette::GRUVBOX_LIGHT
    );
    assert!(window.debug_bounds("SETTINGS_LIGHT_THEME_META").is_some());
    assert!(window.debug_bounds("SETTINGS_DARK_THEME_META").is_some());
    assert_eq!(
        crate::i18n::tf(
            "format.theme_in_use",
            &[(
                "meta",
                crate::i18n::text("ui.used_when_the_app_is_light").to_string(),
            )],
        ),
        "Used when the app is light · in use now"
    );
    crate::i18n::testing::with_locale(crate::i18n::Locale::SimplifiedChinese, || {
        assert_eq!(
            crate::i18n::tf(
                "format.theme_in_use",
                &[(
                    "meta",
                    crate::i18n::text("ui.used_when_the_app_is_light").to_string(),
                )],
            ),
            "浅色模式下使用 · 当前使用"
        );
    });

    // Narrow windows and zoomed viewports fold into 3×2 rows without wrapping or truncating labels.
    for (win_w, win_h, zoom) in [(960., 700., 1.0_f32), (1440., 900., 1.3_f32)] {
        window.simulate_resize(size(px(win_w), px(win_h)));
        host.update(&mut window, |root, window, cx| {
            root.set_zoom(zoom, window, cx);
        })
        .unwrap();
        window.run_until_parked();

        for (gallery_sel, mode_tag, keys) in [
            (
                "SETTINGS_LIGHT_THEME_GALLERY",
                "LIGHT",
                LightTheme::ALL
                    .into_iter()
                    .map(|choice| (choice.key(), choice.label()))
                    .collect::<Vec<_>>(),
            ),
            (
                "SETTINGS_DARK_THEME_GALLERY",
                "DARK",
                DarkTheme::ALL
                    .into_iter()
                    .map(|choice| (choice.key(), choice.label()))
                    .collect::<Vec<_>>(),
            ),
        ] {
            let gallery = window.debug_bounds(gallery_sel).unwrap();
            let row0_top = window
                .debug_bounds(static_selector(format!(
                    "APPEARANCE_{mode_tag}_THEME_{}",
                    keys[0].0
                )))
                .unwrap()
                .top();
            let row0_bottom = window
                .debug_bounds(static_selector(format!(
                    "APPEARANCE_{mode_tag}_THEME_{}",
                    keys[0].0
                )))
                .unwrap()
                .bottom();
            let row1_top = window
                .debug_bounds(static_selector(format!(
                    "APPEARANCE_{mode_tag}_THEME_{}",
                    keys[3].0
                )))
                .unwrap()
                .top();
            assert!(
                row1_top >= row0_bottom,
                "{win_w}x{win_h}@{zoom} {mode_tag}: {row1_top:?} < {row0_bottom:?}"
            );
            for (row_offset, chunk) in keys.chunks(3).enumerate() {
                let mut prev_right = gallery.left();
                let expected_top = if row_offset == 0 { row0_top } else { row1_top };
                for (key, label_text) in chunk {
                    let card = window
                        .debug_bounds(static_selector(format!(
                            "APPEARANCE_{mode_tag}_THEME_{key}"
                        )))
                        .unwrap();
                    assert!(card.left() >= prev_right, "{key}: {card:?} {prev_right:?}");
                    assert!(
                        card.right() <= gallery.right(),
                        "{key}: {card:?} {gallery:?}"
                    );
                    assert!(
                        (f32::from(card.top()) - f32::from(expected_top)).abs() < 1.5,
                        "{key}: top {:?} vs {expected_top:?}",
                        card.top()
                    );
                    prev_right = card.right();

                    let label_bounds = window
                        .debug_bounds(static_selector(format!("APPEARANCE_THEME_LABEL_{key}")))
                        .unwrap();
                    assert!(
                        label_bounds.size.height <= px(22. * zoom),
                        "label wrapped at {win_w}x{win_h}@{zoom}: {key} {label_bounds:?}"
                    );
                    let shaped_width = shaped_ui_text_width(&mut window, label_text);
                    assert!(
                        shaped_width <= label_bounds.size.width,
                        "{label_text} truncated at {win_w}x{win_h}@{zoom}: needed {shaped_width:?}, got {:?}",
                        label_bounds.size.width
                    );
                }
            }
        }
    }
    window.simulate_resize(size(px(1200.), px(900.)));
    host.update(&mut window, |root, window, cx| {
        root.set_zoom(1.0, window, cx);
    })
    .unwrap();
    window.run_until_parked();

    // Terminal settings no longer has the Colors section.
    host.update(&mut window, |root, window, cx| {
        root.enter_settings_pane(SettingsPane::Terminal, window, cx);
    })
    .unwrap();
    window.run_until_parked();
    assert!(window
        .debug_bounds("SETTINGS_LIGHT_THEME_GALLERY")
        .is_none());
    assert!(window.debug_bounds("SETTINGS_DARK_THEME_GALLERY").is_none());
    assert!(window.debug_bounds("SETTINGS_APP_PALETTE").is_none());
    assert!(window.debug_bounds("SETTINGS_TERMINAL_COLORS").is_none());
    let sections = [
        "SETTINGS_TERMINAL_TEXT",
        "SETTINGS_TERMINAL_CURSOR",
        "SETTINGS_TERMINAL_KEYBOARD",
        "SETTINGS_TERMINAL_SCROLLBACK",
    ];
    let tops = sections
        .iter()
        .filter_map(|selector| window.debug_bounds(selector).map(|bounds| bounds.top()))
        .collect::<Vec<_>>();
    assert_eq!(
        window.debug_bounds("SETTINGS_TERMINAL_KEYBOARD").is_some(),
        cfg!(target_os = "macos")
    );
    assert_eq!(tops.len(), if cfg!(target_os = "macos") { 4 } else { 3 });
    assert!(tops.is_sorted(), "{tops:?}");
    assert!(window.debug_bounds("SETTINGS_OPTION_AS_META").is_some() == cfg!(target_os = "macos"));
    let pane = window.debug_bounds("SETTINGS_TERMINAL_PANE").unwrap();
    let font = window.debug_bounds("SETTINGS_TERMINAL_FONT").unwrap();
    assert!(
        font.top() >= pane.top() && font.bottom() <= pane.bottom(),
        "{pane:?} {font:?}"
    );
}

#[test]
fn settings_routes_fall_back_to_general() {
    assert_eq!(SettingsPane::from_route(None), SettingsPane::General);
    assert_eq!(
        SettingsPane::from_route(Some("account")),
        SettingsPane::Account
    );
    assert_eq!(SettingsPane::Account.key(), "account");
    assert_eq!(SettingsPane::Account.icon(), "user-round.svg");
    assert_eq!(
        SettingsPane::from_route(Some("appearance")),
        SettingsPane::Appearance
    );
    assert_eq!(
        SettingsPane::from_route(Some("not-a-pane")),
        SettingsPane::General
    );
    assert_eq!(
        SettingsPane::from_route(Some("missions")),
        SettingsPane::Missions
    );
    assert_eq!(
        SettingsPane::from_route(Some("notifications")),
        SettingsPane::Notifications
    );
    assert_eq!(SettingsPane::Notifications.key(), "notifications");
    assert_eq!(SettingsPane::Notifications.icon(), "bell.svg");
    assert_eq!(SettingsPane::Missions.key(), "missions");
    assert_eq!(SettingsPane::Missions.label(), "Missions");
    assert_eq!(SettingsPane::Missions.icon(), "rocket.svg");
}

#[test]
fn nav_search_filters_labels_and_removes_empty_groups() {
    crate::i18n::testing::with_locale(crate::i18n::Locale::English, || {
        let all = filtered_nav_groups("");
        assert_eq!(
            all.iter().map(|(label, _)| *label).collect::<Vec<_>>(),
            ["App", "Side panel", "Integrations", "Cloud", "System"]
        );
        assert_eq!(all.iter().map(|(_, panes)| panes.len()).sum::<usize>(), 15);
        assert_eq!(
            all[0],
            (
                "App",
                vec![
                    SettingsPane::General,
                    SettingsPane::Appearance,
                    SettingsPane::Notifications,
                    SettingsPane::Missions,
                    SettingsPane::Terminal,
                    SettingsPane::Shortcuts,
                    SettingsPane::Archived,
                ],
            )
        );

        assert_eq!(
            all[1],
            (
                "Side panel",
                vec![SettingsPane::SidePanel, SettingsPane::Files]
            )
        );
        assert_eq!(
            filtered_nav_groups("files"),
            vec![("Side panel", vec![SettingsPane::Files])]
        );
        assert_eq!(
            SettingsPane::from_route(Some("side-panel")),
            SettingsPane::SidePanel
        );
        assert_eq!(SettingsPane::from_route(Some("files")), SettingsPane::Files);
        assert_eq!(
            all[2],
            (
                "Integrations",
                vec![SettingsPane::Agents, SettingsPane::Usage]
            )
        );
        assert_eq!(
            all[3],
            ("Cloud", vec![SettingsPane::Account, SettingsPane::Remote])
        );
        assert_eq!(
            filtered_nav_groups("remote"),
            vec![("Cloud", vec![SettingsPane::Remote])]
        );
        assert_eq!(
            SettingsPane::from_route(Some("skills")),
            SettingsPane::Skills
        );
        assert_eq!(SettingsPane::from_route(Some("mcp")), SettingsPane::Mcp);
        assert_eq!(
            filtered_nav_groups("mcp"),
            vec![("Integrations", vec![SettingsPane::Mcp])]
        );
        assert_eq!(
            filtered_nav_groups("skills"),
            vec![("Integrations", vec![SettingsPane::Skills])]
        );

        let terminal = filtered_nav_groups("  TERM  ");
        assert_eq!(terminal, vec![("App", vec![SettingsPane::Terminal])]);
        for row in ["line height", "scrollback", "copy on select", "blink"] {
            assert_eq!(
                filtered_nav_groups(row),
                vec![("App", vec![SettingsPane::Terminal])],
                "{row}"
            );
        }
        for row in ["light theme", "dark theme"] {
            assert_eq!(
                filtered_nav_groups(row),
                vec![("App", vec![SettingsPane::Appearance])],
                "{row}"
            );
        }
        assert!(filtered_nav_groups("palette").is_empty());
        assert_eq!(
            filtered_nav_groups("option as meta").is_empty(),
            !cfg!(target_os = "macos")
        );

        let archived = filtered_nav_groups("archived");
        assert_eq!(archived, vec![("App", vec![SettingsPane::Archived])]);
        assert!(filtered_nav_groups("no such setting").is_empty());
        assert_eq!(
            filtered_nav_groups("updates"),
            vec![("System", vec![SettingsPane::Updates])]
        );
        assert_eq!(
            SettingsPane::from_route(Some("updates")),
            SettingsPane::Updates
        );
    });
}

#[test]
fn missions_pane_renders_default_crew_at_two_rem_sizes() {
    use gpui::{size, Render, TestAppContext, VisualTestContext};

    struct PaneHost {
        default_crew: Entity<StyledSelect>,
    }
    impl Render for PaneHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .p_6()
                .child(missions_settings_pane(self.default_crew.clone()))
        }
    }

    let mut cx = TestAppContext::single();
    let host = cx.add_window(|window, cx| {
        window.resize(size(px(1200.), px(900.)));
        let noop: SelectHandler = Rc::new(|_, _, _| {});
        let default_crew = cx.new(|cx| {
            StyledSelect::new(
                "settings-default-crew",
                cx.focus_handle(),
                "",
                vec![SelectOption::new("", "No default")],
                noop.clone(),
                cx,
            )
        });
        PaneHost { default_crew }
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(host.into(), &cx);
    for rem in [16., 20.8] {
        host.update(&mut window, |_, window, _| window.set_rem_size(px(rem)))
            .unwrap();
        window.run_until_parked();
        let pane = window.debug_bounds("SETTINGS_MISSIONS_PANE").unwrap();
        let heading = window
            .debug_bounds("SETTINGS_MISSIONS_DEFAULTS_HEADING")
            .unwrap();
        let card = window.debug_bounds("SETTINGS_MISSIONS_CARD").unwrap();
        let crew = window.debug_bounds("SETTINGS_DEFAULT_CREW").unwrap();
        assert!(window
            .debug_bounds("SETTINGS_MISSION_PERMISSIONS")
            .is_none());
        assert!(
            heading.size.height >= px(rem) && heading.bottom() <= card.top(),
            "{rem}: {heading:?} {card:?}"
        );
        assert!(
            card.size.width > px(0.) && card.size.width <= pane.size.width,
            "{rem}: {card:?} {pane:?}"
        );
        assert!(
            crew.bottom() <= card.bottom() && crew.right() <= card.right(),
            "{rem}: {crew:?} {card:?}"
        );
    }
}

#[test]
fn content_scrollbar_starts_below_the_drag_strip_at_the_window_edge() {
    use gpui::{point, size, Modifiers, Pixels, Point, Render, TestAppContext, VisualTestContext};

    use crate::app_settings::SIDEBAR_DEFAULT;

    const HEIGHT: f32 = 320.;
    // ScrollbarKind::App's track width, private to ui/scrollbar.rs.
    const GUTTER: f32 = 10.;

    struct ColumnHost {
        default_crew: Entity<StyledSelect>,
        content_scroll: ScrollHandle,
        content_scrollbar: Entity<Scrollbar>,
    }
    impl Render for ColumnHost {
        fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let zoom = f32::from(window.rem_size()) / 16.;
            div()
                .size_full()
                .flex()
                .child(div().flex_none().h_full().w(px(SIDEBAR_DEFAULT * zoom)))
                .child(settings_content_column(
                    (SETTINGS_DRAG_INSET > 0.).then(|| {
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h(px(TITLEBAR_DRAG_HEIGHT * zoom))
                    }),
                    Some(
                        div()
                            .flex()
                            .flex_col()
                            .child(missions_settings_pane(self.default_crew.clone()))
                            .child(div().h(px(400. * zoom)))
                            .into_any_element(),
                    ),
                    &self.content_scroll,
                    self.content_scrollbar.clone(),
                    zoom,
                    px(40. * zoom),
                    SETTINGS_CONTENT_MAX_WIDTH,
                ))
        }
    }

    let mut cx = TestAppContext::single();
    let host = cx.add_window(|_, cx| {
        let noop: SelectHandler = Rc::new(|_, _, _| {});
        let default_crew = cx.new(|cx| {
            StyledSelect::new(
                "settings-default-crew",
                cx.focus_handle(),
                "",
                vec![SelectOption::new("", "No default")],
                noop.clone(),
                cx,
            )
        });
        let content_scroll = ScrollHandle::new();
        let owner = cx.entity_id();
        let content_scrollbar = cx.new(|_| Scrollbar::app(content_scroll.clone(), owner));
        ColumnHost {
            default_crew,
            content_scroll,
            content_scrollbar,
        }
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(host.into(), &cx);
    let scroll = host
        .update(&mut window, |host, _, _| host.content_scroll.clone())
        .unwrap();
    let near = |a: Pixels, b: Pixels| (f32::from(a) - f32::from(b)).abs() <= 0.5;

    for width in [1000., 2000.] {
        for rem in [16., 20.8] {
            let zoom = rem / 16.;
            let track_top = px(SETTINGS_DRAG_INSET * zoom);
            let content_top = px(SETTINGS_CONTENT_TOP * zoom);
            window.simulate_resize(size(px(width), px(HEIGHT)));
            scroll.set_offset(point(px(0.), px(0.)));
            host.update(&mut window, |_, window, _| {
                window.set_rem_size(px(rem));
                window.refresh();
            })
            .unwrap();
            window.run_until_parked();

            let track = window
                .debug_bounds("SETTINGS_CONTENT_SCROLLBAR_TRACK")
                .unwrap();
            let label = format!("{width}x{rem}: track {track:?}");

            assert!(near(track.right(), px(width)), "{label}");
            assert!(near(track.left(), px(SIDEBAR_DEFAULT * zoom)), "{label}");
            assert!(near(track.top(), track_top), "{label}");
            assert!(
                near(track.bottom(), px(HEIGHT - TITLEBAR_DRAG_HEIGHT * zoom)),
                "{label}"
            );
            let pane = window.debug_bounds("SETTINGS_MISSIONS_PANE").unwrap();
            assert!(
                near(pane.top(), content_top),
                "{label}: pane {pane:?} content {content_top:?}"
            );

            let hit_strip = window.debug_bounds("SCROLLBAR_TRACK").unwrap();
            assert!(near(hit_strip.left(), px(width - GUTTER * zoom)), "{label}");
            assert!(near(hit_strip.right(), px(width)), "{label}");
            assert!(near(hit_strip.top(), track.top()), "{label}");
            assert!(near(hit_strip.bottom(), track.bottom()), "{label}");

            // Hover reveals the track before probing its clicks.
            let max = scroll.max_offset().y;
            assert!(max > px(0.), "{label}: the pane should overflow");
            let inside_left = px(width - GUTTER * zoom + 1.);
            let outside_left = px(width - GUTTER * zoom - 1.);
            let inside_right = px(width - 1.);
            let below_top = track.top() + px(1.);
            let bottom = track.bottom() - px(1.);
            let below_bottom = track.bottom() + px(1.);
            let mut click_from = |from: Pixels, at: Point<Pixels>| {
                scroll.set_offset(point(px(0.), from));
                host.update(&mut window, |_, window, _| window.refresh())
                    .unwrap();
                window.run_until_parked();
                window.simulate_mouse_move(at, None, Modifiers::default());
                window.simulate_click(at, Modifiers::default());
                scroll.offset().y
            };
            assert!(
                near(click_from(-max, point(inside_left, below_top)), px(0.)),
                "{label}: a click on the gutter's left edge should jump to the top"
            );
            assert!(
                near(click_from(-max, point(inside_right, below_top)), px(0.)),
                "{label}: a click on the gutter's right edge should jump to the top"
            );
            assert!(
                near(click_from(px(0.), point(inside_right, bottom)), -max),
                "{label}: a click at the track's bottom should jump to the bottom"
            );
            assert!(
                near(click_from(-max, point(outside_left, below_top)), -max),
                "{label}: a click left of the gutter should not scroll"
            );
            if SETTINGS_DRAG_INSET > 0. {
                assert!(
                    near(
                        click_from(-max, point(inside_right, track.top() - px(1.))),
                        -max
                    ),
                    "{label}: a click in the drag strip should not scroll"
                );
            }
            assert!(
                near(
                    click_from(px(0.), point(inside_right, below_bottom)),
                    px(0.)
                ),
                "{label}: a click below the track should not scroll"
            );
        }
    }
}

#[test]
fn settings_back_button_clears_window_navigation() {
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{size, TestAppContext, VisualTestContext};

    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let window = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "settings-back".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = AppRoute::Settings;
        root
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    for zoom in [1.0, 1.3] {
        for width in [1440., 2400.] {
            visual.simulate_resize(size(px(width), px(900.)));
            window
                .update(&mut cx, |root, window, cx| {
                    root.set_zoom(zoom, window, cx);
                })
                .unwrap();
            cx.run_until_parked();
            let back = visual.debug_bounds("SETTINGS_BACK").unwrap();
            let sidebar_top = if cfg!(any(windows, target_os = "linux")) {
                12. + 32.
            } else {
                52.
            };
            assert!(
                back.top()
                    >= visual
                        .debug_bounds("WINDOW_TITLEBAR_CONTROLS")
                        .unwrap()
                        .bottom()
                    && back.top() >= px(sidebar_top * zoom)
                    && back.top() < px((sidebar_top + 16.) * zoom),
                "zoom {zoom} width {width}: {back:?}"
            );
        }
    }
}

#[test]
fn launch_dims_prefer_direct_layouts_and_fall_back_to_mission_geometry() {
    let direct = HashMap::from([("direct".to_owned(), (132, 38))]);

    assert_eq!(
        launch_dims_for("direct", &direct, (104, 42)),
        Some((132, 38))
    );
    assert_eq!(
        launch_dims_for("mission", &direct, (104, 42)),
        Some((104, 42))
    );
}

#[test]
fn appearance_keyboard_tabs_between_theme_cards_and_applies_selection() {
    use crate::theme::{DarkTheme, ThemeVariant};
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{size, Modifiers, ScrollWheelEvent, TestAppContext, VisualTestContext};

    let _theme = ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let host = cx.add_window(|window, cx| {
        NativeRoot::new(
            "appearance-settings-keyboard".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(960.), px(700.)));
    host.update(&mut visual, |root, window, cx| {
        root.enter_settings_pane(SettingsPane::Appearance, window, cx);
        root.set_theme_intent(ThemeIntent::Dark, cx);
        root.set_dark_theme(DarkTheme::Runner, cx);
        window.refresh();
    })
    .unwrap();
    visual.run_until_parked();

    let mut catppuccin = visual
        .debug_bounds("APPEARANCE_DARK_THEME_catppuccin-mocha")
        .unwrap();
    if catppuccin.bottom() > px(680.) {
        let scroll_target = visual
            .debug_bounds("SETTINGS_LIGHT_THEME_GALLERY")
            .unwrap()
            .center();
        visual.simulate_event(ScrollWheelEvent {
            position: scroll_target,
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-320.))),
            ..Default::default()
        });
        visual.run_until_parked();
        catppuccin = visual
            .debug_bounds("APPEARANCE_DARK_THEME_catppuccin-mocha")
            .unwrap();
    }

    visual.simulate_click(catppuccin.center(), Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        visual.read(|cx| store.read(cx).settings.dark_app_theme),
        DarkTheme::CatppuccinMocha
    );
    let catppuccin_focus = host
        .update(&mut visual, |_, window, cx| window.focused(cx).unwrap())
        .unwrap();

    visual.simulate_keystrokes("ctrl-tab");
    visual.run_until_parked();
    assert_eq!(
        host.update(&mut visual, |_, window, cx| window.focused(cx))
            .unwrap(),
        Some(catppuccin_focus.clone())
    );

    visual.simulate_keystrokes("tab");
    visual.run_until_parked();
    let rose_pine_focus = host
        .update(&mut visual, |_, window, cx| window.focused(cx).unwrap())
        .unwrap();
    assert_ne!(rose_pine_focus, catppuccin_focus);

    visual.simulate_keystrokes("space");
    visual.run_until_parked();
    assert_eq!(
        visual.read(|cx| store.read(cx).settings.dark_app_theme),
        DarkTheme::RosePineMoon
    );
    assert_eq!(theme::active_variant(), ThemeVariant::RosePineMoon);
    assert!(visual
        .debug_bounds("APPEARANCE_THEME_CHECK_rose-pine-moon")
        .is_some());
    assert!(visual
        .debug_bounds("APPEARANCE_THEME_CHECK_catppuccin-mocha")
        .is_none());

    visual.simulate_keystrokes("tab");
    visual.run_until_parked();
    let tokyo_focus = host
        .update(&mut visual, |_, window, cx| window.focused(cx).unwrap())
        .unwrap();
    assert_ne!(tokyo_focus, rose_pine_focus);

    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert_eq!(
        visual.read(|cx| store.read(cx).settings.dark_app_theme),
        DarkTheme::TokyoNight
    );
    assert_eq!(theme::active_variant(), ThemeVariant::TokyoNight);

    visual.simulate_keystrokes("shift-tab");
    visual.run_until_parked();
    assert_eq!(
        host.update(&mut visual, |_, window, cx| window.focused(cx))
            .unwrap(),
        Some(rose_pine_focus)
    );

    visual.simulate_keystrokes("space");
    visual.run_until_parked();
    assert_eq!(
        visual.read(|cx| store.read(cx).settings.dark_app_theme),
        DarkTheme::RosePineMoon
    );
    assert_eq!(theme::active_variant(), ThemeVariant::RosePineMoon);
}

#[test]
fn theme_card_focus_style_occludes_shadow_with_opaque_panel_background() {
    use crate::theme::ThemeVariant;
    use crate::theme_snapshot::ThemeGuard;

    let _theme = ThemeGuard::new();
    for variant in ThemeVariant::ALL {
        theme::set_active_variant(variant);
        let focused = appearance::theme_card_focus_style(Default::default());
        assert_eq!(focused.background, Some(gpui::Fill::from(theme::panel())));
        assert_eq!(theme::panel().a, 1.0);
        assert_ne!(theme::panel(), theme::accent());
        assert!(focused.box_shadow.is_some());
    }
}

fn static_selector(selector: String) -> &'static str {
    Box::leak(selector.into_boxed_str())
}

fn shaped_ui_text_width(window: &mut gpui::VisualTestContext, text: &str) -> gpui::Pixels {
    let owned = text.to_owned();
    window.update(|window, _| {
        let style = window.text_style();
        let font_size = theme::text_ui().to_pixels(window.rem_size());
        window
            .text_system()
            .shape_line(
                owned.into(),
                font_size,
                &[gpui::TextRun {
                    len: text.len(),
                    font: style.font(),
                    color: theme::text(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
            .width
    })
}
