use super::*;
use gpui::{size, TestAppContext};
use i18n::{testing::with_locale, Language, Locale};

#[test]
fn scrollback_select_follows_shared_settings_and_keeps_a_saved_value_choosable() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    with_locale(Locale::English, || {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, _| {
            store.settings.terminal_scrollback_lines = 7_000
        });
        let windows = ["scrollback-one", "scrollback-two"].map(|label| {
            cx.add_window(|window, cx| {
                NativeRoot::new(
                    label.into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                )
            })
        });
        cx.run_until_parked();
        let choose = |cx: &mut TestAppContext, value: &str| {
            windows[0]
                .update(cx, |root, window, cx| {
                    root.apply_settings_selection(
                        SettingsSelection::TerminalScrollback,
                        value,
                        window,
                        cx,
                    )
                })
                .unwrap();
            cx.run_until_parked();
        };
        let shown = |cx: &mut TestAppContext| {
            windows.map(|window| {
                window
                    .read_with(cx, |root, cx| {
                        let select = root.settings_page.terminal_scrollback.read(cx);
                        (
                            root.settings(cx).terminal_scrollback_lines,
                            select.value().to_owned(),
                            select.selected().unwrap().label.to_string(),
                        )
                    })
                    .unwrap()
            })
        };
        let both = |lines: usize, label: &str| {
            let row = (lines, lines.to_string(), label.to_owned());
            [row.clone(), row]
        };

        assert_eq!(shown(&mut cx), both(7_000, "7,000 lines"));
        choose(&mut cx, "5000");
        assert_eq!(shown(&mut cx), both(5_000, "5,000 lines"));
        choose(&mut cx, "7000");
        assert_eq!(shown(&mut cx), both(7_000, "7,000 lines"));
        choose(&mut cx, "50000");
        assert_eq!(shown(&mut cx), both(50_000, "50,000 lines"));
        choose(&mut cx, "60000");
        assert_eq!(shown(&mut cx), both(50_000, "50,000 lines"));
    });
}

#[test]
fn language_switch_refreshes_two_windows_menus_and_cached_choices_without_resetting_drafts() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    with_locale(Locale::English, || {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, _| {
            store.settings.language = Language::English
        });
        let windows = ["locale-one", "locale-two"].map(|label| {
            cx.add_window(|window, cx| {
                window.resize(size(px(1200.), px(900.)));
                let mut root = NativeRoot::new(
                    label.into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                );
                root.route = AppRoute::Settings;
                root
            })
        });
        cx.run_until_parked();
        let draft = windows[1]
            .read_with(&cx, |root, _| {
                root.settings_page.default_working_dir.clone()
            })
            .unwrap();
        draft.update(&mut cx, |field, cx| {
            field.set_text("/tmp/中文 {count} draft", cx);
            field.select_all(cx);
        });
        cx.run_until_parked();
        let caret = draft.read_with(&cx, |field, _| field.caret_offset());
        let before = store.read_with(&cx, |store, _| store.revisions);
        for (language, expected, menu) in [
            (Language::SimplifiedChinese, "每次询问", "文件"),
            (Language::English, "Ask", "File"),
        ] {
            windows[0]
                .update(&mut cx, |root, window, cx| {
                    root.apply_settings_selection(
                        SettingsSelection::Language,
                        language.key(),
                        window,
                        cx,
                    )
                })
                .unwrap();
            cx.run_until_parked();
            for window in windows {
                window
                    .read_with(&cx, |root, cx| {
                        assert_eq!(root.settings(cx).language, language);
                        assert_eq!(root.settings_page.language.read(cx).value(), language.key());
                        assert_eq!(root.store_revisions.locale, store.read(cx).revisions.locale);
                        assert_eq!(
                            root.settings_page
                                .quit_behavior
                                .read(cx)
                                .selected()
                                .unwrap()
                                .label,
                            expected
                        );
                    })
                    .unwrap();
            }
            let menus = cx.update(|cx| cx.get_menus().unwrap());
            assert_eq!(menus[1].name, menu);
            assert!(
                matches!(&menus[1].items[0], gpui::OwnedMenuItem::Action { name, .. } if name == if language == Language::English { "New Window" } else { "新建窗口" })
            );
            #[cfg(target_os = "macos")]
            assert!(
                matches!(&menus[0].items[0], gpui::OwnedMenuItem::Action { name, .. } if name == if language == Language::English { "About Runner" } else { "关于 Runner" })
            );
            draft.read_with(&cx, |field, _| {
                assert_eq!(field.text(), "/tmp/中文 {count} draft");
                assert_eq!(field.caret_offset(), caret);
            });
            let same_draft = windows[1]
                .read_with(&cx, |root, _| {
                    root.settings_page.default_working_dir.clone()
                })
                .unwrap();
            assert_eq!(draft.entity_id(), same_draft.entity_id());
        }
        let after = store.read_with(&cx, |store, _| store.revisions);
        assert_eq!(after.locale, before.locale + 2);
        assert_eq!(after.terminal_settings, before.terminal_settings);
        assert_eq!(after.role_surfaces, before.role_surfaces);
        assert_eq!(after.crews, before.crews);
    });
}

