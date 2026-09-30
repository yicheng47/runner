use super::logic::runtime_default_effort_label;
use super::logic::runtime_model_placeholder;
use super::logic::validate_role_handle;
use super::*;
use runner_backend::model::Runtime;

fn runtime_with_defaults(
    default_model: Option<&str>,
    default_effort: Option<&str>,
) -> RuntimeCatalogEntry {
    RuntimeCatalogEntry {
        name: Runtime::Codex,
        display_name: "Codex".into(),
        command: "codex".into(),
        native_fork: true,
        description: "OpenAI Codex CLI".into(),
        install_url: String::new(),
        default_enabled: true,
        available: true,
        default_model: default_model.map(str::to_owned),
        default_effort: default_effort.map(str::to_owned),
        models: Vec::new(),
        efforts: Vec::new(),
    }
}

#[test]
fn role_edit_restores_disabled_runtimes_in_selector_order() {
    let page = role_page_harness("role-runtime-order");
    let mut role = create_test_role(&page.core, "restored-runtime", None);
    role.runtime = "codex".into();
    let mut runtimes = runner_backend::ops::runtime::runtime_catalog(&page.core)
        .unwrap()
        .into_iter()
        .filter(|entry| !matches!(entry.name, Runtime::Codex | Runtime::Pi))
        .map(|mut entry| {
            entry.available = true;
            entry
        })
        .collect();

    super::logic::ensure_runtime_present(&page.core, &mut runtimes, &role.runtime);
    super::logic::ensure_runtime_present(&page.core, &mut runtimes, "pi");
    let options = super::logic::role_edit_runtime_options(&runtimes, &role, "pi");
    assert_eq!(
        options
            .iter()
            .map(|option| option.value.as_str())
            .collect::<Vec<_>>(),
        [
            "codex",
            "claude-code",
            "antigravity",
            "pi",
            "copilot",
            "trae"
        ]
    );
}

#[test]
fn role_handle_validation_matches_the_shipped_contract() {
    for valid in ["", "a", "0", "coder-2", "coder_2", &"a".repeat(32)] {
        assert_eq!(validate_role_handle(valid), None, "{valid}");
    }
    for invalid in ["Coder", "-coder", "_coder", "coder!", &"a".repeat(33)] {
        assert!(validate_role_handle(invalid).is_some(), "{invalid}");
    }
}

#[test]
fn runtime_default_labels_include_known_values() {
    let runtimes = [runtime_with_defaults(Some("gpt-5.6-sol"), Some("xhigh"))];
    assert_eq!(
        runtime_model_placeholder(&runtimes, "codex", None),
        "default (gpt-5.6-sol)"
    );
    assert_eq!(
        runtime_default_effort_label(&runtimes, "codex"),
        "Runtime default (xhigh)"
    );

    let runtimes = [runtime_with_defaults(None, None)];
    assert_eq!(
        runtime_model_placeholder(&runtimes, "codex", None),
        "default"
    );
    assert_eq!(
        runtime_default_effort_label(&runtimes, "codex"),
        "Runtime default"
    );
}

#[test]
fn default_role_model_offers_efforts_and_filters_known_capabilities() {
    use runner_backend::ops::runtime::RuntimeCatalogOption;
    let mut runtime = runtime_with_defaults(Some("limited-model"), None);
    runtime.efforts = ["", "low", "high"]
        .into_iter()
        .map(|value| RuntimeCatalogOption {
            value: value.into(),
            label: value.into(),
            description: None,
            supported_efforts: None,
        })
        .collect();
    runtime.models = vec![RuntimeCatalogOption {
        value: "limited-model".into(),
        label: "Limited".into(),
        description: None,
        supported_efforts: Some(vec!["low".into()]),
    }];
    let values = |runtime: &RuntimeCatalogEntry, model| {
        super::logic::create_role_effort_options(std::slice::from_ref(runtime), "codex", model)
            .into_iter()
            .map(|option| option.value)
            .collect::<Vec<_>>()
    };
    assert_eq!(values(&runtime, ""), ["", "low"]);
    assert_eq!(values(&runtime, "  "), ["", "low"]);
    assert_eq!(values(&runtime, "custom-model"), ["", "low", "high"]);
    runtime.default_model = None;
    assert_eq!(values(&runtime, ""), ["", "low", "high"]);
    assert_eq!(values(&runtime, "limited-model"), ["", "low"]);
    runtime.default_model = Some("custom-default".into());
    assert_eq!(values(&runtime, ""), ["", "low", "high"]);
    runtime.default_model = Some("limited-model".into());
    runtime.models[0].supported_efforts = Some(Vec::new());
    assert_eq!(values(&runtime, ""), [""]);
}

#[test]
fn prompt_meta_counts_lines_and_size() {
    use super::logic::prompt_meta;

    assert_eq!(prompt_meta("one line"), "1 line · 8 B");
    assert_eq!(prompt_meta("a\nb\n"), "2 lines · 4 B");
    let prompt = "x".repeat(60) + "\n";
    assert_eq!(prompt_meta(&prompt.repeat(48)), "48 lines · 2.9 KB");
}

#[test]
fn prompt_preview_clamps_only_long_prompts_at_a_line_boundary() {
    use crate::surfaces::profile_page::{prompt_preview, PREVIEW_LINES as PROMPT_PREVIEW_LINES};

    let fits = (1..=PROMPT_PREVIEW_LINES)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(prompt_preview(&fits), None);
    assert_eq!(
        prompt_preview(&format!("{fits}\n")),
        None,
        "a trailing newline adds no line"
    );

    let long = format!(
        "{fits}\nline {}\nline {}",
        PROMPT_PREVIEW_LINES + 1,
        PROMPT_PREVIEW_LINES + 2
    );
    assert_eq!(prompt_preview(&long), Some(fits.as_str()));
}

#[test]
fn list_cells_read_default_dash_and_live_counts() {
    use super::logic::{crews_label, last_active_label, role_setting_label};
    use chrono::{TimeZone, Utc};
    use runner_backend::ops::role::RoleActivity;

    assert_eq!(role_setting_label(None), ("default".into(), true));
    assert_eq!(role_setting_label(Some("  ")), ("default".into(), true));
    assert_eq!(
        role_setting_label(Some("opus[1m]")),
        ("opus[1m]".into(), false)
    );
    assert_eq!(crews_label(0), "—");
    assert_eq!(crews_label(3), "3");

    let now = Utc.with_ymd_and_hms(2026, 9, 26, 12, 0, 0).unwrap();
    let activity = |sessions, missions, started: Option<chrono::DateTime<Utc>>| RoleActivity {
        role_id: "role".into(),
        active_sessions: sessions,
        active_missions: missions,
        crew_count: 0,
        last_started_at: started,
        direct_session_id: None,
    };
    let started = Utc.with_ymd_and_hms(2026, 9, 23, 17, 41, 0).unwrap();
    assert_eq!(
        last_active_label(&activity(2, 1, Some(started)), &now),
        ("2 sessions · 1 mission".into(), true)
    );
    assert_eq!(
        last_active_label(&activity(1, 0, Some(started)), &now),
        ("1 session".into(), true)
    );
    assert_eq!(
        last_active_label(&activity(0, 0, Some(started)), &now),
        ("Sep 23, 17:41".into(), false)
    );
    let last_year = Utc.with_ymd_and_hms(2025, 12, 30, 9, 5, 0).unwrap();
    assert_eq!(
        last_active_label(&activity(0, 0, Some(last_year)), &now),
        ("Dec 30, 2025".into(), false)
    );
    assert_eq!(
        last_active_label(&activity(0, 0, None), &now),
        ("—".into(), false)
    );
}

#[test]
fn the_crews_heading_counts_crews_not_slots() {
    use super::logic::distinct_crew_count;
    use chrono::Utc;
    use runner_backend::ops::slot::CrewMembership;

    let membership = |crew_id: &str, slot_id: &str| CrewMembership {
        crew_id: crew_id.into(),
        crew_name: crew_id.into(),
        slot_id: slot_id.into(),
        slot_handle: slot_id.into(),
        lead: false,
        position: 0,
        added_at: Utc::now(),
    };
    assert_eq!(distinct_crew_count(&[]), 0);
    assert_eq!(
        distinct_crew_count(&[
            membership("pair", "coder"),
            membership("pair", "coder-2"),
            membership("release", "implementer"),
        ]),
        2
    );
}

#[test]
fn short_ids_keep_both_ends() {
    use super::logic::short_id;

    assert_eq!(short_id("01K000DEFAULT000RUNNERREVW01"), "01K0…ERREVW01");
    assert_eq!(short_id("short"), "short");
}

