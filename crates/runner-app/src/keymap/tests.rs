use super::*;

fn combo_for(code: &str, shift: bool) -> KeyCombo {
    key_combo(code, true, false, false, shift, None, false)
}

#[test]
fn find_in_terminal_follows_toggle_terminal_drawer_and_reaches_the_open_bar() {
    let ids = entries().iter().map(|entry| entry.id).collect::<Vec<_>>();
    let drawer = ids
        .iter()
        .position(|id| *id == "toggle-terminal-drawer")
        .unwrap();
    assert_eq!(ids[drawer + 1], "find-in-terminal");
    let entry = entry("find-in-terminal").unwrap();
    assert_eq!(entry.scope, KeymapScope::Terminal);
    assert!(!entry.fixed);
    assert_eq!(binding_strings(&entry.default), [platform_default("cmd-f")]);
    #[cfg(target_os = "macos")]
    assert_eq!(format_combo(&entry.default), "⌘F");
    assert!(find_conflict(&entry.default, entry.id, &KeymapOverrides::new()).is_none());
    let predicate =
        gpui::KeyBindingContextPredicate::parse(binding_context(entry, &entry.default).unwrap())
            .unwrap();
    for (stack, reaches) in [
        (&["Root", "Terminal"][..], true),
        (&["Root", "TerminalFind", "TextInput"][..], true),
        (&["Root", "TextInput"][..], false),
    ] {
        let contexts = stack
            .iter()
            .map(|context| gpui::KeyContext::parse(context).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            predicate.depth_of(&contexts).is_some(),
            reaches,
            "{stack:?}"
        );
    }
}

#[test]
fn terminal_drawer_binding_is_only_available_in_missions() {
    let entry = entry("toggle-terminal-drawer").unwrap();
    assert_eq!(binding_context(entry, &entry.default), Some("Mission"));
}

#[test]
#[cfg(target_os = "macos")]
fn registry_matches_the_shipped_defaults_and_fixed_entries() {
    assert_eq!(entries().len(), 36);
    assert_eq!(entries().iter().filter(|entry| entry.fixed).count(), 12);
    assert!(entry("new-window").unwrap().fixed);
    assert!(entry("close-pane").unwrap().fixed);
    assert!(entry("system-close-window").unwrap().fixed);
    assert_eq!(format_combo(&entry("close-pane").unwrap().default), "⌘W");
    assert_eq!(
        format_combo(&entry("system-close-window").unwrap().default),
        "⇧⌘W"
    );
    assert!(entry("copy").is_none());
    assert_eq!(format_combo(&entry("new-window").unwrap().default), "⇧⌘N");
    assert_eq!(
        format_combo(&entry("page-previous").unwrap().default),
        "⇧⌘["
    );
    assert_eq!(format_combo(&entry("zoom-in").unwrap().default), "⌘+");
    assert_eq!(format_combo(&entry("stop-session").unwrap().default), "⇧⌘X");
    assert_eq!(
        format_combo(&entry("resume-session").unwrap().default),
        "⇧⌘R"
    );
    assert_eq!(
        format_combo(&entry("split-pane-right").unwrap().default),
        "⌘D"
    );
    assert_eq!(
        format_combo(&entry("split-pane-down").unwrap().default),
        "⇧⌘D"
    );
    assert_eq!(
        format_combo(&entry("toggle-maximize-pane").unwrap().default),
        "⇧⌘↩"
    );
    assert!(!entry("toggle-maximize-pane").unwrap().fixed);
    assert_eq!(format_combo(&entry("new-terminal").unwrap().default), "⌘T");
    assert_eq!(
        entry("pane-previous").unwrap().default,
        entry("mission-tab-previous").unwrap().default
    );
}

#[test]
fn tab_shortcuts_are_global_fixed_and_ignore_overrides() {
    let ids = (1..=9)
        .map(|index| format!("select-tab-{index}"))
        .collect::<Vec<_>>();
    let tab_entries = tab_selection_entries().collect::<Vec<_>>();
    assert_eq!(
        tab_entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
        ids.iter().map(String::as_str).collect::<Vec<_>>()
    );
    for (index, entry) in (1..=9).zip(tab_entries) {
        assert_eq!(entry.title(), format!("Go to tab {index}"));
        assert_eq!(entry.scope, KeymapScope::Global);
        assert!(entry.fixed);
        assert_eq!(
            binding_strings(&entry.default),
            [platform_default(&format!("cmd-{index}"))]
        );
        assert_eq!(binding_context(entry, &entry.default), None);
        let overrides =
            KeymapOverrides::from([(entry.id.to_owned(), Some(combo_for("KeyP", false)))]);
        assert_eq!(
            effective_binding(entry.id, &overrides),
            Some(entry.default.clone())
        );
    }
}

#[test]
fn fixed_shortcuts_turn_off_but_stay_reserved() {
    let mut overrides = KeymapOverrides::from([
        ("close-pane".to_owned(), None),
        ("new-window".to_owned(), Some(combo_for("KeyP", false))),
    ]);
    assert_eq!(effective_binding("close-pane", &overrides), None);
    assert!(is_overridden("close-pane", &overrides));
    assert_eq!(
        effective_binding("new-window", &overrides),
        Some(entry("new-window").unwrap().default.clone())
    );
    assert!(!is_overridden("new-window", &overrides));

    overrides.insert("select-tab-3".into(), None);
    assert!(!tab_selection_unbound(&overrides));
    assert!(effective_binding("select-tab-3", &overrides).is_some());
    assert!(!is_overridden("select-tab-3", &overrides));
    for entry in tab_selection_entries() {
        overrides.insert(entry.id.into(), None);
    }
    assert!(tab_selection_unbound(&overrides));
    assert!(tab_selection_entries().all(|entry| effective_binding(entry.id, &overrides).is_none()));
    assert!(is_overridden("select-tab-1", &overrides));

    let tab_one = entry("select-tab-1").unwrap().default.clone();
    assert_eq!(
        find_conflict(&tab_one, "new-chat", &overrides).unwrap().id,
        "select-tab-1"
    );
}

#[test]
fn overrides_preserve_null_and_absent_as_different_states() {
    let mut overrides = KeymapOverrides::new();
    assert_eq!(
        effective_binding("command-palette", &overrides),
        Some(entry("command-palette").unwrap().default.clone())
    );
    overrides.insert("command-palette".into(), None);
    assert_eq!(effective_binding("command-palette", &overrides), None);
    overrides.insert("new-window".into(), Some(combo_for("KeyP", false)));
    assert_eq!(
        effective_binding("new-window", &overrides),
        Some(entry("new-window").unwrap().default.clone())
    );
}

