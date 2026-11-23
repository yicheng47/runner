#[test]
fn saved_local_only_and_legacy_remote_keys_are_ignored_and_dropped() {
    for key in ["localOnly", "remoteAccessEnabled", "remote_access_enabled"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        std::fs::write(
            &path,
            serde_json::json!({(key):true,"appZoom":1.2}).to_string(),
        )
        .unwrap();
        let settings = super::AppSettings::load(&path).unwrap();
        assert_eq!(settings.app_zoom, 1.2);
        settings.save(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(saved.get(key).is_none());
    }
}

use super::*;

#[test]
fn remote_sidebars_default_for_older_settings_and_partial_views() {
    let old: AppSettings = serde_json::from_str(r#"{"sidebarProjectsOpen":false}"#).unwrap();
    assert!(old.remote_sidebars.is_empty());
    assert!(!old.sidebar_projects_open);
    let partial: AppSettings =
        serde_json::from_str(r#"{"remoteSidebars":{"host":{"activeOnly":true}}}"#).unwrap();
    assert_eq!(
        partial.remote_sidebars["host"],
        RemoteSidebarSettings {
            active_only: true,
            ..Default::default()
        }
    );
}

#[test]
fn language_defaults_migrates_and_persists_intent() {
    use runner_app::i18n::{Language, Locale};
    let old: AppSettings = serde_json::from_str(r#"{"appZoom":1.2}"#).unwrap();
    assert_eq!(old.language, Language::System);
    assert_eq!(old.app_zoom, 1.2);
    let unknown: AppSettings = serde_json::from_str(r#"{"language":"future"}"#).unwrap();
    assert_eq!(unknown.language, Language::System);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    for (intent, saved) in [
        (Language::System, "system"),
        (Language::English, "en"),
        (Language::SimplifiedChinese, "zh-hans"),
    ] {
        let settings = AppSettings {
            language: intent,
            ..old.clone()
        };
        settings.save(&path).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["language"], saved);
        assert_eq!(AppSettings::load(&path).unwrap(), settings);
    }
    assert_eq!(
        Language::System.resolve(Some("zh-Hans")),
        Locale::SimplifiedChinese
    );
    assert_eq!(serde_json::to_value(Language::System).unwrap(), "system");
}

#[test]
fn option_as_meta_defaults_and_persists_each_mode() {
    use runner_app::terminal_ime::OptionAsMeta;

    for json in [r#"{"appZoom":1.2}"#, r#"{"optionAsMeta":"future"}"#] {
        let settings: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.option_as_meta, OptionAsMeta::Automatic);
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    for mode in OptionAsMeta::ALL {
        let settings = AppSettings {
            option_as_meta: mode,
            ..AppSettings::default()
        };
        settings.save(&path).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["optionAsMeta"], mode.key());
        assert_eq!(AppSettings::load(&path).unwrap(), settings);
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn terminal_ctrl_keys_default_to_runner_first_and_persist_each_mode() {
    use crate::keymap::TerminalCtrlKeys;

    for json in [r#"{"appZoom":1.2}"#, r#"{"terminalCtrlKeys":"future"}"#] {
        let settings: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.terminal_ctrl_keys(), TerminalCtrlKeys::RunnerFirst);
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    for (mode, saved) in [
        (TerminalCtrlKeys::RunnerFirst, "runner-first"),
        (TerminalCtrlKeys::TerminalFirst, "terminal-first"),
    ] {
        let settings = AppSettings {
            terminal_ctrl_keys: mode,
            ..AppSettings::default()
        };
        settings.save(&path).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["terminalCtrlKeys"], saved);
        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded, settings);
        assert_eq!(loaded.terminal_ctrl_keys(), mode);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn terminal_ctrl_keys_do_not_exist_on_macos() {
    let settings: AppSettings =
        serde_json::from_str(r#"{"terminalCtrlKeys":"terminal-first"}"#).unwrap();
    assert_eq!(
        settings.terminal_ctrl_keys(),
        crate::keymap::TerminalCtrlKeys::RunnerFirst
    );
    let json = serde_json::to_value(&settings).unwrap();
    assert!(json.get("terminalCtrlKeys").is_none());
}

#[test]
fn file_link_editor_defaults_to_the_default_app_for_pre_feature_settings() {
    let settings: AppSettings = serde_json::from_str(r#"{"appZoom":1.0}"#).unwrap();
    assert_eq!(settings.file_link_editor, FileLinkEditor::DefaultApp);

    let json = serde_json::to_string(&AppSettings {
        file_link_editor: FileLinkEditor::VsCode,
        ..AppSettings::default()
    })
    .unwrap();
    assert!(json.contains(r#""fileLinkEditor":"vscode""#), "{json}");
    let reloaded: AppSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(reloaded.file_link_editor, FileLinkEditor::VsCode);
}

#[test]
fn side_panel_settings_default_to_todays_behaviour_and_round_trip() {
    use crate::file_icons::IconSet;
    let old: AppSettings = serde_json::from_str(r#"{"appZoom":1.0}"#).unwrap();
    assert!(old.show_side_panel && old.files_view && old.changes_view);
    assert_eq!(old.file_icons, IconSet::Catppuccin);
    assert_eq!(old.files_font_family, None);
    assert_eq!(old.files_font_size, None);
    assert_eq!(old.files_font_size(), TERMINAL_FONT_SIZE_DEFAULT);
    assert_eq!(old.markdown_open_as, MarkdownOpenAs::Rendered);
    assert_eq!(old, AppSettings::default());

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ui-settings.json");
    let settings = AppSettings {
        show_side_panel: false,
        files_view: false,
        changes_view: false,
        file_icons: IconSet::Minimal,
        files_font_family: Some(TerminalFontFamily::new("Berkeley Mono")),
        files_font_size: Some(17),
        markdown_open_as: MarkdownOpenAs::Source,
        ..AppSettings::default()
    };
    settings.save(&path).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(json["showSidePanel"], false);
    assert_eq!(json["filesView"], false);
    assert_eq!(json["changesView"], false);
    assert!(json.get("infoView").is_none());
    assert!(json.get("chatPanelOpen").is_none());
    assert!(json.get("missionRailOpen").is_none());
    assert_eq!(json["fileIcons"], "minimal");
    assert_eq!(json["filesFontFamily"], "Berkeley Mono");
    assert_eq!(json["filesFontSize"], 17);
    assert_eq!(json["markdownOpenAs"], "source");
    assert_eq!(AppSettings::load(&path).unwrap(), settings);
    // Legacy side-panel keys (`infoView`, `chatPanelOpen`, `missionRailOpen`,
    // `filesSingleClick`, `newChatPanel`, `filesSyntaxHighlighting`) are
    // ignored cleanly on load.
    let saved: AppSettings = serde_json::from_str(
            r#"{"appZoom":1.0,"infoView":false,"chatPanelOpen":false,"missionRailOpen":false,"filesSingleClick":false,"newChatPanel":"info","filesSyntaxHighlighting":false}"#,
        )
        .unwrap();
    assert_eq!(saved, AppSettings::default());
    for set in IconSet::ALL {
        assert_eq!(IconSet::from_key(set.key()), Some(set));
    }
    for mode in MarkdownOpenAs::ALL {
        assert_eq!(MarkdownOpenAs::parse(mode.key()), Some(mode));
    }
}

#[test]
fn bad_side_panel_values_normalize_to_their_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ui-settings.json");
    for (json, size) in [
        (
            r#"{"fileIcons":"seti","markdownOpenAs":"preview","newChatPanel":"changes","filesFontFamily":"  ","filesFontSize":0}"#,
            None,
        ),
        (
            r#"{"filesFontFamily":"","filesFontSize":99}"#,
            Some(TERMINAL_FONT_SIZE_MAX),
        ),
        (
            r#"{"filesFontFamily":null,"filesFontSize":3}"#,
            Some(TERMINAL_FONT_SIZE_MIN),
        ),
    ] {
        fs::write(&path, json).unwrap();
        let settings = AppSettings::load(&path).unwrap();
        assert_eq!(settings.file_icons, crate::file_icons::IconSet::Catppuccin);
        assert_eq!(settings.markdown_open_as, MarkdownOpenAs::Rendered);
        assert_eq!(settings.files_font_family, None, "{json}");
        assert_eq!(settings.files_font_size, size, "{json}");
    }
}

#[test]
fn the_viewer_font_follows_the_terminal_and_a_missing_one_draws_bundled() {
    let terminal = AppSettings {
        terminal_font_size: 15,
        ..AppSettings::default()
    };
    assert_eq!(terminal.files_font_size(), 15, "follows the terminal");
    let own = AppSettings {
        files_font_size: Some(11),
        ..terminal.clone()
    };
    assert_eq!(own.files_font_size(), 11);
    assert_eq!(
        AppSettings {
            files_font_size: None,
            ..own
        }
        .files_font_size(),
        15,
        "Reset follows the terminal again"
    );

    // A saved font that is not installed draws as the bundled font,
    // and the choice survives a save.
    let missing = TerminalFontFamily::new("Gone Mono");
    assert_eq!(
        missing.resolved_font(false).family.as_ref(),
        crate::terminal::fonts::BUNDLED_FONT
    );
    assert_eq!(missing.resolved_font(true).family.as_ref(), "Gone Mono");
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ui-settings.json");
    AppSettings {
        files_font_family: Some(missing.clone()),
        ..AppSettings::default()
    }
    .save(&path)
    .unwrap();
    assert_eq!(
        AppSettings::load(&path).unwrap().files_font_family,
        Some(missing)
    );
}

#[test]
fn old_mission_permission_setting_is_ignored() {
    let settings: AppSettings =
        serde_json::from_str(r#"{"missionPermissionMode":"auto"}"#).unwrap();
    let saved = serde_json::to_string(&settings).unwrap();
    assert!(!saved.contains("missionPermissionMode"));
}

#[test]
fn zoom_values_snap_and_nudge_within_the_shipped_domain() {
    assert_eq!(normalize_zoom(1.16), 1.2);
    assert_eq!(normalize_zoom(99.), 2.);
    assert_eq!(normalize_zoom(0.01), 0.6);
    assert_eq!(normalize_zoom(-1.), 1.);
    assert_eq!(normalize_zoom(f32::NAN), 1.);
    assert_eq!(nudge_zoom(1., 1), 1.1);
    assert_eq!(nudge_zoom(0.6, -1), 0.6);
    assert_eq!(nudge_zoom(2., 1), 2.);
    assert_eq!(nudge_zoom(1.3, 0), 1.);
}

#[test]
fn sidebar_width_and_collapse_round_trip() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ui-settings.json");
    let settings = AppSettings {
        sidebar_width: 376.,
        sidebar_active_only: true,
        sidebar_collapsed: true,
        ..AppSettings::default()
    };
    settings.save(&path).unwrap();

    let loaded = AppSettings::load(&path).unwrap();
    assert_eq!(loaded.sidebar_width, 376.);
    assert!(loaded.sidebar_collapsed);
    assert!(loaded.sidebar_active_only);
}

#[test]
fn list_page_sizes_default_and_persist_independently() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ui-settings.json");
    std::fs::write(&path, "{}").unwrap();
    let old = AppSettings::load(&path).unwrap();
    assert_eq!((old.roles_page_size, old.crews_page_size), (10, 10));
    let settings = AppSettings {
        roles_page_size: 25,
        crews_page_size: 50,
        ..old
    };
    settings.save(&path).unwrap();
    let loaded = AppSettings::load(&path).unwrap();
    assert_eq!((loaded.roles_page_size, loaded.crews_page_size), (25, 50));
    std::fs::write(&path, r#"{"rolesPageSize":0,"crewsPageSize":8}"#).unwrap();
    let invalid = AppSettings::load(&path).unwrap();
    assert_eq!((invalid.roles_page_size, invalid.crews_page_size), (10, 10));
}

#[test]
fn terminal_and_chat_panel_settings_follow_the_shipped_domains() {
    assert_eq!(normalize_chat_panel_width(376.), 376.);
    assert_eq!(normalize_chat_panel_width(f32::NAN), CHAT_PANEL_DEFAULT);
    assert_eq!(clamp_chat_panel_width(999.), 480.);
    assert_eq!(normalize_file_viewer_width(f32::NAN), 460.);
    assert_eq!(normalize_file_viewer_width(9999.), 460.);
    assert_eq!(clamp_file_viewer_width(9999.), 1200.);
    assert_eq!(normalize_file_tree_width(320.), 320.);
    assert_eq!(normalize_file_tree_width(f32::NAN), 240.);
    assert_eq!(normalize_file_tree_width(100.), 240.);
    assert_eq!(normalize_file_tree_width(481.), 240.);
    assert_eq!(clamp_file_tree_width(100.), 160.);
    assert_eq!(clamp_file_tree_width(9999.), 480.);

    let serialized = serde_json::to_value(AppSettings {
        terminal_cursor_style: TerminalCursorStyle::Bar,
        chat_panel_width: 440.,
        file_viewer_width: 620.,
        ..AppSettings::default()
    })
    .unwrap();
    assert_eq!(serialized["terminalCursorStyle"], "bar");
    assert!(serialized.get("terminalScrollback").is_none());
    assert!(serialized.get("chatPanelOpen").is_none());
    assert!(serialized.get("changesViewerWidth").is_none());
    assert_eq!(serialized["chatPanelWidth"], 440.);
    assert_eq!(serialized["fileViewerWidth"], 620.);

    // Legacy `changesViewerWidth` deserializes cleanly without error and is dropped on save.
    let legacy: AppSettings =
        serde_json::from_str(r#"{"changesViewerWidth":520,"fileViewerWidth":540}"#).unwrap();
    assert_eq!(legacy.file_viewer_width, 540.);
    let legacy_only: AppSettings = serde_json::from_str(r#"{"changesViewerWidth":520}"#).unwrap();
    assert_eq!(legacy_only.file_viewer_width, FILE_VIEWER_DEFAULT);
    assert!(serde_json::to_value(&legacy_only)
        .unwrap()
        .get("changesViewerWidth")
        .is_none());
}

#[test]
fn mission_workspace_preferences_round_trip_and_normalize() {
    let mut settings = AppSettings {
        mission_rail_width: 412.,
        ..AppSettings::default()
    };
    settings
        .last_mission_terminal_ids
        .insert("mission".into(), "session".into());
    let value = serde_json::to_value(&settings).unwrap();
    assert!(value.get("missionRailOpen").is_none());
    assert_eq!(value["missionRailWidth"], 412.);
    assert_eq!(value["lastMissionTerminalIds"]["mission"], "session");

    settings.mission_rail_width = f32::NAN;
    settings
        .last_mission_terminal_ids
        .insert(String::new(), String::new());
    settings.normalize();
    assert_eq!(settings.mission_rail_width, MISSION_RAIL_DEFAULT);
    assert_eq!(settings.last_mission_terminal_ids.len(), 1);
}

#[test]
fn invalid_persisted_values_follow_react_fallbacks() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ui-settings.json");
    fs::write(
            &path,
            r#"{"appZoom":1.26,"windowWidth":0,"windowHeight":-1,"terminalFontSize":99,"sidebarWidth":900}"#,
        )
        .unwrap();

    let loaded = AppSettings::load(&path).unwrap();
    assert_eq!(loaded.app_zoom, 1.3);
    assert_eq!(loaded.terminal_font_size, TERMINAL_FONT_SIZE_MAX);
    assert_eq!(loaded.sidebar_width, SIDEBAR_DEFAULT);
    assert!(!loaded.sidebar_collapsed);
    assert!(!loaded.sidebar_active_only);
    assert!(loaded.sidebar_projects_open);
    assert!(loaded.sidebar_chats_open);

    assert_eq!(normalize_terminal_font_size(0), TERMINAL_FONT_SIZE_DEFAULT);
    assert_eq!(normalize_terminal_font_size(1), TERMINAL_FONT_SIZE_MIN);
}

#[test]
fn terminal_settings_default_round_trip_and_normalize() {
    let old: AppSettings =
        serde_json::from_str(r#"{"terminalFontSize":14,"terminalCursorStyle":"bar"}"#).unwrap();
    assert_eq!(old.terminal_font_size, 14);
    assert_eq!(old.terminal_line_height, 1.4);
    assert!(!old.terminal_cursor_blink);
    assert_eq!(old.terminal_scrollback_lines, 10_000);
    assert!(!old.terminal_copy_on_select);

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ui-settings.json");
    let settings = AppSettings {
        terminal_font_size: 8,
        terminal_line_height: 1.1,
        terminal_cursor_blink: true,
        terminal_scrollback_lines: 50_000,
        terminal_copy_on_select: true,
        ..AppSettings::default()
    };
    settings.save(&path).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(json["terminalLineHeight"], serde_json::json!(1.1));
    assert_eq!(json["terminalCursorBlink"], true);
    assert_eq!(json["terminalScrollbackLines"], 50_000);
    assert_eq!(json["terminalCopyOnSelect"], true);
    assert_eq!(AppSettings::load(&path).unwrap(), settings);

    fs::write(
        &path,
        r#"{"terminalFontSize":40,"terminalLineHeight":1.94,"terminalScrollbackLines":7000}"#,
    )
    .unwrap();
    let loaded = AppSettings::load(&path).unwrap();
    assert_eq!(loaded.terminal_font_size, 32);
    assert_eq!(loaded.terminal_line_height, 1.6);
    assert_eq!(loaded.terminal_scrollback_lines, 7_000);

    assert_eq!(normalize_terminal_font_size(7), 8);
    assert_eq!(normalize_terminal_font_size(20), 20);
    assert_eq!(normalize_terminal_line_height(1.24), 1.2);
    assert_eq!(normalize_terminal_line_height(1.25), 1.3);
    assert_eq!(normalize_terminal_line_height(0.5), 1.0);
    assert_eq!(normalize_terminal_line_height(f32::NAN), 1.4);
    assert_eq!(normalize_terminal_scrollback(0), 1_000);
    assert_eq!(normalize_terminal_scrollback(1_000_000), 50_000);
    for preset in TERMINAL_SCROLLBACK_PRESETS {
        assert_eq!(normalize_terminal_scrollback(preset), preset);
    }
}

#[test]
fn a_skipped_update_version_survives_a_restart() {
    let settings = AppSettings {
        skipped_update_version: Some("0.19.5".into()),
        ..AppSettings::default()
    };
    let saved = serde_json::to_string(&settings).unwrap();
    let loaded: AppSettings = serde_json::from_str(&saved).unwrap();
    assert_eq!(loaded.skipped_update_version.as_deref(), Some("0.19.5"));
    let older: AppSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(older.skipped_update_version, None);
}

#[test]
fn persisted_settings_use_the_current_contract() {
    let value = serde_json::to_value(AppSettings::default()).unwrap();
    assert_eq!(value["appTheme"], "auto");
    assert_eq!(value["lightAppTheme"], "runner-light");
    assert_eq!(value["darkAppTheme"], "carbon");
    assert!(value.get("lightTerminalTheme").is_none());
    assert!(value.get("darkTerminalTheme").is_none());
    assert!(value.get("terminalTheme").is_none());
    assert_eq!(
        value["terminalFontFamily"],
        crate::terminal::fonts::BUNDLED_FONT
    );
    assert_eq!(value["defaultCrewId"], "");
    assert_eq!(value["defaultWorkingDir"], "");
    assert_eq!(value["resumeOnLaunch"], true);
    assert_eq!(value["automaticallyCheckForUpdates"], true);
    assert!(value["skippedUpdateVersion"].is_null());
    assert_eq!(value["defaultRuntime"], "");
    assert_eq!(value["initializedCommandInstall"], false);
    assert_eq!(value["runnerSkillEnabled"], true);
    assert_eq!(value["keymapOverrides"], serde_json::json!({}));
}

#[test]
fn legacy_theme_settings_migrate_and_drop_terminal_theme_keys_on_save() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ui-settings.json");

    for json in [
        r#"{"terminalTheme":"catppuccin-mocha","lightTerminalTheme":"runner","darkTerminalTheme":"catppuccin-mocha","lightAppTheme":"codex","darkAppTheme":"unknown-dark","sidebarWidth":376}"#,
        r#"{"lightTerminalTheme":"rose-pine-dawn","darkTerminalTheme":"runner","sidebarWidth":376}"#,
        r#"{"sidebarWidth":376}"#,
    ] {
        fs::write(&path, json).unwrap();
        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded.light_app_theme, LightTheme::RunnerLight);
        assert_eq!(loaded.dark_app_theme, DarkTheme::Runner);
        assert_eq!(loaded.sidebar_width, 376.);

        loaded.save(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(saved.get("lightTerminalTheme").is_none());
        assert!(saved.get("darkTerminalTheme").is_none());
        assert!(saved.get("terminalTheme").is_none());
    }

    fs::write(
            &path,
            r#"{"lightAppTheme":"gruvbox-light","darkAppTheme":"tokyo-night","lightTerminalTheme":"runner","darkTerminalTheme":"catppuccin-mocha","terminalTheme":"runner","sidebarWidth":376}"#,
        )
        .unwrap();
    let loaded = AppSettings::load(&path).unwrap();
    assert_eq!(loaded.light_app_theme, LightTheme::GruvboxLight);
    assert_eq!(loaded.dark_app_theme, DarkTheme::TokyoNight);
    assert_eq!(loaded.sidebar_width, 376.);

    loaded.save(&path).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["lightAppTheme"], "gruvbox-light");
    assert_eq!(saved["darkAppTheme"], "tokyo-night");
    assert!(saved.get("lightTerminalTheme").is_none());
    assert!(saved.get("darkTerminalTheme").is_none());
    assert!(saved.get("terminalTheme").is_none());
}

#[test]
fn app_themes_round_trip_every_light_and_dark_choice() {
    for (light, dark) in LightTheme::ALL.into_iter().zip(DarkTheme::ALL) {
        let json = serde_json::to_value(AppSettings {
            light_app_theme: light,
            dark_app_theme: dark,
            ..AppSettings::default()
        })
        .unwrap();
        assert_eq!(json["lightAppTheme"], light.key());
        assert_eq!(json["darkAppTheme"], dark.key());
        assert!(json.get("lightTerminalTheme").is_none());
        assert!(json.get("darkTerminalTheme").is_none());
        assert!(json.get("terminalTheme").is_none());

        let reloaded: AppSettings = serde_json::from_value(json).unwrap();
        assert_eq!(reloaded.light_app_theme, light);
        assert_eq!(reloaded.dark_app_theme, dark);
        assert_eq!(LightTheme::parse(light.key()), Some(light));
        assert_eq!(DarkTheme::parse(dark.key()), Some(dark));
    }

    assert_eq!(LightTheme::parse("unknown"), None);
    assert_eq!(DarkTheme::parse("runner-dark"), None);
    let fallback: AppSettings =
        serde_json::from_str(r#"{"lightAppTheme":"invalid","darkAppTheme":"invalid"}"#).unwrap();
    assert_eq!(fallback.light_app_theme, LightTheme::RunnerLight);
    assert_eq!(fallback.dark_app_theme, DarkTheme::Runner);
}

#[test]
fn terminal_palette_comes_from_the_resolved_variant() {
    use runner_terminal::palette;
    assert_eq!(
        terminal_palette(&AppSettings::default(), ThemeVariant::RunnerLight),
        palette::ROSE_PINE_DAWN
    );
    assert_eq!(
        terminal_palette(&AppSettings::default(), ThemeVariant::Carbon),
        palette::RUNNER
    );
    for variant in ThemeVariant::ALL {
        assert_eq!(
            terminal_palette(&AppSettings::default(), variant),
            variant.terminal_palette(),
            "{variant:?}"
        );
    }
}

#[test]
fn keymap_overrides_round_trip_null_and_normalize_bad_entries() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ui-settings.json");
    fs::write(
        &path,
        r#"{
                "keymapOverrides": {
                    "command-palette": null,
                    "toggle-sidebar": {
                        "meta": true,
                        "ctrl": false,
                        "alt": false,
                        "shift": false,
                        "code": "KeyP"
                    },
                    "zoom-in": { "meta": "yes", "code": "Equal" },
                    "future-action": null
                }
            }"#,
    )
    .unwrap();

    let loaded = AppSettings::load(&path).unwrap();
    assert_eq!(loaded.keymap_overrides.len(), 2);
    assert!(loaded.keymap_overrides["command-palette"].is_none());
    assert_eq!(
        loaded.keymap_overrides["toggle-sidebar"]
            .as_ref()
            .unwrap()
            .code,
        "KeyP"
    );
    loaded.save(&path).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(saved["keymapOverrides"]["command-palette"].is_null());
    assert!(saved["keymapOverrides"].get("future-action").is_none());

    fs::write(&path, r#"{"keymapOverrides": ["bad"]}"#).unwrap();
    assert!(AppSettings::load(&path)
        .unwrap()
        .keymap_overrides
        .is_empty());
}

#[test]
fn terminal_fonts_migrate_each_legacy_choice() {
    assert_eq!(
        AppSettings::default().terminal_font_family.family(),
        crate::terminal::fonts::BUNDLED_FONT
    );
    for (saved, expected) in [
        ("JetBrains Mono", crate::terminal::fonts::BUNDLED_FONT),
        ("Meslo Nerd Font", crate::terminal::fonts::BUNDLED_FONT),
        ("Menlo", runner_app::theme::SYSTEM_MONOSPACE_FONT),
        ("System default", "System default"),
        ("Monaco", "Monaco"),
        ("SF Mono", "SF Mono"),
        ("Fira Code", "Fira Code"),
    ] {
        let parsed: TerminalFontFamily = serde_json::from_value(serde_json::json!(saved)).unwrap();
        assert_eq!(parsed.family(), expected, "{saved}");
    }
}

#[test]
fn missing_terminal_font_falls_back_without_overwriting_saved_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ui-settings.json");
    fs::write(&path, r#"{"terminalFontFamily":"Runner Missing Font 931"}"#).unwrap();
    let settings = AppSettings::load(&path).unwrap();
    assert_eq!(
        settings
            .terminal_font_family
            .resolved_font(false)
            .family
            .as_ref(),
        crate::terminal::fonts::BUNDLED_FONT
    );
    settings.save(&path).unwrap();
    assert_eq!(
        AppSettings::load(&path)
            .unwrap()
            .terminal_font_family
            .family(),
        "Runner Missing Font 931"
    );
}

#[test]
fn terminal_fonts_use_bundled_icons_and_platform_fallbacks() {
    let family = TerminalFontFamily::new("SF Mono");
    let font = family.resolved_font(true);
    assert_eq!(font.family.as_ref(), "SF Mono");
    let expected = std::iter::once(crate::terminal::fonts::BUNDLED_FONT)
        .chain(
            runner_app::platform_fonts::TERMINAL_FONT_FALLBACKS
                .iter()
                .copied(),
        )
        .collect::<Vec<_>>();
    assert_eq!(font.fallbacks.unwrap().fallback_list(), expected);
}

#[test]
fn agent_enablement_preserves_shipped_defaults_and_explicit_overrides() {
    let default_enabled = runner_core::protocol::runtime_metadata::runtime_default_enabled;
    let mut settings = AppSettings::default();
    assert_eq!(
        Runtime::ALL
            .into_iter()
            .filter(|runtime| settings.is_agent_enabled(*runtime, default_enabled(*runtime)))
            .collect::<Vec<_>>(),
        [
            Runtime::Codex,
            Runtime::ClaudeCode,
            Runtime::Antigravity,
            Runtime::Pi,
            Runtime::Copilot,
            Runtime::Cursor,
            Runtime::Grok,
            Runtime::OpenCode,
        ]
    );
    settings.disabled_agents.extend([
        "codex".into(),
        "copilot".into(),
        "cursor".into(),
        "grok".into(),
        "opencode".into(),
    ]);
    settings.enabled_agents.insert("trae".into());
    let settings: AppSettings =
        serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
    for runtime in [
        Runtime::Codex,
        Runtime::Copilot,
        Runtime::Cursor,
        Runtime::Grok,
        Runtime::OpenCode,
    ] {
        assert!(!settings.is_agent_enabled(runtime, default_enabled(runtime)));
    }
    assert!(settings.is_agent_enabled(Runtime::Trae, default_enabled(Runtime::Trae)));
}

#[test]
fn cursor_and_grok_model_discovery_follow_defaults_and_explicit_disablement() {
    for runtime in [Runtime::Cursor, Runtime::Grok, Runtime::OpenCode] {
        let mut settings = AppSettings::default();
        assert!(settings.model_runtimes().contains(&runtime));
        settings.disabled_agents.insert(runtime.key().into());
        assert!(!settings.model_runtimes().contains(&runtime));
    }
}

#[test]
fn app_font_is_inter_with_the_platform_fallback_chain() {
    let inter = app_font();
    assert_eq!(inter.family.as_ref(), "Inter");
    #[cfg(windows)]
    assert_eq!(
        inter.fallbacks.unwrap().fallback_list(),
        ["Segoe UI", "Microsoft YaHei"]
    );
    #[cfg(target_os = "linux")]
    assert_eq!(
        inter.fallbacks.clone().unwrap().fallback_list(),
        runner_app::platform_fonts::APP_FONT_FALLBACKS
    );
    #[cfg(target_os = "macos")]
    assert_eq!(
        inter.fallbacks.unwrap().fallback_list(),
        [
            "Inter Variable",
            "Segoe UI",
            "Microsoft YaHei",
            "sans-serif"
        ]
    );
}

#[test]
fn legacy_app_font_choices_still_load() {
    for legacy in ["Inter", "Geist", "Roboto", "System UI"] {
        let json = format!(r#"{{"appFontFamily":"{legacy}","appZoom":1.25}}"#);
        let settings: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(settings.app_zoom, 1.25, "{legacy}");
    }
    let value = serde_json::to_value(AppSettings::default()).unwrap();
    assert!(value.get("appFontFamily").is_none());
}

#[test]
fn settings_without_initialized_mcp_clients_decode_to_empty() {
    let settings: AppSettings = serde_json::from_str(r#"{"appZoom":1.25}"#).unwrap();
    assert!(settings.initialized_mcp_clients.is_empty());
    let settings: AppSettings =
        serde_json::from_str(r#"{"initializedMcpClients":["codex"]}"#).unwrap();
    assert_eq!(
        settings.initialized_mcp_clients,
        ["codex".to_owned()].into_iter().collect()
    );
    assert!(!settings.mcp_registrations_removed);
}

#[test]
fn settings_without_initialized_skill_roots_decode_to_empty() {
    let settings: AppSettings = serde_json::from_str(r#"{"appZoom":1.25}"#).unwrap();
    assert!(settings.initialized_skill_roots.is_empty());
    let settings: AppSettings =
        serde_json::from_str(r#"{"initializedSkillRoots":[".agents/skills"]}"#).unwrap();
    assert_eq!(
        settings.initialized_skill_roots,
        [".agents/skills".to_owned()].into_iter().collect()
    );
    assert!(settings.runner_skill_enabled);
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn automatic_downloads_default_on_and_persist_independently_of_checks() {
    let mut settings: AppSettings =
        serde_json::from_str(r#"{"automaticallyCheckForUpdates":false}"#).unwrap();
    assert!(settings.automatically_download_updates);
    settings.automatically_download_updates = false;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    settings.save(&path).unwrap();
    let loaded = AppSettings::load(&path).unwrap();
    assert!(!loaded.automatically_download_updates);
    assert!(!loaded.automatically_check_for_updates);
}