#[test]
fn general_language_dropdown_persists_keyboard_choices_across_windows() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    with_locale(Locale::English, || {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let windows = ["language-select-one", "language-select-two"].map(|label| {
            cx.add_window(|window, cx| {
                window.resize(size(px(1200.), px(900.)));
                let mut root = NativeRoot::new(
                    label.into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                );
                root.route = AppRoute::Settings;
                root
            })
        });
        cx.run_until_parked();
        let select = windows[0]
            .read_with(&cx, |root, _| root.settings_page.language.clone())
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(windows[0].into(), &cx);
        for (keys, language, expected) in [
            ("space end enter", Language::SimplifiedChinese, "简体中文"),
            ("space home down enter", Language::English, "English"),
            ("space home enter", Language::System, "System"),
        ] {
            windows[0]
                .update(&mut visual, |_, window, cx| {
                    select.read(cx).focus_handle().focus(window, cx)
                })
                .unwrap();
            visual.simulate_keystrokes(keys);
            visual.run_until_parked();
            for window in windows {
                window
                    .read_with(&visual, |root, cx| {
                        let selected = root.settings_page.language.read(cx);
                        assert_eq!(selected.value(), language.key());
                        assert_eq!(selected.selected().unwrap().label, expected);
                        assert_eq!(root.settings(cx).language, language);
                    })
                    .unwrap();
            }
            assert_eq!(
                AppSettings::load(&temp.path().join("settings.json"))
                    .unwrap()
                    .language,
                language
            );
            assert!(visual.debug_bounds("SETTINGS_LANGUAGE_HEADING").is_none());
            let control = visual.debug_bounds("SETTINGS_LANGUAGE_SELECT").unwrap();
            assert_eq!(control.size.width, SETTINGS_SELECT_WIDTH);
            windows[0]
                .read_with(&visual, |root, _| {
                    assert_eq!(root.settings_page.language.entity_id(), select.entity_id())
                })
                .unwrap();
        }
        store.update(&mut visual, |store, cx| {
            store.sync_locale_from(Some("zh-Hans"), cx)
        });
        visual.run_until_parked();
        select.read_with(&visual, |select, _| {
            assert_eq!(select.value(), "system");
            assert_eq!(select.selected().unwrap().label, "跟随系统");
        });
        windows[0]
            .update(&mut visual, |_, window, cx| {
                select.read(cx).focus_handle().focus(window, cx)
            })
            .unwrap();
        visual.simulate_keystrokes("space home enter");
        visual.run_until_parked();
        assert_eq!(
            store.read_with(&visual, |store, _| store.resolved_locale),
            Locale::English
        );
        select.read_with(&visual, |select, _| {
            assert_eq!(select.value(), "system");
            assert_eq!(select.selected().unwrap().label, "System");
        });
    });
}