#[test]
fn normalization_drops_unknown_and_malformed_values() {
    let values = serde_json::from_value::<BTreeMap<String, serde_json::Value>>(serde_json::json!({
        "command-palette": null,
        "toggle-sidebar": {
            "meta": true, "ctrl": false, "alt": false, "shift": false, "code": "KeyP"
        },
        "zoom-in": { "meta": "yes", "code": "Equal" },
        "zoom-out": {
            "meta": true, "ctrl": false, "alt": false, "shift": false, "code": "a-b"
        },
        "unknown": {
            "meta": true, "ctrl": false, "alt": false, "shift": false, "code": "KeyU"
        }
    }))
    .unwrap();
    let normalized = normalize_override_values(values);
    assert_eq!(normalized.len(), 2);
    assert!(normalized.contains_key("command-palette"));
    assert!(normalized["command-palette"].is_none());
    assert_eq!(normalized["toggle-sidebar"].as_ref().unwrap().code, "KeyP");
    assert!(!normalized.contains_key("zoom-out"));
}

#[test]
fn conflicts_follow_scope_overlap_and_refuse_conflicting_restore() {
    let mut overrides = KeymapOverrides::new();
    overrides.insert("pane-previous".into(), Some(combo_for("KeyJ", false)));
    overrides.insert(
        "mission-tab-next".into(),
        Some(entry("pane-previous").unwrap().default.clone()),
    );
    assert!(find_conflict(
        &entry("pane-previous").unwrap().default,
        "pane-previous",
        &overrides
    )
    .is_none());

    overrides.insert(
        "toggle-sidebar".into(),
        Some(entry("pane-previous").unwrap().default.clone()),
    );
    let conflict = clear_override("pane-previous", &mut overrides).unwrap_err();
    assert_eq!(conflict.id, "toggle-sidebar");
    assert!(overrides.contains_key("pane-previous"));
}

#[test]
fn conflicts_include_system_owned_shortcuts_and_distinct_close_keys() {
    let overrides = KeymapOverrides::new();
    let reserved = reserved_entries()
        .iter()
        .find(|entry| entry.id == "system-toggle-fullscreen")
        .unwrap();
    let conflict = find_conflict(&reserved.default, "new-chat", &overrides).unwrap();
    assert_eq!(conflict.id, "system-toggle-fullscreen");
    let copy = reserved_entries()
        .iter()
        .find(|entry| entry.id == "copy")
        .unwrap();
    assert_eq!(
        find_conflict(&copy.default, "new-chat", &overrides)
            .unwrap()
            .id,
        "copy"
    );
    for id in ["close-pane", "system-close-window"] {
        let close = entry(id).unwrap();
        assert!(find_conflict(&close.default, id, &overrides).is_none());
        assert_eq!(
            find_conflict(&close.default, "new-chat", &overrides)
                .unwrap()
                .id,
            id
        );
    }
}

#[test]
fn fixed_modal_confirm_wins_user_global_overrides_at_every_control_depth() {
    let cx = gpui::TestAppContext::single();
    cx.update(|cx| {
        let key = Keystroke::parse(&platform_default("cmd-enter")).unwrap();
        for (id, global_action) in [
            ("new-terminal", &NewTerminal as &dyn gpui::Action),
            ("new-mission", &NewMission),
            ("zoom-reset", &ZoomReset),
        ] {
            let overrides =
                KeymapOverrides::from([(id.into(), Some(default_combo("cmd-enter", None, false)))]);
            install_bindings(cx, &overrides, TerminalCtrlKeys::RunnerFirst, false);
            for (context, action) in [("StartChat", &ConfirmStartChat as &dyn gpui::Action)] {
                for nested in [false, true] {
                    let mut contexts = vec![
                        gpui::KeyContext::parse("Root").unwrap(),
                        gpui::KeyContext::parse(context).unwrap(),
                    ];
                    if nested {
                        contexts.push(gpui::KeyContext::parse("TextInput").unwrap());
                    }
                    let bindings = cx.key_bindings();
                    let bindings = bindings.borrow();
                    assert!(bindings
                        .bindings_for_input(std::slice::from_ref(&key), &contexts)
                        .0[0]
                        .action()
                        .partial_eq(action));
                }
            }
            let bindings = cx.key_bindings();
            let bindings = bindings.borrow();
            assert!(bindings
                .bindings_for_input(
                    std::slice::from_ref(&key),
                    &[gpui::KeyContext::parse("Root").unwrap()]
                )
                .0[0]
                .action()
                .partial_eq(global_action));
        }
    });
}

#[test]
fn create_bindings_are_global_rebindable_and_modal_keys_stay_out_of_settings() {
    let cx = gpui::TestAppContext::single();
    cx.update(|cx| {
        for (id, action) in [
            ("new-terminal", &NewTerminal as &dyn gpui::Action),
            ("new-mission", &NewMission as &dyn gpui::Action),
        ] {
            assert_eq!(entry(id).unwrap().scope, KeymapScope::Global);
            assert!(!entry(id).unwrap().fixed);
            for combo in [
                Some(entry(id).unwrap().default.clone()),
                Some(combo_for("KeyP", true)),
                None,
            ] {
                let overrides = KeymapOverrides::from([(id.into(), combo.clone())]);
                install_bindings(cx, &overrides, TerminalCtrlKeys::RunnerFirst, false);
                let keymap = cx.key_bindings();
                let keymap = keymap.borrow();
                let bindings = keymap.bindings_for_action(action).collect::<Vec<_>>();
                assert_eq!(bindings.len(), usize::from(combo.is_some()));
                if let Some(combo) = combo {
                    assert!(bindings[0].predicate().is_none());
                    assert_eq!(
                        bindings[0]
                            .match_keystrokes(&[
                                Keystroke::parse(&binding_strings(&combo)[0]).unwrap()
                            ]),
                        Some(false)
                    );
                }
            }
        }
        install_bindings(
            cx,
            &KeymapOverrides::new(),
            TerminalCtrlKeys::RunnerFirst,
            true,
        );
        let bindings = cx.key_bindings();
        let bindings = bindings.borrow();
        assert_eq!(bindings.bindings_for_action(&ConfirmStartChat).count(), 1);
        assert!(
            entries()
                .iter()
                .all(|entry| !entry.id.starts_with("confirm-")
                    && !entry.id.starts_with("start-chat-"))
        );
    });
    assert_eq!(
        fixed_shortcut("cmd-enter"),
        format_combo(&default_combo("cmd-enter", None, false))
    );
}