#[test]
fn narrow_tables_drop_columns_and_keep_the_role() {
    use super::logic::{role_table_columns, RoleColumn};

    let all = [
        RoleColumn::Runtime,
        RoleColumn::Model,
        RoleColumn::Effort,
        RoleColumn::Crews,
        RoleColumn::LastActive,
    ];
    assert_eq!(role_table_columns(1136.), all);
    assert_eq!(
        role_table_columns(796.),
        [
            RoleColumn::Runtime,
            RoleColumn::Model,
            RoleColumn::LastActive
        ]
    );
    assert_eq!(role_table_columns(500.), [RoleColumn::LastActive]);
    assert!(role_table_columns(336.).is_empty());
}

struct RolePageHarness {
    visual: gpui::VisualTestContext,
    host: gpui::WindowHandle<crate::NativeRoot>,
    core: crate::AppCore,
    _cx: gpui::TestAppContext,
    _temp: tempfile::TempDir,
    _theme: crate::theme_snapshot::ThemeGuard,
}

fn role_page_harness(label: &str) -> RolePageHarness {
    use crate::theme_snapshot::ThemeGuard;
    use crate::*;
    use gpui::{TestAppContext, VisualTestContext};
    use runner_backend::{db, event_bus, events, mcp, router, session, shell_path, windows};
    use std::sync::{Arc, Mutex, RwLock};

    let theme = ThemeGuard::new();
    theme::set_active_variant(theme::ThemeVariant::Carbon);
    let temp = tempfile::tempdir().unwrap();
    let pool = Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap());
    let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
    let runtime_discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
    let core = AppCore {
        db: pool.clone(),
        app_data_dir: temp.path().to_owned(),
        sessions: session::SessionManager::new(
            runtime_shell_env.clone(),
            runtime_discovery.clone(),
            Arc::new(session::pty_runtime::PtyRuntime::new()),
        ),
        runtime_shell_env,
        runtime_discovery,
        usage: Arc::new(runner_backend::usage::UsageService::default()),
        buses: event_bus::BusRegistry::new(),
        routers: router::RouterRegistry::new(),
        mission_grid_hint: Arc::new(Mutex::new(None)),
        mcp: Arc::new(mcp::McpHandle::new()),
        windows: Arc::new(windows::WindowRegistry::new()),
        events: events::EventChannel::new(),
        session_event_observer: Default::default(),
        app_version: "0.0.0-test".into(),
    };
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
        #[cfg(not(windows))]
        let updater = cx.new(|cx| crate::Updater::new(false, cx));
        #[cfg(windows)]
        let updater = cx.new(|cx| crate::Updater::new(false, temp.path().join("updates"), cx));
        cx.set_global(crate::GlobalUpdater(updater));
    });
    let label = label.to_owned();
    let host = cx.add_window(|window, cx| {
        NativeRoot::new(
            label,
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    let visual = VisualTestContext::from_window(host.into(), &cx);
    RolePageHarness {
        visual,
        host,
        core,
        _cx: cx,
        _temp: temp,
        _theme: theme,
    }
}

fn create_test_role(core: &crate::AppCore, handle: &str, system_prompt: Option<String>) -> Role {
    create_test_role_with_args(core, handle, system_prompt, Vec::new())
}

fn create_test_role_with_args(
    core: &crate::AppCore,
    handle: &str,
    system_prompt: Option<String>,
    args: Vec<String>,
) -> Role {
    runner_backend::ops::role::role_create(
        core,
        runner_backend::ops::role::CreateRoleInput {
            handle: handle.into(),
            display_name: format!("Role {handle}"),
            runtime: Runtime::ClaudeCode,
            command: "claude".into(),
            args,
            working_dir: Some("/tmp/runner-role-page".into()),
            system_prompt,
            env: Default::default(),
            model: Some("opus".into()),
            effort: None,
            codex_speed: None,
            permission_mode: runner_backend::router::runtime::PermissionMode::AcceptEdits,
        },
    )
    .unwrap()
}

#[test]
fn selected_chat_panel_refreshes_after_a_shared_role_edit() {
    use crate::surfaces::AppRoute;
    use runner_backend::ops::role::{role_list, role_update, UpdateRoleInput};

    let mut page = role_page_harness("chat-panel-role-edit");
    let role = create_test_role(&page.core, "panel-role", None);
    let conn = page.core.db.get().unwrap();
    conn.execute(
        "INSERT INTO sessions (id, role_id, status) VALUES ('panel-chat', ?1, 'stopped')",
        [&role.id],
    )
    .unwrap();
    runner_backend::repo::node::create_tab(
        &conn,
        None,
        "",
        0,
        &runner_app::pane_layout::PaneLayout::single(Some("panel-chat"), &["panel-chat".into()])
            .serialize()
            .unwrap(),
    )
    .unwrap();
    let nodes = runner_backend::repo::node::list(&conn).unwrap();
    let roles = role_list(&page.core).unwrap();
    page.host
        .update(&mut page.visual, |root, _, cx| {
            root.app_store.update(cx, |store, cx| {
                store.replace_roles(roles, cx);
                store.replace_nodes(nodes, cx);
                store.refresh_sessions(cx);
            });
            root.apply_tab_rows(cx);
            root.tabs.activate_session("panel-chat");
            root.route = AppRoute::Chat;
            root.sync_active_chat_detail(cx);
            cx.notify();
        })
        .unwrap();
    page.visual
        .simulate_resize(gpui::size(gpui::px(3000.), gpui::px(900.)));
    page.visual.run_until_parked();
    assert!(page.visual.debug_bounds("COLUMN_TEXT opus").is_some());
    assert!(page.visual.debug_bounds("COLUMN_TEXT default").is_some());
    assert!(page.visual.debug_bounds("COLUMN_TEXT sonnet").is_none());
    assert!(page.visual.debug_bounds("COLUMN_TEXT high").is_none());

    role_update(
        &page.core,
        &role.id,
        UpdateRoleInput {
            model: Some(Some("sonnet".into())),
            effort: Some(Some("high".into())),
            ..Default::default()
        },
    )
    .unwrap();
    let roles = role_list(&page.core).unwrap();
    page.host
        .update(&mut page.visual, |root, _, cx| {
            root.app_store
                .update(cx, |store, cx| store.replace_roles(roles, cx));
        })
        .unwrap();
    page.visual.run_until_parked();
    assert_eq!(
        page.read(|root| root.active_focused_session_id())
            .as_deref(),
        Some("panel-chat")
    );
    assert!(page.visual.debug_bounds("COLUMN_TEXT sonnet").is_some());
    assert!(page.visual.debug_bounds("COLUMN_TEXT high").is_some());
}

impl RolePageHarness {
    fn open_role(&mut self, handle: &str) {
        let handle = handle.to_owned();
        self.host
            .update(&mut self.visual, |root, window, cx| {
                root.open_role_detail(handle, window, cx)
            })
            .unwrap();
        self.visual.run_until_parked();
    }

    fn click(&mut self, selector: &'static str) {
        let bounds = self
            .visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on screen"));
        self.visual
            .simulate_click(bounds.center(), gpui::Modifiers::default());
        self.visual.run_until_parked();
    }

    fn read<T>(&mut self, read: impl FnOnce(&crate::NativeRoot) -> T) -> T {
        self.host
            .update(&mut self.visual, |root, _, _| read(root))
            .unwrap()
    }

    fn in_place_edit(&mut self) -> bool {
        self.read(|root| root.role_surfaces.edit.is_some())
    }
}

#[test]
fn role_page_columns_split_wide_and_stack_at_the_minimum_width() {
    use gpui::{px, size};

    let mut page = role_page_harness("role-page-layout");
    create_test_role(
        &page.core,
        "page-reviewer",
        Some("You are a reviewer in a two-person peer coding loop.\n\n".repeat(40)),
    );
    page.open_role("page-reviewer");
    // 640 wide is the smallest window Runner restores to; the sidebar keeps its default width.
    for width in [640., 900., 1100., 1440.] {
        page.visual.simulate_resize(size(px(width), px(900.)));
        page.visual.run_until_parked();
        let scroll = page.visual.debug_bounds("ROLE_DETAIL_SCROLL").unwrap();
        let container = page.visual.debug_bounds("ROLE_DETAIL_CONTAINER").unwrap();
        let header = page.visual.debug_bounds("ROLE_DETAIL_HEADER").unwrap();
        let profile = page.visual.debug_bounds("ROLE_PAGE_PROFILE").unwrap();
        let prompt = page.visual.debug_bounds("ROLE_PAGE_PROMPT").unwrap();
        let left_slack = container.left() - scroll.left();
        let right_slack = scroll.right() - container.right();
        assert!(
            (left_slack - right_slack).abs() <= px(1.),
            "{width}: container is off-centre, slack {left_slack:?} vs {right_slack:?}"
        );
        for (name, bounds) in [("profile", profile), ("prompt", prompt)] {
            assert!(
                bounds.left() >= header.left() - px(1.)
                    && bounds.right() <= header.right() + px(1.),
                "{width}: {name} {bounds:?} leaves the page {header:?}"
            );
        }
        if width >= 1100. {
            assert!(
                profile.right() + px(1.) < prompt.left(),
                "{width}: the prompt should sit beside the profile, {profile:?} vs {prompt:?}"
            );
            assert_eq!(profile.top(), prompt.top(), "{width}: columns share a top");
        }
        if width <= 640. {
            assert!(
                prompt.top() >= profile.bottom(),
                "{width}: the prompt should wrap under the profile, {profile:?} vs {prompt:?}"
            );
        }
        let actions = page.visual.debug_bounds("ROLE_PAGE_ACTIONS").unwrap();
        assert!(
            actions.size.width <= px(ROLE_COLUMN_WIDTH + 1.),
            "{width}: Chat now and Edit keep the column's width: {actions:?}"
        );
        if prompt.top() >= profile.bottom() {
            assert!(
                (profile.size.width - prompt.size.width).abs() <= px(1.),
                "{width}: a stacked profile spans the page like the prompt: {profile:?} vs {prompt:?}"
            );
        } else {
            assert!(
                (profile.size.width - px(ROLE_COLUMN_WIDTH)).abs() <= px(1.),
                "{width}: beside the prompt the profile keeps its column: {profile:?}"
            );
        }
    }
    assert!(
        page.visual.debug_bounds("ENTITY_SIDEBAR_TOGGLE").is_none(),
        "the shell must not add an open-sidebar cluster while the sidebar is open"
    );
}

#[test]
fn entity_pages_keep_the_sidebar_cluster_when_it_collapses() {
    use crate::surfaces::AppRoute;
    use gpui::{px, size};

    let mut page = role_page_harness("role-page-cluster");
    create_test_role(&page.core, "page-reviewer", None);
    page.open_role("page-reviewer");
    page.visual.simulate_resize(size(px(1100.), px(900.)));
    page.visual.run_until_parked();
    let host = page.host;
    for route in [
        AppRoute::RoleDetail("page-reviewer".into()),
        AppRoute::Roles,
        AppRoute::Crews,
        AppRoute::CrewEditor("crew".into()),
    ] {
        host.update(&mut page.visual, |root, _, cx| {
            root.set_sidebar_collapsed(true, false, cx);
            root.route = route.clone();
            cx.notify();
        })
        .unwrap();
        page.visual.run_until_parked();
        #[cfg(not(target_os = "macos"))]
        assert!(
            page.visual.debug_bounds("ENTITY_SIDEBAR_TOGGLE").is_none(),
            "{route:?}: the platform chrome owns the sidebar toggle off macOS"
        );
        #[cfg(target_os = "macos")]
        {
            let toggle = page
                .visual
                .debug_bounds("ENTITY_SIDEBAR_TOGGLE")
                .unwrap_or_else(|| {
                    panic!("{route:?}: a collapsed sidebar leaves no open-sidebar cluster")
                });
            let padding = host
                .update(&mut page.visual, |root, window, cx| {
                    root.workspace_titlebar_padding(window, cx)
                })
                .unwrap();
            let column = page.visual.debug_bounds("APP_CONTENT_COLUMN").unwrap();
            assert_eq!(toggle.top(), column.top(), "{route:?}");
            assert!(
                (toggle.left() - column.left() - px(padding)).abs() <= px(1.),
                "{route:?}: {toggle:?} vs column {column:?} and padding {padding}"
            );
            assert_eq!(
                toggle.size.height,
                px(runner_app::ui::WORKSPACE_HEADER_HEIGHT),
                "{route:?}: the cluster row must match the pane header height"
            );
            assert!(
                toggle.size.width > px(3. * 28.),
                "{route:?}: the cluster must carry the page arrows beside the toggle, got {:?}",
                toggle.size.width
            );
        }
    }
}

#[test]
fn role_list_table_fits_the_minimum_window_and_counts_its_rows() {
    use crate::surfaces::AppRoute;
    use gpui::{px, size};

    let mut page = role_page_harness("role-list-layout");
    for index in 0..9 {
        create_test_role(&page.core, &format!("page-role-{index}"), None);
    }
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_roles(window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    assert_eq!(page.read(|root| root.route.clone()), AppRoute::Roles);
    for width in [640., 1100., 1440.] {
        page.visual.simulate_resize(size(px(width), px(700.)));
        page.visual.run_until_parked();
        let rows = page.visual.debug_bounds("ROLE_TABLE_ROWS").unwrap();
        let header = page.visual.debug_bounds("ROLE_TABLE_HEADER").unwrap();
        let actions = page.visual.debug_bounds("ROLE_ROW_ACTIONS").unwrap();
        let new_role = page.visual.debug_bounds("PAGINATED_LIST_ACTION").unwrap();
        let search = page.visual.debug_bounds("PAGINATED_LIST_SEARCH").unwrap();
        assert!(
            new_role.right() <= rows.right() + px(1.) && new_role.size.width > px(40.),
            "{width}: + New role {new_role:?} is pushed off the page {rows:?}"
        );
        assert!(
            header.top() - search.bottom() >= px(32.),
            "{width}: the table header sits too close under the search {search:?} vs {header:?}"
        );
        assert!(
            actions.right() <= rows.right() + px(1.),
            "{width}: row actions {actions:?} overflow the table {rows:?}"
        );
        assert!(
            (header.right() - rows.right()).abs() <= px(1.),
            "{width}: header {header:?} and rows {rows:?} end apart"
        );
    }
    let roles = runner_backend::ops::role::role_list(&page.core)
        .unwrap()
        .len();
    let (filtered, total, searching) = page.read(|root| {
        let list = &root.role_surfaces.list;
        (list.filtered_count, list.total_count, list.searching())
    });
    assert_eq!((filtered, total, searching), (roles, roles, false));
    assert_eq!(
        runner_app::ui::count_label(filtered, total, "roles", searching),
        format!("{roles} roles")
    );
}

// GPUI's test frames keep a selector's bounds once it has been drawn, so a
// "not shown" check only holds for an element that window has never drawn.
#[test]
fn the_prompt_starts_collapsed_and_collapses_again_for_another_role() {
    let mut page = role_page_harness("role-prompt-clamp");
    let long = (1..=40)
        .map(|line| format!("Line {line} of the brief."))
        .collect::<Vec<_>>()
        .join("\n");
    create_test_role(&page.core, "page-long", Some(long));
    create_test_role(
        &page.core,
        "page-other",
        Some(String::from("# Short\n\nOne line.")),
    );
    page.open_role("page-other");
    assert!(page.visual.debug_bounds("ROLE_PROMPT_TEXT").is_some());
    assert!(
        page.visual.debug_bounds("ROLE_PROMPT_TOGGLE").is_none(),
        "a short prompt shows whole, with no toggle"
    );

    page.open_role("page-long");
    assert!(!page.read(|root| root.role_surfaces.prompt_expanded));
    let collapsed = page.visual.debug_bounds("ROLE_PROMPT_TEXT").unwrap();
    page.click("ROLE_PROMPT_TOGGLE");
    assert!(page.read(|root| root.role_surfaces.prompt_expanded));
    let expanded = page.visual.debug_bounds("ROLE_PROMPT_TEXT").unwrap();
    assert!(
        expanded.size.height > collapsed.size.height,
        "expanding should show more of the prompt: {collapsed:?} vs {expanded:?}"
    );
    page.click("ROLE_PROMPT_TOGGLE");
    assert!(
        !page.read(|root| root.role_surfaces.prompt_expanded),
        "the same toggle collapses it again"
    );
    page.click("ROLE_PROMPT_TOGGLE");
    assert!(page.read(|root| root.role_surfaces.prompt_expanded));

    page.open_role("page-other");
    assert!(!page.read(|root| root.role_surfaces.prompt_expanded));
    page.open_role("page-long");
    assert!(!page.read(|root| root.role_surfaces.prompt_expanded));
    assert_eq!(
        page.visual.debug_bounds("ROLE_PROMPT_TEXT").unwrap().size,
        collapsed.size,
        "coming back shows the collapsed prompt"
    );
}

#[test]
fn edit_edits_in_place_and_cancel_discards_the_draft() {
    let mut page = role_page_harness("role-edit-cancel");
    let role = create_test_role(&page.core, "page-coder", Some("Ship it.".into()));
    page.open_role("page-coder");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_role_edit(role.clone(), window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    assert!(page.in_place_edit());
    assert!(page.visual.debug_bounds("ROLE_EDIT_IN_PLACE").is_some());
    assert!(page.visual.debug_bounds("ROLE_EDITING_TAG").is_some());
    assert!(page.visual.debug_bounds("ROLE_PROMPT_EDITOR").is_some());
    assert!(page.visual.debug_bounds("ROLE_EDIT_DIRTY").is_none());
    let clean_setup = page.visual.debug_bounds("ROLE_PAGE_SETUP").unwrap();

    let name = page.read(|root| {
        root.role_surfaces
            .edit
            .as_ref()
            .unwrap()
            .display_name
            .clone()
    });
    page.host
        .update(&mut page.visual, |_, _, cx| {
            name.update(cx, |input, cx| input.set_text("Renamed", cx))
        })
        .unwrap();
    page.visual.run_until_parked();
    assert!(page.visual.debug_bounds("ROLE_EDIT_DIRTY").is_some());
    assert_eq!(
        page.visual.debug_bounds("ROLE_PAGE_SETUP").unwrap().top(),
        clean_setup.top(),
        "\"Unsaved changes\" appears in reserved space; the setup below never moves"
    );

    page.click("ROLE_EDIT_CANCEL");
    assert!(!page.in_place_edit());
    let stored = runner_backend::ops::role::role_get(&page.core, &role.id).unwrap();
    assert_eq!(stored.display_name, "Role page-coder");

    // Escape from a field cancels too, as it closes the drawer.
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_role_edit(role.clone(), window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    assert!(page.in_place_edit());
    page.visual.simulate_keystrokes("escape");
    page.visual.run_until_parked();
    assert!(!page.in_place_edit());
}

#[test]
fn an_untouched_form_is_clean_even_when_an_arg_holds_a_space() {
    let mut page = role_page_harness("role-edit-args");
    let role = create_test_role_with_args(
        &page.core,
        "page-coder",
        Some("Ship it.".into()),
        vec!["--label".into(), "two words".into()],
    );
    assert!(role.args.iter().any(|arg| arg == "two words"));
    page.open_role("page-coder");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_role_edit(role.clone(), window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    let dirty = page
        .host
        .update(&mut page.visual, |root, _, cx| {
            super::logic::role_edit_is_dirty(root.role_surfaces.edit.as_ref().unwrap(), cx)
        })
        .unwrap();
    assert!(!dirty, "opening the editor changes nothing");
    assert!(page.visual.debug_bounds("ROLE_EDIT_IN_PLACE").is_some());
    assert!(page.visual.debug_bounds("ROLE_EDIT_DIRTY").is_none());

    let args = page.read(|root| root.role_surfaces.edit.as_ref().unwrap().args.clone());
    page.host
        .update(&mut page.visual, |_, _, cx| {
            args.update(cx, |input, cx| {
                input.set_text("--label two  words --verbose", cx)
            })
        })
        .unwrap();
    page.visual.run_until_parked();
    assert!(page.visual.debug_bounds("ROLE_EDIT_DIRTY").is_some());
}

#[test]
fn saving_a_rename_keeps_an_arg_that_holds_a_space() {
    let mut page = role_page_harness("role-save-args");
    let role = create_test_role_with_args(
        &page.core,
        "page-coder",
        None,
        vec!["--label".into(), "two words".into()],
    );
    page.open_role("page-coder");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_role_edit(role.clone(), window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    let name = page.read(|root| {
        root.role_surfaces
            .edit
            .as_ref()
            .unwrap()
            .display_name
            .clone()
    });
    page.host
        .update(&mut page.visual, |_, _, cx| {
            name.update(cx, |input, cx| input.set_text("Renamed coder", cx))
        })
        .unwrap();
    page.visual.run_until_parked();
    page.click("ROLE_EDIT_SAVE");
    assert!(!page.in_place_edit());
    let stored = runner_backend::ops::role::role_get(&page.core, &role.id).unwrap();
    assert_eq!(stored.display_name, "Renamed coder");
    assert_eq!(stored.args, role.args, "a rename leaves the command alone");
    assert!(stored.args.iter().any(|arg| arg == "two words"));
}

#[test]
fn a_legacy_shell_role_saves_only_after_an_agent_is_picked() {
    let mut page = role_page_harness("role-legacy-shell");
    let timestamp = "2026-09-21T00:00:00Z".parse().unwrap();
    runner_backend::repo::role::insert(
        &page.core.db.get().unwrap(),
        &runner_backend::repo::role::RoleRow {
            id: "legacy-shell".into(),
            handle: "legacy-shell".into(),
            display_name: "Legacy shell".into(),
            runtime: "shell".into(),
            command: "/bin/zsh".into(),
            args_json: Some(Vec::new()),
            working_dir: None,
            system_prompt: None,
            env_json: Some(Default::default()),
            model: None,
            effort: None,
            codex_speed: None,
            created_at: timestamp,
            updated_at: timestamp,
        },
    )
    .unwrap();
    let role = runner_backend::ops::role::role_get(&page.core, "legacy-shell").unwrap();
    let options = super::logic::role_edit_runtime_options(
        &[runtime_with_defaults(None, None)],
        &role,
        &role.runtime,
    );
    assert_eq!(
        options
            .iter()
            .map(|option| option.value.as_str())
            .collect::<Vec<_>>(),
        ["codex"],
        "the agent select offers the agent runtimes to switch to"
    );

    page.open_role("legacy-shell");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_role_edit(role.clone(), window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    let name = page.read(|root| {
        root.role_surfaces
            .edit
            .as_ref()
            .unwrap()
            .display_name
            .clone()
    });
    page.host
        .update(&mut page.visual, |_, _, cx| {
            name.update(cx, |input, cx| input.set_text("Renamed shell", cx))
        })
        .unwrap();
    page.visual.run_until_parked();
    page.click("ROLE_EDIT_SAVE");

    assert!(page.in_place_edit(), "a refused save stays in edit mode");
    assert_eq!(
        page.read(|root| root.role_surfaces.edit.as_ref().unwrap().error.clone()),
        Some(
            "unknown runtime 'shell' — valid runtimes: codex, claude-code, antigravity, pi, copilot, trae"
                .into()
        )
    );
    let stored = runner_backend::ops::role::role_get(&page.core, "legacy-shell").unwrap();
    assert_eq!(stored.display_name, "Legacy shell");
    assert_eq!(stored.runtime, "shell");
}

#[test]
fn a_search_with_no_matches_keeps_the_count() {
    use crate::surfaces::AppRoute;

    let mut page = role_page_harness("role-list-no-matches");
    let total = runner_backend::ops::role::role_list(&page.core)
        .unwrap()
        .len();
    page.visual.run_until_parked();
    // Seed the list as a finished zero-match search before the Roles route
    // first draws, since test frames keep a selector's bounds once drawn.
    page.host
        .update(&mut page.visual, |root, _, cx| {
            let list = &mut root.role_surfaces.list;
            list.query = "zzz-no-role".into();
            list.debounced_query = "zzz-no-role".into();
            list.items.clear();
            list.filtered_count = 0;
            list.total_count = total;
            list.loaded = true;
            list.loading = false;
            root.route = AppRoute::Roles;
            cx.notify();
        })
        .unwrap();
    page.visual.run_until_parked();
    assert!(
        page.visual.debug_bounds("PAGINATED_LIST_COUNT").is_some(),
        "the pager row and its count stay under the no-match card"
    );
    let (filtered, total_count, searching) = page.read(|root| {
        let list = &root.role_surfaces.list;
        (list.filtered_count, list.total_count, list.searching())
    });
    assert_eq!(
        runner_app::ui::count_label(filtered, total_count, "roles", searching),
        format!("0 of {total} roles")
    );
}

#[test]
fn the_prompt_editor_fills_the_column_while_editing() {
    use gpui::{px, size};

    let mut page = role_page_harness("role-edit-height");
    let role = create_test_role(&page.core, "page-coder", Some("Ship it.".into()));
    page.open_role("page-coder");
    page.visual.simulate_resize(size(px(1440.), px(900.)));
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_role_edit(role.clone(), window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    let profile = page.visual.debug_bounds("ROLE_PAGE_PROFILE").unwrap();
    let prompt = page.visual.debug_bounds("ROLE_PAGE_PROMPT").unwrap();
    let card = page.visual.debug_bounds("ROLE_PROMPT_CARD").unwrap();
    let editor = page.visual.debug_bounds("ROLE_PROMPT_EDITOR").unwrap();
    assert!(
        (prompt.bottom() - profile.bottom()).abs() <= px(1.),
        "the prompt column should run the profile column's height: {prompt:?} vs {profile:?}"
    );
    assert!(
        card.size.height > profile.size.height * 0.8,
        "the editor card should fill the column, got {card:?} beside {profile:?}"
    );
    assert!(editor.bottom() <= card.bottom() && editor.size.height > card.size.height * 0.8);
}

#[test]
fn save_in_place_goes_through_the_shared_submit() {
    let mut page = role_page_harness("role-edit-save");
    let role = create_test_role(&page.core, "page-coder", Some("Ship it.".into()));
    page.open_role("page-coder");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_role_edit(role.clone(), window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    let (name, prompt) = page.read(|root| {
        let form = root.role_surfaces.edit.as_ref().unwrap();
        (form.display_name.clone(), form.system_prompt.clone())
    });
    page.host
        .update(&mut page.visual, |_, _, cx| {
            name.update(cx, |input, cx| input.set_text("Renamed coder", cx));
            prompt.update(cx, |input, cx| {
                input.set_text("# Coder\n\nShip it, tested.", cx)
            });
        })
        .unwrap();
    page.visual.run_until_parked();

    page.click("ROLE_EDIT_SAVE");
    assert!(!page.in_place_edit(), "a clean save leaves edit mode");
    let stored = runner_backend::ops::role::role_get(&page.core, &role.id).unwrap();
    assert_eq!(stored.display_name, "Renamed coder");
    assert_eq!(
        stored.system_prompt.as_deref(),
        Some("# Coder\n\nShip it, tested.")
    );
    assert_eq!(
        stored.model.as_deref(),
        Some("opus"),
        "untouched fields survive"
    );
    let shown = page.read(|root| root.role_surfaces.detail.role.clone().unwrap());
    assert_eq!(shown.display_name, "Renamed coder");
}

#[test]
fn codex_speed_edit_reloads_and_runtime_switch_clears_it() {
    use runner_backend::model::CodexSpeed;
    let mut page = role_page_harness("role-codex-speed");
    let role = runner_backend::ops::role::role_create(
        &page.core,
        serde_json::from_value(serde_json::json!({
            "handle": "speed-coder",
            "display_name": "Speed coder",
            "runtime": "codex",
            "command": "codex",
            "codex_speed": "fast"
        }))
        .unwrap(),
    )
    .unwrap();
    page.open_role("speed-coder");
    assert!(page.visual.debug_bounds("ROLE_SPEED_DETAIL").is_some());
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_role_edit(role.clone(), window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    assert!(page.visual.debug_bounds("ROLE_SPEED_EDIT").is_some());
    assert_eq!(
        page.read(|root| root.role_surfaces.edit.as_ref().unwrap().speed.clone()),
        "fast"
    );
    page.host
        .update(&mut page.visual, |root, _, cx| {
            let form = root.role_surfaces.edit.as_mut().unwrap();
            form.speed = "standard".into();
            form.speed_select
                .update(cx, |select, cx| select.set_value("standard", cx));
            cx.notify();
        })
        .unwrap();
    page.click("ROLE_EDIT_SAVE");
    let stored = runner_backend::ops::role::role_get(&page.core, &role.id).unwrap();
    assert_eq!(stored.codex_speed, Some(CodexSpeed::Standard));
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_role_edit(stored, window, cx);
            root.select_role_edit_runtime("claude-code".into(), cx);
        })
        .unwrap();
    page.visual.run_until_parked();
    assert_eq!(
        page.read(|root| root.role_surfaces.edit.as_ref().unwrap().speed.clone()),
        "inherit"
    );
    page.click("ROLE_EDIT_SAVE");
    let stored = runner_backend::ops::role::role_get(&page.core, &role.id).unwrap();
    assert_eq!(stored.runtime, "claude-code");
    assert_eq!(stored.codex_speed, None);
}

#[test]
fn create_role_fast_speed_shows_credit_note_and_resets_on_runtime_switch() {
    let mut page = role_page_harness("role-create-speed");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_create_role(window, cx);
            root.select_create_role_runtime("codex".into(), cx);
        })
        .unwrap();
    assert_eq!(
        page.read(|root| root.role_surfaces.create.as_ref().unwrap().speed.clone()),
        "inherit"
    );
    page.host
        .update(&mut page.visual, |root, _, cx| {
            let form = root.role_surfaces.create.as_mut().unwrap();
            form.speed = "fast".into();
            form.speed_select
                .update(cx, |select, cx| select.set_value("fast", cx));
            cx.notify();
        })
        .unwrap();
    page.visual.run_until_parked();
    assert!(
        page.visual.debug_bounds("NEW_ROLE_SPEED_NOTE").is_some(),
        "Fast credit note must be visible without opening a tooltip"
    );
    page.host
        .update(&mut page.visual, |root, _, cx| {
            root.select_create_role_runtime("claude-code".into(), cx)
        })
        .unwrap();
    let (runtime, speed) = page.read(|root| {
        let form = root.role_surfaces.create.as_ref().unwrap();
        (form.runtime.clone(), form.speed.clone())
    });
    assert_eq!(runtime, "claude-code");
    assert_eq!(speed, "inherit");
}

#[test]
fn leaving_the_role_page_discards_an_in_place_draft() {
    let mut page = role_page_harness("role-edit-leave");
    let role = create_test_role(&page.core, "page-coder", None);
    page.open_role("page-coder");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_role_edit(role.clone(), window, cx)
        })
        .unwrap();
    assert!(page.in_place_edit());
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_roles(window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    assert!(!page.in_place_edit());
}

#[test]
fn edit_details_opens_the_role_page_in_edit_mode() {
    use crate::surfaces::AppRoute;

    let mut page = role_page_harness("role-menu-edit");
    let role = create_test_role(&page.core, "page-planner", None);
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_roles(window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.handle_role_menu_action(RoleMenuAction::Edit(Box::new(role.clone())), window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    assert_eq!(
        page.read(|root| root.route.clone()),
        AppRoute::RoleDetail("page-planner".into())
    );
    assert!(page.in_place_edit());
    assert!(page.visual.debug_bounds("ROLE_EDIT_IN_PLACE").is_some());
}

/// Records the width layout offers a text leaf, in the order offered.
struct MeasureProbe(std::rc::Rc<std::cell::RefCell<Vec<gpui::AvailableSpace>>>);

impl gpui::IntoElement for MeasureProbe {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for MeasureProbe {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut gpui::Window,
        _: &mut gpui::App,
    ) -> (gpui::LayoutId, ()) {
        let offered = self.0.clone();
        let layout =
            window.request_measured_layout(Default::default(), move |_, available, _, _| {
                offered.borrow_mut().push(available.width);
                gpui::size(gpui::px(10.), gpui::px(10.))
            });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        _: &mut (),
        _: &mut gpui::Window,
        _: &mut gpui::App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        _: &mut (),
        _: &mut (),
        _: &mut gpui::Window,
        _: &mut gpui::App,
    ) {
    }
}

#[test]
fn profile_text_is_first_shaped_at_its_column_width() {
    use super::detail::column_text;
    use gpui::prelude::*;
    use gpui::{div, px, rems, size, AvailableSpace, TestAppContext, VisualTestContext};
    use std::cell::RefCell;
    use std::rc::Rc;

    // GPUI keeps a non-wrapping line as first shaped, so a first offer of
    // 0 px left every profile value as "…" inside the min_w(0) columns.
    struct Column(Rc<RefCell<Vec<AvailableSpace>>>);
    impl gpui::Render for Column {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div().size_full().flex().flex_col().child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .child(
                        div()
                            .w(rems(ROLE_COLUMN_WIDTH / 16.))
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .child(
                                div().min_w(px(0.)).flex().flex_col().child(
                                    column_text("~/repos/yicheng47/runner", ROLE_COLUMN_WIDTH)
                                        .child(MeasureProbe(self.0.clone())),
                                ),
                            ),
                    )
                    .child(div().flex_1().flex_basis(px(360.)).min_w(px(0.))),
            )
        }
    }
    let offered = Rc::new(RefCell::new(Vec::new()));
    let mut cx = TestAppContext::single();
    let probe = offered.clone();
    let window = cx.add_window(move |_, _| Column(probe));
    let visual = VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    visual.run_until_parked();
    let first = offered.borrow().first().copied();
    assert_eq!(
        first,
        Some(AvailableSpace::Definite(px(ROLE_COLUMN_WIDTH))),
        "all offers: {:?}",
        offered.borrow()
    );
}

// A flex column's first sizing pass offers its `min_w(0)` rows no width; a
// label or paragraph that wraps there is measured a character per line, and
// the setup section kept that height as a gap below its last row.
#[test]
fn the_setup_section_ends_at_its_last_row_in_view_and_edit() {
    use gpui::{px, size};

    let mut page = role_page_harness("role-setup-height");
    let role = create_test_role(
        &page.core,
        "page-reviewer",
        Some("You are a reviewer in a two-person peer coding loop.\n\n".repeat(40)),
    );
    page.open_role("page-reviewer");
    for edit in [false, true] {
        if edit {
            let role = role.clone();
            page.host
                .update(&mut page.visual, |root, window, cx| {
                    root.open_role_edit(role, window, cx)
                })
                .unwrap();
            page.visual.run_until_parked();
        }
        for width in [640., 1100., 1440.] {
            page.visual.simulate_resize(size(px(width), px(900.)));
            page.visual.run_until_parked();
            let setup = page.visual.debug_bounds("ROLE_PAGE_SETUP").unwrap();
            let last = page.visual.debug_bounds("ROLE_SETUP_LAST").unwrap();
            assert!(
                (setup.bottom() - last.bottom()).abs() <= px(1.),
                "edit {edit}, {width}: the setup runs past its last row, {setup:?} vs {last:?}"
            );
            if edit {
                assert!(
                    last.size.width <= px(ROLE_COLUMN_WIDTH + 1.),
                    "{width}: the edit form keeps the column's width: {last:?}"
                );
            }
        }
    }
}

#[test]
fn new_role_entry_points_use_a_page_and_cancel_returns_to_the_list() {
    use crate::surfaces::AppRoute;
    for entry in ["NEW_ROLE", "EMPTY_NEW_ROLE"] {
        let mut page = role_page_harness("new-role-entry");
        page.host
            .update(&mut page.visual, |root, window, cx| {
                root.open_roles(window, cx)
            })
            .unwrap();
        page.visual.run_until_parked();
        if entry == "EMPTY_NEW_ROLE" {
            page.host
                .update(&mut page.visual, |root, _, cx| {
                    let list = &mut root.role_surfaces.list;
                    list.items.clear();
                    list.total_count = 0;
                    list.filtered_count = 0;
                    cx.notify();
                })
                .unwrap();
            page.visual.run_until_parked();
        }
        page.click(entry);
        assert_eq!(page.read(|root| root.route.clone()), AppRoute::NewRole);
        for selector in [
            "ROLE_NEW_TAG",
            "ROLE_EDIT_IN_PLACE",
            "ROLE_NAME_FIELD",
            "ROLE_CREATE_HANDLE",
            "ROLE_CREATE_AVATAR",
            "ROLE_PROMPT_CARD",
        ] {
            assert!(
                page.visual.debug_bounds(selector).is_some(),
                "missing {selector}"
            );
        }
        assert!(page.visual.debug_bounds("ROLE_CREWS_SECTION").is_none());
        assert!(page.visual.debug_bounds("ROLE_ACTIVITY_LINES").is_none());
        assert!(page.visual.debug_bounds("ROLE_EDIT_DIRTY").is_none());
        assert!(page.visual.debug_bounds("EMPTY_PROFILE_TILE").is_none());
        page.host
            .update(&mut page.visual, |root, window, cx| {
                assert!(root.render_entity_overlays(window, cx).is_empty())
            })
            .unwrap();
        page.click("ROLE_EDIT_CANCEL");
        assert_eq!(page.read(|root| root.route.clone()), AppRoute::Roles);
        assert!(page.read(|root| root.role_surfaces.create.is_none()));
    }
}

#[test]
fn new_role_avatar_preview_is_stable_until_the_handle_is_typed() {
    let mut page = role_page_harness("new-role-avatar-preview");
    let initial = page
        .host
        .update(&mut page.visual, |root, window, cx| {
            root.open_create_role(window, cx);
            root.role_surfaces.create.as_ref().unwrap().avatar_seed(cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    assert!(!initial.is_empty());
    assert!(page.visual.debug_bounds("ROLE_CREATE_AVATAR").is_some());
    assert!(page.visual.debug_bounds("EMPTY_PROFILE_TILE").is_none());
    page.host
        .update(&mut page.visual, |root, _, cx| {
            root.role_surfaces
                .create
                .as_ref()
                .unwrap()
                .display_name
                .update(cx, |field, cx| field.set_text("Preview coder", cx));
        })
        .unwrap();
    page.visual.run_until_parked();
    page.host
        .update(&mut page.visual, |root, _, cx| {
            let form = root.role_surfaces.create.as_ref().unwrap();
            assert_eq!(form.avatar_seed(cx), initial);
            form.handle
                .update(cx, |field, cx| field.set_text("preview-coder", cx));
        })
        .unwrap();
    page.visual.run_until_parked();
    page.host
        .update(&mut page.visual, |root, _, cx| {
            let form = root.role_surfaces.create.as_ref().unwrap();
            assert_eq!(form.avatar_seed(cx), "preview-coder");
            form.handle.update(cx, |field, cx| field.set_text("", cx));
        })
        .unwrap();
    page.visual.run_until_parked();
    page.host
        .update(&mut page.visual, |root, window, cx| {
            assert_eq!(
                root.role_surfaces.create.as_ref().unwrap().avatar_seed(cx),
                initial
            );
            root.close_create_role(window, cx);
            root.open_create_role(window, cx);
            assert_ne!(
                root.role_surfaces.create.as_ref().unwrap().avatar_seed(cx),
                initial
            );
        })
        .unwrap();
}

#[test]
fn role_edit_saves_effort_with_default_model_and_resets_it_on_runtime_change() {
    let mut page = role_page_harness("edit-default-model-effort");
    let role = create_test_role(&page.core, "default-effort-role", None);
    page.open_role("default-effort-role");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_role_edit(role, window, cx);
            let form = root.role_surfaces.edit.as_mut().unwrap();
            form.model.update(cx, |field, cx| field.set_text("", cx));
            form.effort = "high".into();
        })
        .unwrap();
    page.visual.run_until_parked();
    page.host
        .update(&mut page.visual, |root, _, cx| {
            let form = root.role_surfaces.edit.as_ref().unwrap();
            assert!(form.model.read(cx).text().is_empty());
            assert!(!form.effort_select.read(cx).is_disabled());
            assert_eq!(form.effort, "high");
        })
        .unwrap();
    page.click("ROLE_EDIT_SAVE");
    let saved =
        runner_backend::ops::role::role_get_by_handle(&page.core, "default-effort-role").unwrap();
    assert_eq!(saved.model, None);
    assert_eq!(saved.effort.as_deref(), Some("high"));
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_role_edit(saved, window, cx);
            root.role_surfaces.edit.as_mut().unwrap().runtimes =
                runner_backend::ops::runtime::runtime_catalog(&page.core).unwrap();
            root.select_role_edit_runtime("codex".into(), cx);
            assert!(root.role_surfaces.edit.as_ref().unwrap().effort.is_empty());
        })
        .unwrap();
}

#[test]
fn new_role_saves_effort_speed_and_prompt_then_opens_view_mode() {
    use crate::surfaces::AppRoute;
    for model_override in [Some("gpt-5.6-sol"), None] {
        let mut page = role_page_harness("new-role-submit");
        page.host
            .update(&mut page.visual, |root, window, cx| {
                root.open_create_role(window, cx)
            })
            .unwrap();
        page.visual.run_until_parked();
        page.host
            .update(&mut page.visual, |root, _, cx| {
                let form = root.role_surfaces.create.as_mut().unwrap();
                form.runtimes = runner_backend::ops::runtime::runtime_catalog(&page.core).unwrap();
                root.select_create_role_runtime("codex".into(), cx);
                let form = root.role_surfaces.create.as_mut().unwrap();
                form.handle
                    .update(cx, |input, cx| input.set_text("new-coder", cx));
                form.display_name
                    .update(cx, |input, cx| input.set_text("New coder", cx));
                form.system_prompt
                    .update(cx, |input, cx| input.set_text("# Build\n\nShip it.", cx));
                form.model.update(cx, |input, cx| {
                    input.set_text(model_override.unwrap_or_default(), cx)
                });
                form.effort = "high".into();
                form.speed = "fast".into();
                cx.notify();
            })
            .unwrap();
        page.visual.run_until_parked();
        assert!(page.visual.debug_bounds("ROLE_CREATE_AVATAR").is_some());
        let model = page.visual.debug_bounds("ROLE_EDIT_ROW Model").unwrap();
        let effort = page.visual.debug_bounds("ROLE_EDIT_ROW Effort").unwrap();
        assert_eq!(model.top(), effort.top());
        assert_eq!(model.size.width, effort.size.width);
        assert!(page.visual.debug_bounds("ROLE_SPEED_EDIT").is_some());
        page.host
            .update(&mut page.visual, |root, _, cx| {
                let form = root.role_surfaces.create.as_ref().unwrap();
                assert!(!form.effort_select.read(cx).is_disabled());
                assert_eq!(form.effort, "high");
            })
            .unwrap();
        assert!(page.read(|root| super::logic::create_role_can_submit(
            root.role_surfaces.create.as_ref().unwrap()
        )));
        page.click("ROLE_EDIT_SAVE");
        let role = runner_backend::ops::role::role_get_by_handle(&page.core, "new-coder").unwrap();
        assert_eq!(role.effort.as_deref(), Some("high"));
        assert_eq!(role.model.as_deref(), model_override);
        assert_eq!(
            role.codex_speed,
            Some(runner_backend::model::CodexSpeed::Fast)
        );
        assert_eq!(role.system_prompt.as_deref(), Some("# Build\n\nShip it."));
        assert_eq!(
            page.read(|root| root.route.clone()),
            AppRoute::RoleDetail("new-coder".into())
        );
        assert!(page
            .read(|root| root.role_surfaces.create.is_none() && root.role_surfaces.edit.is_none()));
    }
}

#[test]
fn new_role_validation_and_runtime_changes_keep_creation_safe() {
    let mut page = role_page_harness("new-role-validation");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_create_role(window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    page.host
        .update(&mut page.visual, |root, _, cx| {
            let form = root.role_surfaces.create.as_mut().unwrap();
            form.runtimes = runner_backend::ops::runtime::runtime_catalog(&page.core).unwrap();
            form.handle
                .update(cx, |input, cx| input.set_text("bad!", cx));
            form.display_name
                .update(cx, |input, cx| input.set_text("Bad", cx));
            cx.notify();
        })
        .unwrap();
    page.visual.run_until_parked();
    assert!(page.read(|root| root
        .role_surfaces
        .create
        .as_ref()
        .unwrap()
        .handle_error
        .is_some()));
    assert!(!page.read(|root| super::logic::create_role_can_submit(
        root.role_surfaces.create.as_ref().unwrap()
    )));
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.submit_create_role(window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    assert!(runner_backend::ops::role::role_get_by_handle(&page.core, "bad!").is_err());
    page.host
        .update(&mut page.visual, |root, _, cx| {
            root.select_create_role_runtime("codex".into(), cx);
            root.role_surfaces.create.as_mut().unwrap().effort = "xhigh".into();
            root.select_create_role_runtime("pi".into(), cx);
            assert!(root
                .role_surfaces
                .create
                .as_ref()
                .unwrap()
                .effort
                .is_empty());
        })
        .unwrap();
}

#[test]
fn creating_routes_support_history_and_discard_drafts_when_leaving() {
    use crate::surfaces::AppRoute;
    let mut page = role_page_harness("creating-history");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            assert_eq!(root.route, AppRoute::Roles);
            root.open_create_role(window, cx);
            root.role_surfaces
                .create
                .as_ref()
                .unwrap()
                .display_name
                .update(cx, |field, cx| field.set_text("Draft", cx));
            root.navigate_runtime_page(-1, window, cx);
            assert_eq!(root.route, AppRoute::Roles);
            assert!(root.role_surfaces.create.is_none());
            root.navigate_runtime_page(1, window, cx);
            assert_eq!(root.route, AppRoute::NewRole);
            assert_eq!(
                root.role_surfaces
                    .create
                    .as_ref()
                    .unwrap()
                    .display_name
                    .read(cx)
                    .text(),
                ""
            );
            root.open_crews(window, cx);
            root.open_create_crew(window, cx);
            root.navigate_runtime_page(-1, window, cx);
            assert_eq!(root.route, AppRoute::Crews);
            root.navigate_runtime_page(1, window, cx);
            assert_eq!(root.route, AppRoute::NewCrew);
            root.navigate_runtime_page(-1, window, cx);
            assert_eq!(root.route, AppRoute::Crews);
        })
        .unwrap();
    page.visual.run_until_parked();
}

#[test]
fn new_role_layout_fits_the_minimum_window_in_both_themes() {
    let mut page = role_page_harness("new-role-small");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_create_role(window, cx)
        })
        .unwrap();
    page.visual
        .simulate_resize(gpui::size(gpui::px(640.), gpui::px(480.)));
    for variant in [
        theme::ThemeVariant::Carbon,
        theme::ThemeVariant::RunnerLight,
    ] {
        theme::set_active_variant(variant);
        page.host
            .update(&mut page.visual, |_, _, cx| cx.notify())
            .unwrap();
        page.visual.run_until_parked();
        let profile = page.visual.debug_bounds("ROLE_PAGE_PROFILE").unwrap();
        let prompt = page.visual.debug_bounds("ROLE_PAGE_PROMPT").unwrap();
        assert!(profile.right() <= gpui::px(640.));
        assert!(prompt.right() <= gpui::px(640.));
        assert!(prompt.top() >= profile.bottom());
    }
}

#[test]
fn chat_panel_opens_role_clamps_toggles_and_resets_on_another_chat() {
    use crate::surfaces::AppRoute;
    let mut page = role_page_harness("panel-prompt-actions");
    let role = create_test_role(
        &page.core,
        "panel-coder",
        Some("# Instructions\n\n".to_owned() + &"- Build the feature.\n".repeat(60)),
    );
    let conn = page.core.db.get().unwrap();
    for id in [
        "first-panel-chat",
        "second-panel-chat",
        "runtime-panel-chat",
    ] {
        conn.execute(
            "INSERT INTO sessions (id, role_id, status) VALUES (?1, ?2, 'stopped')",
            [
                Some(id),
                (id != "runtime-panel-chat").then_some(role.id.as_str()),
            ],
        )
        .unwrap();
        runner_backend::repo::node::create_tab(
            &conn,
            None,
            "",
            0,
            &runner_app::pane_layout::PaneLayout::single(Some(id), &[id.into()])
                .serialize()
                .unwrap(),
        )
        .unwrap();
    }
    for id in ["first-archived-panel-chat", "second-archived-panel-chat"] {
        conn.execute(
            "INSERT INTO sessions (id, role_id, status, archived_at) VALUES (?1, ?2, 'stopped', '2026-09-30T00:00:00Z')",
            [id, role.id.as_str()],
        )
        .unwrap();
    }
    let nodes = runner_backend::repo::node::list(&conn).unwrap();
    let roles = runner_backend::ops::role::role_list(&page.core).unwrap();
    page.host
        .update(&mut page.visual, |root, _, cx| {
            root.app_store.update(cx, |store, cx| {
                store.replace_roles(roles, cx);
                store.replace_nodes(nodes, cx);
                store.refresh_sessions(cx);
            });
            root.apply_tab_rows(cx);
            root.tabs.activate_session("first-panel-chat");
            root.route = AppRoute::Chat;
            root.sync_active_chat_detail(cx);
            cx.notify();
        })
        .unwrap();
    page.visual
        .simulate_resize(gpui::size(gpui::px(1440.), gpui::px(3000.)));
    page.visual.run_until_parked();
    assert!(page.visual.debug_bounds("CHAT_PANEL_OPEN_ROLE").is_some());
    assert!(page
        .visual
        .debug_bounds("CHAT_PANEL_PROMPT_TOGGLE")
        .is_some());
    let collapsed = page
        .visual
        .debug_bounds("CHAT_PANEL_PROMPT_TEXT")
        .unwrap()
        .size
        .height;
    page.click("CHAT_PANEL_PROMPT_TOGGLE");
    assert!(page.read(|root| root.chat_panel_prompt_expanded));
    assert!(
        page.visual
            .debug_bounds("CHAT_PANEL_PROMPT_TEXT")
            .unwrap()
            .size
            .height
            > collapsed
    );
    page.click("CHAT_PANEL_PROMPT_TOGGLE");
    assert!(!page.read(|root| root.chat_panel_prompt_expanded));
    page.click("CHAT_PANEL_PROMPT_TOGGLE");
    page.host
        .update(&mut page.visual, |root, _, cx| {
            root.tabs.activate_session("second-panel-chat");
            root.sync_active_chat_detail(cx);
            cx.notify();
        })
        .unwrap();
    page.visual.run_until_parked();
    assert!(!page.read(|root| root.chat_panel_prompt_expanded));
    page.click("CHAT_PANEL_PROMPT_TOGGLE");
    for id in ["first-archived-panel-chat", "second-archived-panel-chat"] {
        let archived = runner_backend::ops::session::session_get(&page.core, id)
            .unwrap()
            .unwrap();
        page.host
            .update(&mut page.visual, |root, window, cx| {
                root.enter_settings_route(Some("archived"), window, cx);
                root.open_archived_chat(archived, window, cx);
            })
            .unwrap();
        page.visual.run_until_parked();
        assert_eq!(
            page.read(|root| root.chat_panel_prompt_session.clone()),
            Some(id.into())
        );
        assert!(!page.read(|root| root.chat_panel_prompt_expanded));
        page.click("CHAT_PANEL_PROMPT_TOGGLE");
        page.host
            .update(&mut page.visual, |root, _, cx| {
                root.sync_active_chat_detail(cx);
                cx.notify();
            })
            .unwrap();
        page.visual.run_until_parked();
        assert!(page.read(|root| root.chat_panel_prompt_expanded));
    }
    page.host
        .update(&mut page.visual, |root, _, cx| {
            root.set_route(AppRoute::Chat, cx);
            root.sync_active_chat_detail(cx);
            cx.notify();
        })
        .unwrap();
    page.visual.run_until_parked();
    assert!(!page.read(|root| root.chat_panel_prompt_expanded));
    page.click("CHAT_PANEL_OPEN_ROLE");
    assert_eq!(
        page.read(|root| root.route.clone()),
        AppRoute::RoleDetail("panel-coder".into())
    );
    page.host
        .update(&mut page.visual, |root, _, cx| {
            root.tabs.activate_session("runtime-panel-chat");
            root.set_route(AppRoute::Chat, cx);
            root.sync_active_chat_detail(cx);
            cx.notify();
        })
        .unwrap();
    page.visual.run_until_parked();
    assert!(!page.read(|root| root.chat_panel_prompt_expanded));
}

#[test]
fn new_role_enter_is_ime_safe_and_tabs_start_with_name_then_handle() {
    use gpui::EntityInputHandler;
    let mut page = role_page_harness("new-role-keyboard");
    page.visual.run_until_parked();
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_create_role(window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    page.host
        .update(&mut page.visual, |root, window, cx| {
            assert!(
                root.role_surfaces
                    .create
                    .as_ref()
                    .unwrap()
                    .display_name
                    .read(cx)
                    .focus_handle()
                    .is_focused(window),
                "creation should focus the name"
            );
        })
        .unwrap();
    page.visual.simulate_keystrokes("tab");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            let form = root.role_surfaces.create.as_ref().unwrap();
            assert!(form.handle.read(cx).focus_handle().is_focused(window));
            form.handle.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "coder", Some(5..5), window, cx)
            });
        })
        .unwrap();
    page.visual.run_until_parked();
    page.visual.simulate_keystrokes("enter escape");
    page.visual.run_until_parked();
    assert!(page.read(|root| root.role_surfaces.create.is_some()));
    page.host
        .update(&mut page.visual, |root, window, cx| {
            let form = root.role_surfaces.create.as_mut().unwrap();
            form.handle.update(cx, |input, cx| {
                input.replace_text_in_range(None, "keyboard-coder", window, cx)
            });
            form.display_name
                .update(cx, |input, cx| input.set_text("Keyboard coder", cx));
            form.runtimes = runner_backend::ops::runtime::runtime_catalog(&page.core).unwrap();
            root.select_create_role_runtime("codex".into(), cx);
        })
        .unwrap();
    page.visual.run_until_parked();
    page.visual.simulate_keystrokes("enter");
    page.visual.run_until_parked();
    assert!(runner_backend::ops::role::role_get_by_handle(&page.core, "keyboard-coder").is_ok());
}