#[test]
fn language_row_keeps_readable_text_at_narrow_widths_and_enlarged_zoom() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    with_locale(Locale::English, || {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let host = cx.add_window(|window, cx| {
            window.resize(size(px(800.), px(600.)));
            let mut root = NativeRoot::new(
                "language-layout".into(),
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
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        for intent in [ThemeIntent::Dark, ThemeIntent::Light] {
            for language in [Language::English, Language::SimplifiedChinese] {
                for width in [800., 1200.] {
                    for zoom in [1., 1.3] {
                        visual.simulate_resize(size(px(width), px(600.)));
                        host.update(&mut visual, |root, window, cx| {
                            root.set_theme_intent(intent, cx);
                            root.set_zoom(zoom, window, cx);
                            root.apply_settings_selection(
                                SettingsSelection::Language,
                                language.key(),
                                window,
                                cx,
                            );
                        })
                        .unwrap();
                        visual.run_until_parked();
                        let row = visual.debug_bounds("SETTINGS_WRAPPED_ROW").unwrap();
                        let text = visual.debug_bounds("SETTINGS_WRAPPED_ROW_TEXT").unwrap();
                        let description = visual
                            .debug_bounds("SETTINGS_WRAPPED_ROW_DESCRIPTION")
                            .unwrap();
                        let control = visual.debug_bounds("SETTINGS_LANGUAGE_SELECT").unwrap();
                        let case = format!("{intent:?}/{language:?}/{width}/{zoom}");
                        assert!(
                            description.size.width >= px(200. * zoom),
                            "{case}: {description:?}"
                        );
                        assert!(
                            description.size.height <= px(3. * 22. * zoom),
                            "{case}: {description:?}"
                        );
                        assert!(
                            text.left() >= row.left()
                                && text.right() <= row.right()
                                && control.left() >= row.left()
                                && control.right() <= row.right()
                                && control.bottom() <= row.bottom(),
                            "{case}: {row:?} {text:?} {control:?}"
                        );
                        assert!(
                            f32::from(control.size.width - px(176. * zoom)).abs() <= 1.,
                            "{case}"
                        );
                        if width == 800. && zoom == 1.3 {
                            assert!(control.top() >= text.bottom(), "{case}");
                            assert!(
                                f32::from(control.left() - text.left()).abs() <= 1.,
                                "{case}"
                            );
                        } else if width == 1200. {
                            assert!(control.left() >= text.right(), "{case}");
                            assert!(control.top() < text.bottom(), "{case}");
                        }
                    }
                }
            }
        }
    });
}

#[test]
fn open_archive_confirmation_follows_the_other_window_language() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    with_locale(Locale::English, || {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, _| {
            store.settings.language = Language::English
        });
        let windows = ["message-one", "message-two"].map(|label| {
            cx.add_window(|window, cx| {
                NativeRoot::new(
                    label.into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                )
            })
        });
        store.update(&mut cx, |store, _| {
            store
                .sessions
                .push(runner_core::protocol::session::DirectSessionEntry {
                    claude_account: None,
                    session_id: "owned-shell".into(),
                    project_id: None,
                    role_id: None,
                    handle: None,
                    agent_runtime: "shell".into(),
                    agent_command: "zsh".into(),
                    agent_model: None,
                    agent_effort: None,
                    display_name: "literal {count} 终端".into(),
                    status: SessionStatus::Running,
                    title: None,
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
                })
        });
        windows[1]
            .update(&mut cx, |root, window, cx| {
                root.request_archive_all(
                    None,
                    vec!["owned-shell".into()],
                    ArchiveAllSource::Chat,
                    window,
                    cx,
                );
            })
            .unwrap();
        cx.run_until_parked();
        let confirm_focus = windows[1]
            .read_with(&cx, |root, _| {
                root.terminal_close_confirm.as_ref().unwrap().focus.clone()
            })
            .unwrap();
        for (language, body) in [
            (
                Language::SimplifiedChinese,
                "此操作将永久关闭 1 个终端。已关闭的终端无法恢复。",
            ),
            (
                Language::English,
                "This permanently closes 1 terminal. Closed terminals cannot be restored.",
            ),
        ] {
            windows[0]
                .update(&mut cx, |root, window, cx| {
                    root.apply_settings_selection(
                        SettingsSelection::Language,
                        language.key(),
                        window,
                        cx,
                    )
                })
                .unwrap();
            cx.run_until_parked();
            windows[1]
                .read_with(&cx, |root, _| {
                    let confirm = root.terminal_close_confirm.as_ref().unwrap();
                    let TerminalCloseTarget::ArchiveAll {
                        confirmation_body,
                        session_ids,
                        ..
                    } = &confirm.target
                    else {
                        panic!("archive target retained")
                    };
                    assert_eq!(confirmation_body.resolve(), body);
                    assert_eq!(session_ids, &["owned-shell"]);
                    assert_eq!(confirm.focus, confirm_focus);
                })
                .unwrap();
        }
    });
}