#[test]
fn installed_close_keys_match_fixed_defaults_without_a_focus_context() {
    let cx = gpui::TestAppContext::single();
    cx.update(|cx| {
        install_bindings(
            cx,
            &KeymapOverrides::new(),
            TerminalCtrlKeys::RunnerFirst,
            false,
        );
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        for (id, action) in [
            ("close-pane", &CloseTab as &dyn gpui::Action),
            ("system-close-window", &CloseWindow as &dyn gpui::Action),
        ] {
            let bindings = keymap.bindings_for_action(action).collect::<Vec<_>>();
            assert_eq!(bindings.len(), 1);
            assert!(bindings[0].predicate().is_none());
            let key = Keystroke::parse(&binding_strings(&entry(id).unwrap().default)[0]).unwrap();
            assert_eq!(bindings[0].match_keystrokes(&[key]), Some(false));
        }
        #[cfg(any(windows, target_os = "linux"))]
        assert!(keymap
            .all_bindings_for_input(&[Keystroke::parse("ctrl-w").unwrap()])
            .is_empty());
    });
}

#[test]
fn stop_and_resume_defaults_have_no_conflicts() {
    let overrides = KeymapOverrides::new();
    for id in ["stop-session", "resume-session"] {
        let entry = entry(id).unwrap();
        assert!(
            find_conflict(&entry.default, id, &overrides).is_none(),
            "{id}"
        );
    }
}

#[test]
fn capture_validity_and_formatting_match_the_react_contract() {
    let bare = KeyEventLike {
        meta: false,
        ctrl: false,
        alt: false,
        shift: false,
        code: "KeyK".into(),
        key: "k".into(),
    };
    assert!(combo_from_event(&bare).is_none());
    let function = KeyEventLike {
        code: "F12".into(),
        key: "f12".into(),
        ..bare.clone()
    };
    assert_eq!(format_combo(&combo_from_event(&function).unwrap()), "F12");
    assert!(combo_from_keystroke(&Keystroke::parse("cmd-f24").unwrap()).is_some());
    assert!(combo_from_keystroke(&Keystroke::parse("cmd-f25").unwrap()).is_none());
    let plus = KeyEventLike {
        meta: true,
        shift: true,
        code: "Equal".into(),
        key: "+".into(),
        ..bare
    };
    assert_eq!(
        format_combo(&combo_from_event(&plus).unwrap()),
        if cfg!(target_os = "linux") {
            "Shift+Super++"
        } else if cfg!(windows) {
            "Shift+Win++"
        } else {
            "⇧⌘+"
        }
    );
}

#[test]
fn shift_optional_matches_both_zoom_in_forms_only() {
    let combo = entry("zoom-in").unwrap().default.clone();
    #[cfg(target_os = "macos")]
    assert_eq!((combo.meta, combo.ctrl, combo.alt), (true, false, false));
    for shift in [false, true] {
        assert!(combo_matches_event(
            &combo,
            &KeyEventLike {
                meta: combo.meta,
                ctrl: combo.ctrl,
                alt: combo.alt,
                shift,
                code: "Equal".into(),
                key: if shift { "+" } else { "=" }.into(),
            }
        ));
    }
    let ordinary = combo_for("Equal", false);
    assert!(!combo_matches_event(
        &ordinary,
        &KeyEventLike {
            meta: true,
            ctrl: false,
            alt: false,
            shift: true,
            code: "Equal".into(),
            key: "+".into(),
        }
    ));
}