#[test]
fn new_role_keyboard_reaches_the_breadcrumb() {
    use crate::surfaces::AppRoute;
    let mut page = role_page_harness("new-role-breadcrumb-keyboard");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_create_role(window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    page.visual.simulate_keystrokes("shift-tab enter");
    page.visual.run_until_parked();
    assert_eq!(page.read(|root| root.route.clone()), AppRoute::Roles);
}

#[test]
fn role_edit_tabs_across_the_runtime_select_in_both_directions() {
    let mut page = role_page_harness("role-runtime-tab-order");
    let role = create_test_role(&page.core, "keyboard-role", None);
    page.open_role("keyboard-role");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_role_edit(role, window, cx);
            root.role_surfaces
                .edit
                .as_ref()
                .unwrap()
                .cancel_focus
                .focus(window, cx);
        })
        .unwrap();
    page.visual.run_until_parked();
    page.visual.simulate_keystrokes("tab");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            let form = root.role_surfaces.edit.as_ref().unwrap();
            assert!(form
                .runtime_select
                .read(cx)
                .focus_handle()
                .is_focused(window));
        })
        .unwrap();
    page.visual.simulate_keystrokes("shift-tab");
    page.host
        .update(&mut page.visual, |root, window, _| {
            assert!(root
                .role_surfaces
                .edit
                .as_ref()
                .unwrap()
                .cancel_focus
                .is_focused(window));
        })
        .unwrap();
    page.visual.simulate_keystrokes("tab tab");
    page.host
        .update(&mut page.visual, |root, window, cx| {
            let form = root.role_surfaces.edit.as_ref().unwrap();
            assert!(form.model.read(cx).focus_handle().is_focused(window));
        })
        .unwrap();
    page.visual.simulate_keystrokes("shift-tab shift-tab");
    page.host
        .update(&mut page.visual, |root, window, _| {
            assert!(root
                .role_surfaces
                .edit
                .as_ref()
                .unwrap()
                .cancel_focus
                .is_focused(window));
        })
        .unwrap();
}