#[test]
fn system_resolution_changes_invalidate_even_when_the_saved_intent_does_not_change() {
    with_locale(Locale::English, || {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let before = store.read_with(&cx, |store, _| store.revisions);
        store.update(&mut cx, |store, cx| {
            store.sync_locale_from(Some("zh-Hant-HK"), cx)
        });
        store.read_with(&cx, |store, _| {
            assert_eq!(store.settings.language, Language::System);
            assert_eq!(store.resolved_locale, Locale::SimplifiedChinese);
            assert_eq!(store.revisions.locale, before.locale + 1);
            assert_eq!(store.revisions.settings, before.settings);
            let reactions = store.revisions.reactions_since(before);
            assert!(reactions.notify_shell());
            assert!(!reactions.apply_terminal_settings);
            assert!(!reactions.reload_role_surfaces);
            assert!(!reactions.reload_crew_surfaces);
        });
        assert_eq!(
            cx.update(|cx| cx.get_menus().unwrap()[1].name.clone()),
            "文件"
        );
        store.update(&mut cx, |store, cx| {
            store.sync_locale_from(Some("en-US"), cx)
        });
        assert_eq!(
            cx.update(|cx| cx.get_menus().unwrap()[1].name.clone()),
            "File"
        );
    });
}

#[test]
fn diagnostics_left_the_nav_and_a_saved_pane_opens_about() {
    assert_eq!(
        SettingsPane::from_route(Some("diagnostics")),
        SettingsPane::About
    );
    assert_eq!(
        SYSTEM_PANES,
        [SettingsPane::Updates, SettingsPane::About].as_slice()
    );
    assert!(filtered_nav_groups("diagnostics").is_empty());
    assert_eq!(
        filtered_nav_groups("application logs"),
        vec![("System", vec![SettingsPane::About])]
    );
}

#[test]
fn shortcut_search_translates_while_override_ids_and_combinations_stay_stable() {
    let overrides: keymap::KeymapOverrides = serde_json::from_str(
        r#"{"new-chat":{"ctrl":true,"meta":false,"alt":false,"shift":true,"code":"KeyJ"}}"#,
    )
    .unwrap();
    let before = serde_json::to_value(&overrides).unwrap();
    let binding = keymap::effective_binding("new-chat", &overrides).unwrap();
    with_locale(Locale::SimplifiedChinese, || {
        let rows = shortcut_rows("新建标签页", &overrides);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id(), "new-chat");
        assert_eq!(rows[0].title(), "新建标签页");
        assert_eq!(
            keymap::effective_binding("new-chat", &overrides),
            Some(binding)
        );
        assert_eq!(serde_json::to_value(&overrides).unwrap(), before);
        assert!(shortcut_rows("打开另一个", &overrides)
            .iter()
            .any(|row| row.id() == "new-window"));
    });
}