#[test]
#[cfg(target_os = "macos")]
fn registry_defaults_compile_to_gpui_runtime_keystrokes() {
    type ExpectedBinding<'a> = (&'a str, &'a str, bool, bool, bool, bool);
    type ExpectedEntry<'a> = (&'a str, &'a [ExpectedBinding<'a>]);
    let expected: [ExpectedEntry<'_>; 33] = [
        (
            "new-window",
            &[("shift-cmd-n", "n", true, false, false, true)],
        ),
        ("new-chat", &[("cmd-n", "n", true, false, false, false)]),
        (
            "command-palette",
            &[("cmd-k", "k", true, false, false, false)],
        ),
        (
            "search-files",
            &[("shift-cmd-o", "o", true, false, false, true)],
        ),
        (
            "toggle-sidebar",
            &[("cmd-s", "s", true, false, false, false)],
        ),
        (
            "switch-sidebar-view",
            &[("cmd-.", ".", true, false, false, false)],
        ),
        (
            "toggle-terminal-drawer",
            &[("alt-f12", "f12", false, false, true, false)],
        ),
        (
            "open-settings",
            &[("cmd-,", ",", true, false, false, false)],
        ),
        (
            "page-previous",
            &[("cmd-{", "{", true, false, false, false)],
        ),
        ("page-next", &[("cmd-}", "}", true, false, false, false)]),
        (
            "zoom-in",
            &[
                ("cmd-=", "=", true, false, false, false),
                ("cmd-+", "+", true, false, false, false),
            ],
        ),
        ("zoom-out", &[("cmd--", "-", true, false, false, false)]),
        ("zoom-reset", &[("cmd-0", "0", true, false, false, false)]),
        ("select-tab-1", &[("cmd-1", "1", true, false, false, false)]),
        ("select-tab-2", &[("cmd-2", "2", true, false, false, false)]),
        ("select-tab-3", &[("cmd-3", "3", true, false, false, false)]),
        ("select-tab-4", &[("cmd-4", "4", true, false, false, false)]),
        ("select-tab-5", &[("cmd-5", "5", true, false, false, false)]),
        ("select-tab-6", &[("cmd-6", "6", true, false, false, false)]),
        ("select-tab-7", &[("cmd-7", "7", true, false, false, false)]),
        ("select-tab-8", &[("cmd-8", "8", true, false, false, false)]),
        ("select-tab-9", &[("cmd-9", "9", true, false, false, false)]),
        (
            "pane-previous",
            &[("cmd-[", "[", true, false, false, false)],
        ),
        ("pane-next", &[("cmd-]", "]", true, false, false, false)]),
        (
            "split-pane-right",
            &[("cmd-d", "d", true, false, false, false)],
        ),
        (
            "split-pane-down",
            &[("shift-cmd-d", "d", true, false, false, true)],
        ),
        (
            "toggle-maximize-pane",
            &[("shift-cmd-enter", "enter", true, false, false, true)],
        ),
        ("close-pane", &[("cmd-w", "w", true, false, false, false)]),
        (
            "system-close-window",
            &[("shift-cmd-w", "w", true, false, false, true)],
        ),
        (
            "stop-session",
            &[("shift-cmd-x", "x", true, false, false, true)],
        ),
        (
            "resume-session",
            &[("shift-cmd-r", "r", true, false, false, true)],
        ),
        (
            "mission-tab-previous",
            &[("cmd-[", "[", true, false, false, false)],
        ),
        (
            "mission-tab-next",
            &[("cmd-]", "]", true, false, false, false)],
        ),
    ];

    for (id, bindings) in expected {
        assert_eq!(
            binding_strings(&entry(id).unwrap().default),
            bindings
                .iter()
                .map(|(binding, ..)| (*binding).to_owned())
                .collect::<Vec<_>>(),
            "{id}"
        );
        for (binding, key, platform, ctrl, alt, shift) in bindings {
            assert_eq!(
                Keystroke::parse(binding).unwrap(),
                Keystroke {
                    modifiers: gpui::Modifiers {
                        platform: *platform,
                        control: *ctrl,
                        alt: *alt,
                        shift: *shift,
                        ..Default::default()
                    },
                    key: (*key).to_owned(),
                    key_char: None,
                },
                "{id}: {binding}"
            );
        }
    }

    let recorded_bracket = combo_for("BracketLeft", true);
    assert_eq!(binding_strings(&recorded_bracket), ["cmd-{"]);
    assert_eq!(
        Keystroke::parse(&binding_strings(&recorded_bracket)[0]).unwrap(),
        Keystroke {
            modifiers: gpui::Modifiers {
                platform: true,
                ..Default::default()
            },
            key: "{".into(),
            key_char: None,
        }
    );

    let recorded_letter = combo_for("KeyK", true);
    assert_eq!(binding_strings(&recorded_letter), ["shift-cmd-k"]);
    assert!(
        Keystroke::parse(&binding_strings(&recorded_letter)[0])
            .unwrap()
            .modifiers
            .shift
    );
}
#[test]
#[cfg(target_os = "macos")]
fn macos_registry_defaults_match_expected_combos() {
    let expected = [
        (
            "new-window",
            key_combo("KeyN", true, false, false, true, None, false),
        ),
        (
            "new-chat",
            key_combo("KeyN", true, false, false, false, None, false),
        ),
        (
            "new-terminal",
            key_combo("KeyT", true, false, false, false, None, false),
        ),
        (
            "new-mission",
            key_combo("KeyM", true, false, false, true, None, false),
        ),
        (
            "command-palette",
            key_combo("KeyK", true, false, false, false, None, false),
        ),
        (
            "search-files",
            key_combo("KeyO", true, false, false, true, None, false),
        ),
        (
            "toggle-sidebar",
            key_combo("KeyS", true, false, false, false, None, false),
        ),
        (
            "switch-sidebar-view",
            key_combo("Period", true, false, false, false, None, false),
        ),
        (
            "toggle-terminal-drawer",
            key_combo("F12", false, false, true, false, None, false),
        ),
        (
            "find-in-terminal",
            key_combo("KeyF", true, false, false, false, None, false),
        ),
        (
            "open-settings",
            key_combo("Comma", true, false, false, false, None, false),
        ),
        (
            "page-previous",
            key_combo("BracketLeft", true, false, false, true, None, false),
        ),
        (
            "page-next",
            key_combo("BracketRight", true, false, false, true, None, false),
        ),
        (
            "zoom-in",
            key_combo("Equal", true, false, false, false, Some("+"), true),
        ),
        (
            "zoom-out",
            key_combo("Minus", true, false, false, false, None, false),
        ),
        (
            "zoom-reset",
            key_combo("Digit0", true, false, false, false, None, false),
        ),
        (
            "select-tab-1",
            key_combo("Digit1", true, false, false, false, None, false),
        ),
        (
            "select-tab-2",
            key_combo("Digit2", true, false, false, false, None, false),
        ),
        (
            "select-tab-3",
            key_combo("Digit3", true, false, false, false, None, false),
        ),
        (
            "select-tab-4",
            key_combo("Digit4", true, false, false, false, None, false),
        ),
        (
            "select-tab-5",
            key_combo("Digit5", true, false, false, false, None, false),
        ),
        (
            "select-tab-6",
            key_combo("Digit6", true, false, false, false, None, false),
        ),
        (
            "select-tab-7",
            key_combo("Digit7", true, false, false, false, None, false),
        ),
        (
            "select-tab-8",
            key_combo("Digit8", true, false, false, false, None, false),
        ),
        (
            "select-tab-9",
            key_combo("Digit9", true, false, false, false, None, false),
        ),
        (
            "pane-previous",
            key_combo("BracketLeft", true, false, false, false, None, false),
        ),
        (
            "pane-next",
            key_combo("BracketRight", true, false, false, false, None, false),
        ),
        (
            "split-pane-right",
            key_combo("KeyD", true, false, false, false, None, false),
        ),
        (
            "split-pane-down",
            key_combo("KeyD", true, false, false, true, None, false),
        ),
        (
            "toggle-maximize-pane",
            key_combo("Enter", true, false, false, true, None, false),
        ),
        (
            "close-pane",
            key_combo("KeyW", true, false, false, false, None, false),
        ),
        (
            "system-close-window",
            key_combo("KeyW", true, false, false, true, None, false),
        ),
        (
            "stop-session",
            key_combo("KeyX", true, false, false, true, None, false),
        ),
        (
            "resume-session",
            key_combo("KeyR", true, false, false, true, None, false),
        ),
        (
            "mission-tab-previous",
            key_combo("BracketLeft", true, false, false, false, None, false),
        ),
        (
            "mission-tab-next",
            key_combo("BracketRight", true, false, false, false, None, false),
        ),
        (
            "system-quit",
            key_combo("KeyQ", true, false, false, false, None, false),
        ),
        (
            "system-quit-stop",
            key_combo("KeyQ", true, false, true, false, None, false),
        ),
        (
            "system-hide",
            key_combo("KeyH", true, false, false, false, None, false),
        ),
        (
            "system-hide-others",
            key_combo("KeyH", true, false, true, false, None, false),
        ),
        (
            "system-minimize",
            key_combo("KeyM", true, false, false, false, None, false),
        ),
        (
            "system-toggle-fullscreen",
            key_combo("KeyF", true, true, false, false, None, false),
        ),
        (
            "copy",
            key_combo("KeyC", true, false, false, false, None, false),
        ),
        (
            "system-paste",
            key_combo("KeyV", true, false, false, false, None, false),
        ),
    ];
    let actual = entries()
        .iter()
        .chain(reserved_entries())
        .map(|entry| (entry.id, &entry.default))
        .collect::<Vec<_>>();
    assert_eq!(
        serde_json::to_vec(&actual).unwrap(),
        serde_json::to_vec(expected.as_slice()).unwrap()
    );
}