#[test]
fn new_role_backend_error_keeps_the_draft_on_the_creating_page() {
    let mut page = role_page_harness("new-role-error");
    create_test_role(&page.core, "duplicate-page-role", None);
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_create_role(window, cx)
        })
        .unwrap();
    page.visual.run_until_parked();
    page.host
        .update(&mut page.visual, |root, _, cx| {
            let form = root.role_surfaces.create.as_mut().unwrap();
            form.runtimes = runner_backend::ops::runtime::runtime_catalog(&page.core).unwrap();
            form.handle
                .update(cx, |input, cx| input.set_text("duplicate-page-role", cx));
            form.display_name
                .update(cx, |input, cx| input.set_text("Duplicate", cx));
            root.select_create_role_runtime("codex".into(), cx);
        })
        .unwrap();
    page.visual.run_until_parked();
    page.click("ROLE_EDIT_SAVE");
    assert_eq!(
        page.read(|root| root.route.clone()),
        crate::surfaces::AppRoute::NewRole
    );
    assert!(page.read(|root| {
        let form = root.role_surfaces.create.as_ref().unwrap();
        form.error.is_some() && !form.submitting
    }));
}

#[test]
fn settings_returns_to_the_same_creating_draft() {
    use crate::surfaces::AppRoute;
    let mut page = role_page_harness("creating-settings-return");
    page.visual.run_until_parked();
    page.host
        .update(&mut page.visual, |root, window, cx| {
            root.open_create_role(window, cx);
            root.role_surfaces
                .create
                .as_ref()
                .unwrap()
                .display_name
                .update(cx, |input, cx| input.set_text("Draft role", cx));
            root.settings_return_route = AppRoute::NewRole;
            root.set_route(AppRoute::Settings, cx);
            root.leave_settings(window, cx);
            assert_eq!(root.route, AppRoute::NewRole);
            assert_eq!(
                root.role_surfaces
                    .create
                    .as_ref()
                    .unwrap()
                    .display_name
                    .read(cx)
                    .text(),
                "Draft role"
            );
        })
        .unwrap();
    page.visual.run_until_parked();
}