#[test]
fn windows_defaults_replace_command_with_control() {
    for (macos, windows) in [
        ("cmd-n", "ctrl-n"),
        ("cmd-w", "ctrl-shift-w"),
        ("cmd-t", "ctrl-shift-t"),
        ("shift-cmd-m", "shift-ctrl-m"),
        ("cmd-enter", "ctrl-enter"),
        ("shift-cmd-w", "alt-f4"),
        ("shift-cmd-n", "shift-ctrl-n"),
        ("cmd-d", "ctrl-shift-d"),
        ("shift-cmd-d", "alt-shift-d"),
        ("cmd-k", "ctrl-shift-p"),
        ("cmd-s", "ctrl-b"),
        ("shift-cmd-k", "shift-ctrl-k"),
        ("shift-cmd-r", "shift-ctrl-r"),
        ("shift-cmd-x", "shift-ctrl-x"),
        ("shift-cmd-z", "shift-ctrl-z"),
        ("cmd-c", "ctrl-c"),
        ("cmd-v", "ctrl-v"),
        ("cmd-h", "ctrl-h"),
        ("cmd-alt-h", "ctrl-alt-h"),
        ("cmd-,", "ctrl-,"),
        ("cmd--", "ctrl--"),
        ("cmd-1", "ctrl-1"),
        ("cmd-[", "ctrl-["),
        ("shift-cmd-[", "shift-ctrl-["),
        ("cmd-]", "ctrl-]"),
        ("shift-cmd-]", "shift-ctrl-]"),
        ("cmd-{", "ctrl-{"),
        ("cmd-}", "ctrl-}"),
        ("cmd-left", "ctrl-left"),
        ("ctrl-cmd-f", "f11"),
        ("cmd-f", "ctrl-shift-f"),
        ("alt-f12", "alt-f12"),
    ] {
        let actual = windows_default(macos);
        assert_eq!(actual, windows, "{macos}");
        let keystroke = Keystroke::parse(&actual).unwrap();
        assert!(!keystroke.modifiers.platform);
        assert!(code_from_gpui_key(&keystroke.key).is_some(), "{macos}");
        #[cfg(target_os = "macos")]
        assert_eq!(platform_default(macos).as_bytes(), macos.as_bytes());
    }
    for entry in entries().iter().chain(reserved_entries()) {
        for binding in binding_strings(&entry.default) {
            assert_ne!(windows_default(&binding), "ctrl-w", "{}", entry.id);
            let windows = Keystroke::parse(&windows_default(&binding)).unwrap();
            assert!(!windows.modifiers.platform, "{}", entry.id);
            assert!(code_from_gpui_key(&windows.key).is_some(), "{}", entry.id);
            if windows.key.len() == 1 && windows.key.as_bytes()[0].is_ascii_alphabetic() {
                assert!(
                    windows.modifiers.control || windows.modifiers.alt,
                    "{}",
                    entry.id
                );
            }
            // Ctrl+Alt is AltGr text input on Windows and many Linux layouts.
            if entries().contains(entry) {
                assert!(
                    !(windows.modifiers.control && windows.modifiers.alt),
                    "{}",
                    entry.id
                );
            }
        }
    }
}

#[test]
fn windows_defaults_have_no_unexpected_scope_collisions() {
    let defaults = entries()
        .iter()
        .chain(reserved_entries())
        .filter(|entry| {
            !matches!(
                entry.id,
                "system-quit"
                    | "system-quit-stop"
                    | "system-hide"
                    | "system-hide-others"
                    | "system-minimize"
            )
        })
        .map(|entry| {
            let binding = windows_default(&binding_strings(&entry.default)[0]);
            let keystroke = Keystroke::parse(&binding).unwrap();
            let mut combo = combo_from_keystroke(&keystroke).unwrap();
            combo.shift_optional = entry.default.shift_optional;
            (entry, combo)
        })
        .collect::<Vec<_>>();
    for (index, (left, left_combo)) in defaults.iter().enumerate() {
        for (right, right_combo) in &defaults[index + 1..] {
            assert!(
                !left.scope.overlaps(right.scope) || !combos_collide(left_combo, right_combo),
                "{} collides with {}",
                left.id,
                right.id
            );
        }
    }
    let fullscreen = Keystroke::parse(&windows_default("ctrl-cmd-f")).unwrap();
    assert_eq!(
        code_from_gpui_key(&fullscreen.key),
        Some(("F11".into(), false))
    );
}

#[test]
fn default_navigation_reaches_terminals_and_user_overrides_keep_their_modifiers() {
    for id in ["new-chat", "command-palette", "page-previous", "page-next"] {
        let entry = entry(id).unwrap();
        assert_eq!(
            binding_context(entry, &entry.default),
            Some(if matches!(id, "new-chat" | "command-palette") {
                "!(TextInput && !LauncherSearch) && !Settings"
            } else {
                "!TextInput && !Settings"
            })
        );
        let user_combo = key_combo("KeyP", false, true, false, false, None, false);
        let overrides = KeymapOverrides::from([(id.into(), Some(user_combo.clone()))]);
        assert_eq!(effective_binding(id, &overrides), Some(user_combo.clone()));
        assert_eq!(binding_strings(&user_combo), ["ctrl-p"]);
        assert_eq!(
            binding_context(entry, &user_combo),
            if cfg!(any(windows, target_os = "linux")) {
                binding_context(entry, &entry.default)
            } else {
                Some("!TextInput && !Settings && !Terminal")
            }
        );
        let alt_combo = key_combo("KeyP", false, false, true, false, None, false);
        assert_eq!(
            binding_context(entry, &alt_combo),
            Some("!TextInput && !Settings && !Terminal")
        );
    }
    let palette = entry("command-palette").unwrap();
    let predicate = gpui::KeyBindingContextPredicate::parse(
        binding_context(palette, &palette.default).unwrap(),
    )
    .unwrap();
    for (stack, opens) in [
        (&["Root"][..], true),
        (&["Root", "LauncherSearch", "TextInput"][..], true),
        (&["Root", "TextInput"][..], false),
        (&["Root", "Settings", "TextInput"][..], false),
    ] {
        let contexts = stack
            .iter()
            .map(|context| gpui::KeyContext::parse(context).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(predicate.eval(&contexts), opens, "{stack:?}");
    }
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn windows_table_has_no_command_modifier_or_quit_binding() {
    for entry in entries().iter().chain(reserved_entries()) {
        assert!(!entry.default.meta, "{}", entry.id);
        assert!(!matches!(
            entry.id,
            "system-quit"
                | "system-quit-stop"
                | "system-hide"
                | "system-hide-others"
                | "system-minimize"
        ));
        for binding in binding_strings(&entry.default) {
            assert!(!binding.contains("cmd-"), "{}: {binding}", entry.id);
            assert!(!Keystroke::parse(&binding).unwrap().modifiers.platform);
        }
        if entries().iter().any(|candidate| candidate.id == entry.id) {
            assert!(
                find_conflict(&entry.default, entry.id, &KeymapOverrides::new()).is_none(),
                "{}",
                entry.id
            );
        }
    }
    for (id, hint) in [
        ("new-chat", "Ctrl+N"),
        ("new-terminal", "Ctrl+Shift+T"),
        ("new-mission", "Ctrl+Shift+M"),
        ("new-window", "Ctrl+Shift+N"),
        ("command-palette", "Ctrl+Shift+P"),
        ("search-files", "Ctrl+Shift+O"),
        ("toggle-sidebar", "Ctrl+B"),
        ("switch-sidebar-view", "Ctrl+."),
        ("split-pane-right", "Ctrl+Shift+D"),
        ("split-pane-down", "Alt+Shift+D"),
        ("toggle-maximize-pane", "Ctrl+Shift+Enter"),
        ("close-pane", "Ctrl+Shift+W"),
        ("system-close-window", "Alt+F4"),
        ("stop-session", "Ctrl+Shift+X"),
        ("resume-session", "Ctrl+Shift+R"),
        ("pane-previous", "Ctrl+["),
        ("pane-next", "Ctrl+]"),
        ("mission-tab-previous", "Ctrl+["),
        ("mission-tab-next", "Ctrl+]"),
        ("page-previous", "Ctrl+Shift+["),
        ("page-next", "Ctrl+Shift+]"),
        ("open-settings", "Ctrl+,"),
        ("zoom-in", "Ctrl++"),
        ("find-in-terminal", "Ctrl+Shift+F"),
    ] {
        assert_eq!(format_combo(&entry(id).unwrap().default), hint);
    }
    let copy = reserved_entries()
        .iter()
        .find(|entry| entry.id == "copy")
        .unwrap();
    assert_eq!(format_combo(&copy.default), "Ctrl+C");
    assert!(find_conflict(
        &key_combo("KeyQ", false, true, false, true, None, false),
        "new-chat",
        &KeymapOverrides::new()
    )
    .is_none());
}

fn fired(cx: &App, key: &str, stack: &[&str]) -> Option<Box<dyn gpui::Action>> {
    let contexts = stack
        .iter()
        .map(|context| gpui::KeyContext::parse(context).unwrap())
        .collect::<Vec<_>>();
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    keymap
        .bindings_for_input(&[Keystroke::parse(key).unwrap()], &contexts)
        .0
        .first()
        .map(|binding| binding.action().boxed_clone())
}

fn assert_fires(cx: &App, key: &str, stack: &[&str], action: Option<&dyn gpui::Action>) {
    let actual = fired(cx, key, stack);
    match (action, &actual) {
        (Some(expected), Some(actual)) => {
            assert!(
                actual.partial_eq(expected),
                "{key} in {stack:?}: {actual:?}"
            )
        }
        (None, None) => {}
        _ => panic!("{key} in {stack:?}: expected {action:?}, got {actual:?}"),
    }
}

#[test]
fn zoom_keys_and_their_rebinds_step_the_file_text_only_in_the_viewer() {
    let mut overrides = KeymapOverrides::new();
    let rebound = combo_for("KeyK", false);
    overrides.insert("zoom-in".into(), Some(rebound.clone()));
    let rebound = binding_strings(&rebound)[0].clone();
    let reset = binding_strings(&entry("zoom-reset").unwrap().default)[0].clone();
    let cx = gpui::TestAppContext::single();
    cx.update(|cx| {
        install_bindings(cx, &overrides, TerminalCtrlKeys::RunnerFirst, false);
        for viewer in [
            &["Root", "FileViewer"][..],
            &["Root", "FileViewer", "TextInput"],
        ] {
            assert_fires(cx, &rebound, viewer, Some(&crate::ZoomFileIn));
            assert_fires(cx, &reset, viewer, Some(&ZoomReset));
        }
        for elsewhere in [TERMINAL, SIDEBAR] {
            assert_fires(cx, &rebound, elsewhere, Some(&ZoomIn));
            assert_fires(cx, &reset, elsewhere, Some(&ZoomReset));
        }
    });
}

const TERMINAL: &[&str] = &["Root", "ChatSplit", "Terminal"];
const MISSION_TERMINAL: &[&str] = &["Root", "Mission", "Terminal"];
const SIDEBAR: &[&str] = &["Root"];

#[test]
fn minimize_keeps_command_m_on_macos_and_no_ctrl_m_elsewhere() {
    let minimize = reserved_entries()
        .iter()
        .find(|entry| entry.id == "system-minimize");
    assert_eq!(minimize.is_some(), cfg!(target_os = "macos"));
    assert!(entries().iter().all(|entry| entry.id != "system-minimize"));
    let cx = gpui::TestAppContext::single();
    cx.update(|cx| {
        for mode in [
            TerminalCtrlKeys::RunnerFirst,
            TerminalCtrlKeys::TerminalFirst,
        ] {
            install_bindings(cx, &KeymapOverrides::new(), mode, false);
            let keymap = cx.key_bindings();
            let keymap = keymap.borrow();
            let bindings = keymap
                .bindings_for_action(&crate::Minimize)
                .collect::<Vec<_>>();
            #[cfg(target_os = "macos")]
            {
                assert_eq!(bindings.len(), 1);
                assert!(bindings[0].predicate().is_none());
                assert_eq!(
                    bindings[0].match_keystrokes(&[Keystroke::parse("cmd-m").unwrap()]),
                    Some(false)
                );
            }
            #[cfg(any(windows, target_os = "linux"))]
            {
                assert!(bindings.is_empty());
                assert!(keymap
                    .all_bindings_for_input(&[Keystroke::parse("ctrl-m").unwrap()])
                    .is_empty());
            }
        }
    });
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn windows_defaults_leave_terminal_editing_keys_to_terminals() {
    for (id, binding) in [
        ("split-pane-right", "ctrl-shift-d"),
        ("split-pane-down", "alt-shift-d"),
        ("command-palette", "ctrl-shift-p"),
        ("toggle-sidebar", "ctrl-b"),
    ] {
        assert_eq!(binding_strings(&entry(id).unwrap().default), [binding]);
    }
    let cx = gpui::TestAppContext::single();
    cx.update(|cx| {
        for mode in [
            TerminalCtrlKeys::RunnerFirst,
            TerminalCtrlKeys::TerminalFirst,
        ] {
            install_bindings(cx, &KeymapOverrides::new(), mode, false);
            for key in ["ctrl-d", "ctrl-k", "ctrl-s", "ctrl-m"] {
                assert_fires(cx, key, TERMINAL, None);
            }
            for stack in [TERMINAL, SIDEBAR] {
                assert_fires(cx, "ctrl-shift-d", stack, Some(&SplitPaneRight));
                assert_fires(cx, "alt-shift-d", stack, Some(&SplitPaneDown));
                assert_fires(cx, "ctrl-shift-p", stack, Some(&CommandPalette));
            }
            assert_fires(cx, "ctrl-b", SIDEBAR, Some(&ToggleSidebar));
        }
    });
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn terminal_first_frees_bare_ctrl_defaults_only_inside_terminals() {
    let cx = gpui::TestAppContext::single();
    cx.update(|cx| {
        type FreedKey<'a> = (&'a str, &'a [&'a str], &'a [&'a str], &'a dyn gpui::Action);
        let bare: [FreedKey<'_>; 6] = [
            ("ctrl-n", TERMINAL, SIDEBAR, &NewTab),
            ("ctrl-b", TERMINAL, SIDEBAR, &ToggleSidebar),
            ("ctrl-.", TERMINAL, SIDEBAR, &SwitchSidebarView),
            (
                "ctrl-[",
                TERMINAL,
                &["Root", "ChatSplit"],
                &FocusPreviousPane,
            ),
            (
                "ctrl-]",
                MISSION_TERMINAL,
                &["Root", "Mission"],
                &MissionTabNext,
            ),
            ("ctrl-1", TERMINAL, SIDEBAR, &SelectTab1),
        ];
        install_bindings(
            cx,
            &KeymapOverrides::new(),
            TerminalCtrlKeys::RunnerFirst,
            false,
        );
        for (key, inside, _, action) in bare {
            assert_fires(cx, key, inside, Some(action));
        }
        assert_fires(cx, "ctrl-,", TERMINAL, Some(&OpenSettings));

        install_bindings(
            cx,
            &KeymapOverrides::new(),
            TerminalCtrlKeys::TerminalFirst,
            false,
        );
        for (key, inside, outside, action) in bare {
            assert_fires(cx, key, inside, None);
            assert_fires(cx, key, outside, Some(action));
        }
        assert_fires(cx, "ctrl-,", TERMINAL, None);
        for (key, action) in [
            ("ctrl-shift-p", &CommandPalette as &dyn gpui::Action),
            ("ctrl-shift-d", &SplitPaneRight),
            ("ctrl-v", &Paste),
            ("ctrl-=", &ZoomIn),
            ("ctrl-+", &ZoomIn),
            ("ctrl--", &ZoomOut),
            ("ctrl-0", &ZoomReset),
            ("ctrl-shift-t", &NewTerminal),
        ] {
            assert_fires(cx, key, TERMINAL, Some(action));
        }
        assert_fires(
            cx,
            "ctrl-shift-f",
            &["Root", "TerminalFind", "TextInput"],
            Some(&FindInTerminal),
        );
    });
}

#[test]
fn switch_sidebar_view_dispatches_defaults_rebinds_and_unbinding() {
    let entry = entry("switch-sidebar-view").unwrap();
    assert!(!entry.fixed);
    assert_eq!(entry.default.code, "Period");
    assert_eq!(windows_default("cmd-."), "ctrl-.");
    let default_key = binding_strings(&entry.default)[0].clone();
    let bare = key_combo("KeyJ", false, true, false, false, None, false);
    let overrides = KeymapOverrides::from([(entry.id.into(), Some(bare))]);
    let cx = gpui::TestAppContext::single();
    cx.update(|cx| {
        install_bindings(
            cx,
            &KeymapOverrides::new(),
            TerminalCtrlKeys::RunnerFirst,
            false,
        );
        for stack in [TERMINAL, MISSION_TERMINAL, SIDEBAR] {
            assert_fires(cx, &default_key, stack, Some(&SwitchSidebarView));
        }
        for stack in [&["Root", "TextInput"][..], &["Root", "Settings"]] {
            assert_fires(cx, &default_key, stack, None);
        }
        install_bindings(cx, &overrides, TerminalCtrlKeys::RunnerFirst, false);
        assert_fires(cx, &default_key, SIDEBAR, None);
        assert_fires(cx, "ctrl-j", TERMINAL, Some(&SwitchSidebarView));
        install_bindings(cx, &overrides, TerminalCtrlKeys::TerminalFirst, false);
        for stack in [TERMINAL, MISSION_TERMINAL] {
            assert_fires(cx, "ctrl-j", stack, None);
        }
        assert_fires(cx, "ctrl-j", SIDEBAR, Some(&SwitchSidebarView));
        let unbound = KeymapOverrides::from([(entry.id.into(), None)]);
        install_bindings(cx, &unbound, TerminalCtrlKeys::RunnerFirst, false);
        assert_fires(cx, &default_key, SIDEBAR, None);
        assert_fires(cx, "ctrl-j", SIDEBAR, None);
        install_bindings(
            cx,
            &KeymapOverrides::new(),
            TerminalCtrlKeys::RunnerFirst,
            false,
        );
        assert_fires(cx, &default_key, SIDEBAR, Some(&SwitchSidebarView));
    });
}

#[test]
fn terminal_first_follows_the_effective_binding() {
    let bare = key_combo("KeyJ", false, true, false, false, None, false);
    let shifted = key_combo("KeyJ", false, true, false, true, None, false);
    let bare_zoom = key_combo("KeyU", false, true, false, false, None, false);
    let overrides = KeymapOverrides::from([
        ("new-terminal".to_owned(), Some(bare.clone())),
        ("new-mission".to_owned(), Some(shifted.clone())),
        ("zoom-reset".to_owned(), Some(bare_zoom.clone())),
        (
            "split-pane-right".to_owned(),
            Some(key_combo("KeyY", false, true, false, false, None, false)),
        ),
    ]);
    assert!(frees_for_terminal(entry("new-terminal").unwrap(), &bare));
    assert!(!frees_for_terminal(entry("new-mission").unwrap(), &shifted));
    assert!(!frees_for_terminal(
        entry("zoom-reset").unwrap(),
        &bare_zoom
    ));
    assert!(!frees_for_terminal(
        entry("new-terminal").unwrap(),
        &key_combo("KeyJ", true, false, false, false, None, false)
    ));
    assert!(!frees_for_terminal(
        entry("new-terminal").unwrap(),
        &key_combo("KeyJ", false, true, true, false, None, false)
    ));
    let cx = gpui::TestAppContext::single();
    cx.update(|cx| {
        install_bindings(cx, &overrides, TerminalCtrlKeys::RunnerFirst, false);
        assert_fires(cx, "ctrl-j", TERMINAL, Some(&NewTerminal));
        assert_fires(cx, "ctrl-y", TERMINAL, Some(&SplitPaneRight));

        install_bindings(cx, &overrides, TerminalCtrlKeys::TerminalFirst, false);
        assert_fires(cx, "ctrl-j", TERMINAL, None);
        assert_fires(cx, "ctrl-j", MISSION_TERMINAL, None);
        assert_fires(cx, "ctrl-j", SIDEBAR, Some(&NewTerminal));
        assert_fires(cx, "ctrl-y", TERMINAL, None);
        assert_fires(cx, "ctrl-y", SIDEBAR, Some(&SplitPaneRight));
        assert_fires(cx, "ctrl-shift-j", TERMINAL, Some(&NewMission));
        assert_fires(cx, "ctrl-u", TERMINAL, Some(&ZoomReset));
        // macOS defaults use ⌘ and are never freed.
        #[cfg(target_os = "macos")]
        for (key, action) in [
            ("cmd-n", &NewTab as &dyn gpui::Action),
            ("cmd-s", &ToggleSidebar),
            ("cmd-1", &SelectTab1),
        ] {
            assert_fires(cx, key, TERMINAL, Some(action));
        }
    });
}

#[test]
fn terminal_first_contexts_parse_and_keep_their_outside_rules() {
    let combo = key_combo("KeyJ", false, true, false, false, None, false);
    let in_terminal = TERMINAL
        .iter()
        .map(|context| gpui::KeyContext::parse(context).unwrap())
        .collect::<Vec<_>>();
    for entry in entries() {
        let context = installed_context(entry, &combo, TerminalCtrlKeys::TerminalFirst);
        let reaches_terminal = context.as_deref().is_none_or(|context| {
            gpui::KeyBindingContextPredicate::parse(context)
                .unwrap()
                .depth_of(&in_terminal)
                .is_some()
        });
        if frees_for_terminal(entry, &combo) {
            assert!(!reaches_terminal, "{}", entry.id);
        } else {
            assert!(entry.id.starts_with("zoom-"), "{}", entry.id);
            assert!(reaches_terminal, "{}", entry.id);
        }
        assert_eq!(
            installed_context(entry, &combo, TerminalCtrlKeys::RunnerFirst).as_deref(),
            binding_context(entry, &combo),
            "{}",
            entry.id
        );
    }
    let settings = entry("open-settings").unwrap();
    let predicate = gpui::KeyBindingContextPredicate::parse(
        &installed_context(settings, &combo, TerminalCtrlKeys::TerminalFirst).unwrap(),
    )
    .unwrap();
    let in_settings_field =
        ["Root", "Settings", "TextInput"].map(|context| gpui::KeyContext::parse(context).unwrap());
    assert!(predicate.depth_of(&in_settings_field).is_some());
    let toggle = entry("toggle-sidebar").unwrap();
    let predicate = gpui::KeyBindingContextPredicate::parse(
        &installed_context(toggle, &combo, TerminalCtrlKeys::TerminalFirst).unwrap(),
    )
    .unwrap();
    assert!(!predicate.eval(&in_settings_field));
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn saved_ctrl_k_palette_override_follows_the_terminal_policy() {
    let ctrl_k = key_combo("KeyK", false, true, false, false, None, false);
    let overrides = KeymapOverrides::from([("command-palette".to_owned(), Some(ctrl_k))]);
    let launcher: &[&str] = &["Root", "LauncherSearch", "TextInput"];
    let cx = gpui::TestAppContext::single();
    cx.update(|cx| {
        install_bindings(cx, &overrides, TerminalCtrlKeys::RunnerFirst, false);
        for stack in [TERMINAL, launcher, SIDEBAR] {
            assert_fires(cx, "ctrl-k", stack, Some(&CommandPalette));
        }
        assert_fires(cx, "ctrl-shift-p", TERMINAL, None);
        assert_fires(cx, "ctrl-k", &["Root", "Settings", "TextInput"], None);

        install_bindings(cx, &overrides, TerminalCtrlKeys::TerminalFirst, false);
        assert_fires(cx, "ctrl-k", TERMINAL, None);
        for stack in [launcher, SIDEBAR] {
            assert_fires(cx, "ctrl-k", stack, Some(&CommandPalette));
        }
    });
}
