use super::*;
use gpui::px;
use runner_core::protocol::runtime::ShellDiscoveryStatus;

#[test]
fn tabs_follow_catalog_support() {
    for runtime in [Runtime::Codex, Runtime::ClaudeCode] {
        assert!(supports_tab(runtime, AgentTab::General));
        assert!(supports_tab(runtime, AgentTab::Skills));
        assert!(supports_tab(runtime, AgentTab::Mcp));
    }
    assert!(supports_tab(Runtime::Pi, AgentTab::Skills));
    assert!(!supports_tab(Runtime::Pi, AgentTab::Mcp));
    assert!(!supports_tab(Runtime::Shell, AgentTab::Skills));
    assert!(!supports_tab(Runtime::Shell, AgentTab::Mcp));
}

#[test]
fn no_shell_description_reports_missing_login_shell() {
    let shell = ShellDiscoveryStatus {
        shell: None,
        outcome: Some(DiscoveryOutcome::NoShell),
        duration_ms: None,
        checking: false,
        using_last_known_good: false,
        last_known_good_captured_at: None,
    };
    assert_eq!(
        shell_description(Some(&shell)),
        "No supported login shell was configured; set an executable override below."
    );
}

#[test]
fn windows_registry_description_does_not_claim_a_shell_probe() {
    let shell = ShellDiscoveryStatus {
        shell: None,
        outcome: Some(DiscoveryOutcome::WindowsRegistry),
        duration_ms: Some(25),
        checking: false,
        using_last_known_good: false,
        last_known_good_captured_at: None,
    };
    assert_eq!(
        shell_description(Some(&shell)),
        "PATH refreshed from the Windows registry in 0.0 s. Spawned agents inherit it."
    );
}

#[test]
fn windows_registry_error_description_does_not_claim_a_shell_failure() {
    let shell = ShellDiscoveryStatus {
        shell: None,
        outcome: Some(DiscoveryOutcome::WindowsRegistryError),
        duration_ms: Some(25),
        checking: false,
        using_last_known_good: true,
        last_known_good_captured_at: Some("saved".into()),
    };
    assert_eq!(
        shell_description(Some(&shell)),
        "Could not read Windows PATH from the registry; refresh or set an override below."
    );
}

fn runtime(state: RuntimeRowState) -> RuntimeExecutableStatus {
    RuntimeExecutableStatus {
        launch_options: Default::default(),
        name: Runtime::Codex,
        display_name: "Codex".into(),
        command: "codex".into(),
        default_model: None,
        default_effort: None,
        detected_path: None,
        override_path: None,
        effective_command: None,
        effective_source: None,
        state,
        invalid_reason: None,
        installed_version: None,
        available_version: None,
        min_version: Some("0.149.0".into()),
        below_minimum: false,
    }
}

fn status(runtime: RuntimeExecutableStatus, checking: bool) -> RuntimeStatusResponse {
    RuntimeStatusResponse {
        shell: ShellDiscoveryStatus {
            shell: Some("/bin/zsh".into()),
            outcome: Some(DiscoveryOutcome::Ok),
            duration_ms: Some(25),
            checking,
            using_last_known_good: false,
            last_known_good_captured_at: None,
        },
        runtimes: vec![runtime],
    }
}

#[test]
fn partitions_all_six_states_in_catalog_order() {
    let rows = [
        RuntimeRowState::NotFound,
        RuntimeRowState::Detected,
        RuntimeRowState::InvalidOverride,
        RuntimeRowState::Override,
        RuntimeRowState::Checking,
        RuntimeRowState::ProbeTimedOut,
    ]
    .map(runtime);
    let (installed, not_installed) = partition_runtimes(&rows);
    assert_eq!(
        installed.iter().map(|row| row.state).collect::<Vec<_>>(),
        [
            RuntimeRowState::Detected,
            RuntimeRowState::Override,
            RuntimeRowState::Checking,
            RuntimeRowState::ProbeTimedOut,
        ]
    );
    assert_eq!(
        not_installed
            .iter()
            .map(|row| row.state)
            .collect::<Vec<_>>(),
        [RuntimeRowState::NotFound, RuntimeRowState::InvalidOverride]
    );
}

#[test]
fn partition_tracks_probe_and_override_transitions_without_stored_state() {
    let mut rows = [runtime(RuntimeRowState::NotFound)];
    for (state, installed_count) in [
        (RuntimeRowState::NotFound, 0),
        (RuntimeRowState::Checking, 1),
        (RuntimeRowState::Detected, 1),
        (RuntimeRowState::Checking, 1),
        (RuntimeRowState::ProbeTimedOut, 1),
        (RuntimeRowState::NotFound, 0),
        (RuntimeRowState::Override, 1),
        (RuntimeRowState::InvalidOverride, 0),
        (RuntimeRowState::NotFound, 0),
    ] {
        rows[0].state = state;
        let (installed, not_installed) = partition_runtimes(&rows);
        assert_eq!(installed.len(), installed_count, "{state:?}");
        assert_eq!(not_installed.len(), 1 - installed_count, "{state:?}");
        assert_eq!(
            runtime_presentation(&rows[0], None, None, false).spinning,
            state == RuntimeRowState::Checking,
        );
    }
}

#[test]
fn derives_every_runtime_row_state() {
    let cases = [
        (RuntimeRowState::Detected, None, "Detected"),
        (RuntimeRowState::Override, None, "Override"),
        (RuntimeRowState::NotFound, None, "Not found"),
        (RuntimeRowState::Checking, None, "Checking…"),
        (
            RuntimeRowState::ProbeTimedOut,
            Some(DiscoveryOutcome::Timeout),
            "Probe timed out",
        ),
        (RuntimeRowState::InvalidOverride, None, "Invalid"),
    ];
    for (state, outcome, badge) in cases {
        assert_eq!(
            runtime_presentation(&runtime(state), outcome, None, false).badge,
            badge
        );
    }
    assert_eq!(
        runtime_presentation(
            &runtime(RuntimeRowState::Detected),
            None,
            Some("Not executable"),
            false,
        )
        .badge,
        "Invalid"
    );
    assert_eq!(
        runtime_presentation(
            &runtime(RuntimeRowState::ProbeTimedOut),
            Some(DiscoveryOutcome::WindowsRegistryError),
            None,
            false,
        )
        .caption
        .as_deref(),
        Some("Windows PATH refresh failed. Refresh or set an explicit executable path.")
    );
}

fn versioned(
    state: RuntimeRowState,
    installed: Option<&str>,
    available: Option<&str>,
) -> RuntimeExecutableStatus {
    let mut row = runtime(state);
    row.detected_path = Some("/Users/jason/.nvm/bin/codex".into());
    row.installed_version = installed.map(str::to_owned);
    row.available_version = available.map(str::to_owned);
    row
}

#[test]
fn caption_leads_with_the_version_and_the_update_arrow() {
    let caption = |row: &RuntimeExecutableStatus, validation: Option<&str>| {
        runtime_presentation(row, Some(DiscoveryOutcome::Ok), validation, false).caption
    };
    assert_eq!(
        caption(
            &versioned(RuntimeRowState::Detected, Some("0.153.4"), None),
            None
        )
        .as_deref(),
        Some("0.153.4 · Detected: /Users/jason/.nvm/bin/codex")
    );
    assert_eq!(
        caption(
            &versioned(RuntimeRowState::Detected, Some("0.153.4"), Some("0.155.0")),
            None
        )
        .as_deref(),
        Some("0.153.4 → 0.155.0 · Detected: /Users/jason/.nvm/bin/codex")
    );
    assert_eq!(
        caption(
            &versioned(RuntimeRowState::Override, Some("0.155.1"), None),
            None
        )
        .as_deref(),
        Some("0.155.1 · Detected: /Users/jason/.nvm/bin/codex")
    );
    assert_eq!(
        caption(&versioned(RuntimeRowState::Detected, None, None), None),
        None
    );
    assert_eq!(
        caption(
            &versioned(RuntimeRowState::Detected, Some("0.153.4"), None),
            Some("Not an executable file.")
        )
        .as_deref(),
        Some("Not an executable file.")
    );
}

#[test]
fn update_button_shows_only_with_a_newer_version_and_allows_running_sessions() {
    let current = versioned(RuntimeRowState::Detected, Some("0.155.0"), None);
    assert_eq!(update_action(&current, 3), None);
    assert_eq!(
        update_action(&versioned(RuntimeRowState::Detected, None, None), 0),
        None
    );
    let mut trae = versioned(RuntimeRowState::Detected, Some("0.1.0"), None);
    trae.name = Runtime::Trae;
    trae.display_name = "TRAE CLI".into();
    assert_eq!(update_action(&trae, 0), None);

    let stale = versioned(RuntimeRowState::Detected, Some("0.153.4"), Some("0.155.0"));
    assert_eq!(
        update_action(&stale, 0),
        Some(UpdateAction { caption: None })
    );
    assert_eq!(
        update_action(&stale, 1),
        Some(UpdateAction {
            caption: Some("1 running Codex session keeps 0.153.4 until it relaunches.".into()),
        })
    );
    assert_eq!(
        update_action(&stale, 3),
        Some(UpdateAction {
            caption: Some("3 running Codex sessions keep 0.153.4 until they relaunch.".into()),
        })
    );
}

#[test]
fn update_button_sits_before_the_toggle_only_while_an_update_exists() {
    let mut cx = gpui::TestAppContext::single();
    let mut stale = versioned(RuntimeRowState::Detected, Some("0.153.4"), Some("0.155.0"));
    stale.effective_source = Some(RuntimeCommandSource::Detected);
    let (_temp, pane) = test_pane(vec![stale], false, &mut cx);
    let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
    let update = window.debug_bounds("AGENT_UPDATE_codex").unwrap();
    let toggle = window.debug_bounds("AGENT_TOGGLE_codex").unwrap();
    assert!(update.right() <= toggle.left());
    assert!(update.top() < toggle.bottom() && update.bottom() > toggle.top());
    assert!(window.debug_bounds("AGENT_UPDATE_CAPTION_codex").is_none());

    open_test_page(pane, Runtime::Codex, &mut window);
    pane.update(&mut window, |pane, _, cx| {
        pane.live_sessions.insert(Runtime::Codex, 2);
        cx.notify();
    })
    .unwrap();
    window.run_until_parked();
    assert!(window.debug_bounds("AGENT_UPDATE_CAPTION_codex").is_some());

    // A fresh window: gpui keeps debug bounds of elements no longer drawn.
    let mut cx = gpui::TestAppContext::single();
    let mut current = versioned(RuntimeRowState::Detected, Some("0.155.0"), None);
    current.effective_source = Some(RuntimeCommandSource::Detected);
    let (_temp, pane) = test_pane(vec![current], false, &mut cx);
    let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
    assert!(window.debug_bounds("AGENT_TOGGLE_codex").is_some());
    assert!(window.debug_bounds("AGENT_UPDATE_codex").is_none());
}

#[test]
fn update_task_row_shows_only_caption_unless_failed_and_omits_output_buttons() {
    use runner_core::protocol::{RuntimeUpdateState, RuntimeUpdateTask};

    let mut stale = versioned(RuntimeRowState::Detected, Some("0.153.4"), Some("0.155.0"));
    stale.effective_source = Some(RuntimeCommandSource::Detected);
    let make_task = |state: RuntimeUpdateState| RuntimeUpdateTask {
        id: "task-codex-1".into(),
        runtime: Runtime::Codex,
        state,
        from_version: Some("0.153.4".into()),
        to_version: Some("0.155.0".into()),
        installed_version: (state == RuntimeUpdateState::Succeeded).then(|| "0.155.0".into()),
        step: String::new(),
        message: None,
        exit_code: None,
        running_sessions: 0,
    };

    // Active, succeeded, and may-need-input states render only the vertically centered caption
    // (no Retry/Reinstall button and no Show output / Answer button).
    for state in [
        RuntimeUpdateState::Running,
        RuntimeUpdateState::Succeeded,
        RuntimeUpdateState::MayNeedInput,
    ] {
        let mut cx = gpui::TestAppContext::single();
        let (_temp, pane) = test_pane(vec![stale.clone()], false, &mut cx);
        pane.update(&mut cx, |pane, _, cx| {
            pane.tasks = vec![make_task(state)];
            cx.notify();
        })
        .unwrap();
        cx.run_until_parked();
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        let card = window.debug_bounds("AGENT_CARD_codex").unwrap();
        let toggle = window.debug_bounds("AGENT_TOGGLE_codex").unwrap();
        let update = window.debug_bounds("AGENT_UPDATE_codex").unwrap();
        let caption = window.debug_bounds("AGENT_TASK_CAPTION_codex").unwrap();
        assert!(
            window.debug_bounds("AGENT_TASK_RETRY_codex").is_none(),
            "{state:?} should not show Retry/Reinstall"
        );
        assert!(
            (update.size.width - caption.size.width).abs() <= px(0.5)
                && (update.size.height - caption.size.height).abs() <= px(0.5),
            "{state:?} update container {update:?} should collapse to caption only {caption:?}"
        );
        assert!(
            (caption.center().y - toggle.center().y).abs() <= px(1.)
                && (caption.center().y - card.center().y).abs() <= px(1.),
            "{state:?} caption {caption:?} should center vertically against toggle {toggle:?} and card {card:?}"
        );

        // Detail page header also renders only the centered caption without output/answer buttons.
        open_test_page(pane, Runtime::Codex, &mut window);
        let detail_toggle = window.debug_bounds("AGENT_TOGGLE_codex").unwrap();
        let detail_update = window.debug_bounds("AGENT_UPDATE_codex").unwrap();
        let detail_caption = window.debug_bounds("AGENT_TASK_CAPTION_codex").unwrap();
        assert!(window.debug_bounds("AGENT_TASK_RETRY_codex").is_none());
        assert!(
            (detail_update.size.height - detail_caption.size.height).abs() <= px(0.5),
            "{state:?} detail header update {detail_update:?} should collapse to caption {detail_caption:?}"
        );
        assert!(
            (detail_caption.center().y - detail_toggle.center().y).abs() <= px(1.),
            "{state:?} detail caption {detail_caption:?} should center against toggle {detail_toggle:?}"
        );
    }

    // Failed and BrokenAfterStop keep only Retry / Reinstall alongside the caption (still no
    // Show output / Answer button).
    for state in [
        RuntimeUpdateState::Failed,
        RuntimeUpdateState::BrokenAfterStop,
    ] {
        let mut cx = gpui::TestAppContext::single();
        let (_temp, pane) = test_pane(vec![stale.clone()], false, &mut cx);
        pane.update(&mut cx, |pane, _, cx| {
            pane.tasks = vec![make_task(state)];
            cx.notify();
        })
        .unwrap();
        cx.run_until_parked();
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        let update = window.debug_bounds("AGENT_UPDATE_codex").unwrap();
        let caption = window.debug_bounds("AGENT_TASK_CAPTION_codex").unwrap();
        let retry = window.debug_bounds("AGENT_TASK_RETRY_codex").unwrap();
        assert!(retry.top() >= caption.bottom());
        assert!(
            (update.size.width - caption.size.width.max(retry.size.width)).abs() <= px(1.),
            "{state:?} button row should hold only Retry/Reinstall, got update={update:?}, caption={caption:?}, retry={retry:?}"
        );
        assert!(
            (update.size.height - (caption.size.height + retry.size.height + px(4.))).abs()
                <= px(1.),
            "{state:?} update height {update:?} should match caption + gap + retry"
        );
    }
}

#[test]
fn formats_known_partial_and_unknown_runtime_defaults() {
    let mut runtime = runtime(RuntimeRowState::Detected);
    runtime.default_model = Some("claude-fable-5[1m]".into());
    runtime.default_effort = Some("xhigh".into());
    assert_eq!(
        runtime_default_value(runtime.default_model.as_deref()),
        "claude-fable-5[1m]"
    );
    assert_eq!(
        runtime_default_effort_value(runtime.default_effort.as_deref()),
        "xhigh"
    );

    runtime.default_effort = None;
    assert_eq!(
        runtime_default_effort_value(runtime.default_effort.as_deref()),
        "runtime default"
    );
}

fn test_store(path: &std::path::Path, cx: &mut gpui::TestAppContext) -> Entity<AppStore> {
    use runner_daemon::{db, session, shell_path};
    use std::sync::{Arc, RwLock};
    let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
    let runtime_discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
    let core = crate::test_support::core(
        Arc::new(db::open_fixture_pool(&path.join("runner.db")).unwrap()),
        path.into(),
        session::SessionManager::new(
            runtime_shell_env.clone(),
            runtime_discovery.clone(),
            Arc::new(session::pty_runtime::PtyRuntime::new()),
        ),
        runtime_shell_env,
        runtime_discovery,
    );
    cx.new(|cx| {
        AppStore::new(
            core,
            None,
            None,
            path.join("settings.json"),
            AppSettings::default(),
            None,
            cx,
        )
    })
}

fn test_pane(
    rows: Vec<RuntimeExecutableStatus>,
    checking: bool,
    cx: &mut gpui::TestAppContext,
) -> (tempfile::TempDir, gpui::WindowHandle<AgentsPane>) {
    test_pane_with_settings(rows, checking, AppSettings::default(), cx)
}

fn test_catalog() -> Vec<RuntimeCatalogEntry> {
    runner_core::protocol::runtime_metadata::runtime_list()
        .into_iter()
        .map(|entry| RuntimeCatalogEntry {
            installed_version: None,
            available_version: None,
            min_version: None,
            below_minimum: false,
            name: entry.name,
            capabilities: runner_core::protocol::runtime::RuntimeCatalogEntry::for_runtime(
                entry.name,
            )
            .map(|entry| entry.capabilities)
            .unwrap_or_default(),
            display_name: entry.display_name,
            command: entry.command,
            native_fork: entry.native_fork,
            description: if entry.name == Runtime::Trae {
                String::new()
            } else {
                "Test agent description".into()
            },
            install_url: if entry.name == Runtime::Trae {
                String::new()
            } else {
                "https://example.com/install".into()
            },
            default_enabled: true,
            available: false,
            default_model: None,
            default_effort: None,
            models: Vec::new(),
            efforts: Vec::new(),
        })
        .collect()
}

fn test_pane_with_settings(
    rows: Vec<RuntimeExecutableStatus>,
    checking: bool,
    settings: AppSettings,
    cx: &mut gpui::TestAppContext,
) -> (tempfile::TempDir, gpui::WindowHandle<AgentsPane>) {
    let temp = tempfile::tempdir().unwrap();
    let store = test_store(temp.path(), cx);
    store.update(cx, |store, _| store.settings = settings);
    let pane = cx.add_window(|window, cx| {
        window.resize(gpui::size(px(1200.), px(1200.)));
        let mut pane = AgentsPane::new(WeakEntity::new_invalid(), store, window, cx);
        pane.catalog = test_catalog();
        let mut snapshot = status(runtime(RuntimeRowState::NotFound), checking);
        snapshot.runtimes = rows;
        pane.apply_status(snapshot, cx);
        pane
    });
    cx.run_until_parked();
    (temp, pane)
}

fn open_test_page(
    pane: gpui::WindowHandle<AgentsPane>,
    runtime: Runtime,
    window: &mut gpui::VisualTestContext,
) {
    let row = window
        .debug_bounds(Box::leak(format!("AGENT_CARD_{runtime}").into_boxed_str()))
        .unwrap();
    window.simulate_click(
        gpui::point(row.left() + px(24.), row.center().y),
        gpui::Modifiers::default(),
    );
    window.run_until_parked();
    pane.read_with(window, |pane, _| {
        assert_eq!(pane.selected_runtime, Some(runtime))
    })
    .unwrap();
}

#[test]
fn all_installed_collapses_not_installed_to_one_line() {
    let mut cx = gpui::TestAppContext::single();
    let (_temp, pane) = test_pane(vec![runtime(RuntimeRowState::Detected)], false, &mut cx);
    let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
    assert!(window.debug_bounds("AGENTS_INSTALLED_EMPTY").is_none());
    assert!(window.debug_bounds("AGENTS_NOT_INSTALLED_HEADER").is_none());
    let empty = window.debug_bounds("AGENTS_NOT_INSTALLED_EMPTY").unwrap();
    assert!(empty.size.height <= px(18.));
    let installed = window.debug_bounds("AGENTS_INSTALLED").unwrap();
    let card = window.debug_bounds("AGENT_CARD_codex").unwrap();
    assert!(installed.top() <= card.top() && installed.bottom() >= card.bottom());
    assert!(empty.top() >= installed.bottom());
}

#[test]
fn checking_does_not_claim_all_agents_are_installed() {
    let mut cx = gpui::TestAppContext::single();
    let (_temp, pane) = test_pane(vec![runtime(RuntimeRowState::Checking)], true, &mut cx);
    let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
    assert!(window.debug_bounds("AGENTS_NOT_INSTALLED_EMPTY").is_none());
    assert!(window.debug_bounds("AGENTS_NOT_INSTALLED_HEADER").is_none());
    let installed = window.debug_bounds("AGENTS_INSTALLED").unwrap();
    let card = window.debug_bounds("AGENT_CARD_codex").unwrap();
    assert!(installed.top() <= card.top() && installed.bottom() >= card.bottom());
    assert!(window.debug_bounds("AGENT_BADGE_codex").is_some());
}

#[test]
fn none_installed_invites_installation_above_all_missing_cards() {
    let mut cx = gpui::TestAppContext::single();
    let rows = runner_core::protocol::runtime_metadata::runtime_list()
        .into_iter()
        .map(|entry| {
            let mut row = runtime(RuntimeRowState::NotFound);
            row.name = entry.name;
            row.display_name = entry.display_name;
            row.command = entry.command;
            row
        })
        .collect();
    let (_temp, pane) = test_pane(rows, false, &mut cx);
    let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
    let invitation = window.debug_bounds("AGENTS_INSTALLED_EMPTY").unwrap();
    let header = window.debug_bounds("AGENTS_NOT_INSTALLED_HEADER").unwrap();
    assert!(invitation.bottom() < header.top());
    assert!(window.debug_bounds("AGENTS_NOT_INSTALLED_EMPTY").is_none());
    let mut previous_bottom = header.bottom();
    for selector in [
        "AGENT_CARD_codex",
        "AGENT_CARD_claude-code",
        "AGENT_CARD_antigravity",
        "AGENT_CARD_pi",
        "AGENT_CARD_copilot",
        "AGENT_CARD_trae",
    ] {
        let card = window.debug_bounds(selector).unwrap();
        assert!(card.top() >= previous_bottom, "{selector}");
        previous_bottom = card.bottom();
    }
}

#[test]
fn install_link_shares_the_description_line_and_both_are_omitted_when_empty() {
    let mut cx = gpui::TestAppContext::single();
    let rows = runner_core::protocol::runtime_metadata::runtime_list()
        .into_iter()
        .filter(|entry| matches!(entry.name, Runtime::Codex | Runtime::Trae))
        .map(|entry| {
            let mut row = runtime(RuntimeRowState::NotFound);
            row.name = entry.name;
            row.display_name = entry.display_name;
            row.command = entry.command;
            row
        })
        .collect();
    let (_temp, pane) = test_pane(rows, false, &mut cx);
    let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
    open_test_page(pane, Runtime::Codex, &mut window);
    let description = window.debug_bounds("AGENT_DESCRIPTION_codex").unwrap();
    let install = window.debug_bounds("AGENT_INSTALL_codex").unwrap();
    assert!(install.left() > description.right());
    assert!(install.top() < description.bottom() && install.bottom() > description.top());
    pane.update(&mut window, |pane, _, cx| {
        pane.open_runtime(Runtime::Trae, cx);
    })
    .unwrap();
    window.run_until_parked();
    assert!(window.debug_bounds("AGENT_PAGE_trae").is_some());
    assert!(window.debug_bounds("AGENT_DESCRIPTION_trae").is_none());
    assert!(window.debug_bounds("AGENT_INSTALL_trae").is_none());
}

#[test]
fn invalid_override_keeps_validation_and_moves_after_a_valid_override() {
    let mut row = runtime(RuntimeRowState::InvalidOverride);
    row.override_path = Some("/missing/codex".into());
    row.invalid_reason = Some("Codex executable does not exist: /missing/codex".into());
    let mut cx = gpui::TestAppContext::single();
    let (_temp, pane) = test_pane(vec![row], false, &mut cx);
    let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
    open_test_page(pane, Runtime::Codex, &mut window);
    assert!(window.debug_bounds("AGENT_VALIDATION_codex").is_some());
    assert!(window.debug_bounds("AGENT_RESET_codex").is_some());
    assert!(window.debug_bounds("AGENT_INSTALL_codex").is_some());
    assert!(window.debug_bounds("AGENT_MODEL_LINE_codex").is_none());
    assert!(window.debug_bounds("AGENT_TOGGLE_codex").is_none());
    assert!(window.debug_bounds("AGENT_SET_DEFAULT_codex").is_none());
    assert!(window.debug_bounds("AGENT_BADGE_codex").is_none());
    pane.update(&mut window, |pane, _, cx| {
        let mut row = runtime(RuntimeRowState::Override);
        row.override_path = Some("/valid/codex".into());
        row.effective_source = Some(RuntimeCommandSource::Override);
        pane.apply_status(status(row, false), cx);
        assert!(pane.validation.is_empty());
        cx.notify();
    })
    .unwrap();
    window.run_until_parked();
    assert!(window.debug_bounds("AGENT_MODEL_LINE_codex").is_some());
    assert!(window.debug_bounds("AGENT_TOGGLE_codex").is_some());
    assert!(window.debug_bounds("AGENT_SET_DEFAULT_codex").is_some());
    assert!(window.debug_bounds("AGENT_BADGE_codex").is_none());
    pane.read_with(&window, |pane, _| {
        assert_eq!(pane.selected_runtime, Some(Runtime::Codex))
    })
    .unwrap();
    pane.update(&mut window, |pane, _, cx| {
        pane.apply_status(status(runtime(RuntimeRowState::NotFound), false), cx);
        cx.notify();
    })
    .unwrap();
    window.run_until_parked();
    assert!(window.debug_bounds("AGENT_PAGE_codex").is_some());
}

#[test]
fn compact_rows_and_agent_defaults_fit_at_two_rem_sizes() {
    for rem in [16., 20.8] {
        let mut cx = gpui::TestAppContext::single();
        let mut claude = runtime(RuntimeRowState::Detected);
        claude.name = Runtime::ClaudeCode;
        claude.display_name = "Claude Code".into();
        claude.default_model = Some("claude-fable-5[1m]".into());
        claude.default_effort = Some("xhigh".into());
        let (_temp, pane) = test_pane(
            vec![claude, runtime(RuntimeRowState::NotFound)],
            false,
            &mut cx,
        );
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        pane.update(&mut window, |_, window, _| {
            window.set_rem_size(px(rem));
            window.refresh();
        })
        .unwrap();
        window.run_until_parked();
        let row = window.debug_bounds("AGENT_CARD_claude-code").unwrap();
        assert!((row.size.height - px(64. * rem / 16.)).abs() <= px(1.));
        assert!(window
            .debug_bounds("AGENT_MODEL_LINE_claude-code")
            .is_none());
        open_test_page(pane, Runtime::ClaudeCode, &mut window);
        let page = window.debug_bounds("AGENT_PAGE_claude-code").unwrap();
        let model = window.debug_bounds("AGENT_MODEL_LINE_claude-code").unwrap();
        assert!(model.bottom() <= page.bottom());
        assert!(window.debug_bounds("AGENT_MCP_LINE_claude-code").is_none());
        assert!(window.debug_bounds("AGENT_MODEL_LINE_codex").is_none());
    }
}

#[test]
fn every_runtime_row_opens_its_page_and_breadcrumb_returns() {
    for installed in [false, true] {
        let mut cx = gpui::TestAppContext::single();
        let rows: Vec<_> = runner_core::protocol::runtime_metadata::runtime_list()
            .into_iter()
            .map(|entry| {
                let mut row = runtime(if installed {
                    RuntimeRowState::Detected
                } else {
                    RuntimeRowState::NotFound
                });
                row.name = entry.name;
                row.display_name = entry.display_name;
                row.command = entry.command;
                row
            })
            .collect();
        let names: Vec<_> = rows.iter().map(|row| row.name).collect();
        let (_temp, pane) = test_pane(rows, false, &mut cx);
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        for name in names {
            open_test_page(pane, name, &mut window);
            assert!(window
                .debug_bounds(Box::leak(format!("AGENT_PAGE_{name}").into_boxed_str()))
                .is_some());
            let back = window.debug_bounds("AGENTS_BREADCRUMB").unwrap();
            window.simulate_click(back.center(), gpui::Modifiers::default());
            window.run_until_parked();
            pane.read_with(&window, |pane, _| assert_eq!(pane.selected_runtime, None))
                .unwrap();
        }
    }
}

#[test]
fn list_toggle_changes_preference_without_opening_the_agent() {
    let mut cx = gpui::TestAppContext::single();
    let (_temp, pane) = test_pane(vec![runtime(RuntimeRowState::Detected)], false, &mut cx);
    let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
    let toggle = window.debug_bounds("AGENT_TOGGLE_codex").unwrap();
    window.simulate_click(toggle.center(), gpui::Modifiers::default());
    window.run_until_parked();
    pane.read_with(&window, |pane, cx| {
        assert_eq!(pane.selected_runtime, None);
        assert!(pane
            .app_store
            .read(cx)
            .settings
            .disabled_agents
            .contains("codex"));
    })
    .unwrap();
}

#[test]
fn hides_runtime_defaults_when_the_runtime_is_not_found() {
    assert!(runtime_defaults_visible(&runtime(
        RuntimeRowState::Detected
    )));
    assert!(runtime_defaults_visible(&runtime(
        RuntimeRowState::Checking
    )));
    assert!(!runtime_defaults_visible(&runtime(
        RuntimeRowState::NotFound
    )));
    assert!(!runtime_defaults_visible(&runtime(
        RuntimeRowState::InvalidOverride
    )));
}

#[test]
fn default_eligibility_requires_enabled_sources() {
    let catalog = test_catalog();
    let mut settings = AppSettings::default();
    settings.disabled_agents.insert("codex".into());
    for state in [
        RuntimeRowState::Detected,
        RuntimeRowState::Override,
        RuntimeRowState::Checking,
        RuntimeRowState::ProbeTimedOut,
        RuntimeRowState::NotFound,
        RuntimeRowState::InvalidOverride,
    ] {
        let mut row = runtime(state);
        assert!(!runtime_default_eligible(
            &row,
            &catalog,
            &AppSettings::default()
        ));
        row.effective_source = Some(RuntimeCommandSource::Detected);
        assert!(runtime_default_eligible(
            &row,
            &catalog,
            &AppSettings::default()
        ));
        assert!(!runtime_default_eligible(&row, &catalog, &settings));
    }
}

#[test]
fn card_actions_pin_and_switch_persisted_default() {
    let mut codex = runtime(RuntimeRowState::Detected);
    codex.effective_source = Some(RuntimeCommandSource::Detected);
    let mut claude = codex.clone();
    claude.name = Runtime::ClaudeCode;
    claude.display_name = "Claude Code".into();
    let mut cx = gpui::TestAppContext::single();
    let (temp, pane) = test_pane(vec![codex, claude], false, &mut cx);
    let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
    assert!(window.debug_bounds("AGENT_DEFAULT_codex").is_none());
    assert!(window.debug_bounds("AGENT_ENABLED_codex").is_none());
    assert!(window.debug_bounds("AGENT_BADGE_codex").is_none());
    assert!(window.debug_bounds("AGENT_BADGE_claude-code").is_none());
    open_test_page(pane, Runtime::Codex, &mut window);
    for (selector, expected) in [
        ("AGENT_SET_DEFAULT_codex", "codex"),
        ("AGENT_SET_DEFAULT_claude-code", "claude-code"),
    ] {
        let runtime = if expected == "codex" {
            Runtime::Codex
        } else {
            Runtime::ClaudeCode
        };
        pane.update(&mut window, |pane, _, cx| pane.open_runtime(runtime, cx))
            .unwrap();
        window.run_until_parked();
        let button = window.debug_bounds(selector).unwrap();
        window.simulate_click(button.center(), gpui::Modifiers::default());
        window.run_until_parked();
        pane.update(&mut window, |pane, _, cx| {
            assert_eq!(pane.app_store.read(cx).settings.default_runtime, expected);
        })
        .unwrap();
        assert_eq!(
            AppSettings::load(&temp.path().join("settings.json"))
                .unwrap()
                .default_runtime,
            expected
        );
        let (marker_selector, identity_selector) = if expected == "codex" {
            ("AGENT_DEFAULT_codex", "AGENT_IDENTITY_codex")
        } else {
            ("AGENT_DEFAULT_claude-code", "AGENT_IDENTITY_claude-code")
        };
        let marker = window.debug_bounds(marker_selector).unwrap();
        let identity = window.debug_bounds(identity_selector).unwrap();
        assert!(
            identity.left() <= marker.left()
                && identity.right() >= marker.right()
                && identity.top() <= marker.top()
                && identity.bottom() >= marker.bottom()
        );
    }
}

#[test]
fn checking_disables_default_action_and_preserves_explicit_marker() {
    for explicit in [false, true] {
        let mut row = runtime(RuntimeRowState::Checking);
        row.effective_source = Some(RuntimeCommandSource::Detected);
        let mut cx = gpui::TestAppContext::single();
        let (_temp, pane) = test_pane_with_settings(
            vec![row],
            true,
            AppSettings {
                default_runtime: if explicit { "codex" } else { "" }.into(),
                ..AppSettings::default()
            },
            &mut cx,
        );
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        open_test_page(pane, Runtime::Codex, &mut window);
        assert!(window.debug_bounds("AGENT_BADGE_codex").is_some());
        if explicit {
            assert!(window.debug_bounds("AGENT_DEFAULT_codex").is_some());
            assert!(window.debug_bounds("AGENT_SET_DEFAULT_codex").is_none());
        } else {
            let button = window.debug_bounds("AGENT_SET_DEFAULT_codex").unwrap();
            window.simulate_click(button.center(), gpui::Modifiers::default());
            window.run_until_parked();
        }
        pane.update(&mut window, |pane, _, cx| {
            assert_eq!(
                pane.app_store.read(cx).settings.default_runtime,
                if explicit { "codex" } else { "" }
            );
            pane.apply_status(status(runtime(RuntimeRowState::NotFound), false), cx);
            assert!(pane.app_store.read(cx).settings.default_runtime.is_empty());
        })
        .unwrap();
    }
}

#[test]
fn unavailable_or_disabled_agents_have_no_default_action() {
    for (state, source, disabled) in [
        (RuntimeRowState::NotFound, None, false),
        (
            RuntimeRowState::InvalidOverride,
            Some(RuntimeCommandSource::Detected),
            false,
        ),
        (RuntimeRowState::Checking, None, false),
        (
            RuntimeRowState::ProbeTimedOut,
            Some(RuntimeCommandSource::Catalog),
            false,
        ),
        (
            RuntimeRowState::Detected,
            Some(RuntimeCommandSource::Detected),
            true,
        ),
    ] {
        let mut row = runtime(state);
        row.effective_source = source;
        let mut settings = AppSettings::default();
        if disabled {
            settings.disabled_agents.insert("codex".into());
        }
        let mut cx = gpui::TestAppContext::single();
        let (_temp, pane) = test_pane_with_settings(
            vec![row],
            state == RuntimeRowState::Checking,
            settings,
            &mut cx,
        );
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        open_test_page(pane, Runtime::Codex, &mut window);
        assert!(
            window.debug_bounds("AGENT_SET_DEFAULT_codex").is_none(),
            "{state:?}"
        );
        assert!(
            window.debug_bounds("AGENT_DEFAULT_codex").is_none(),
            "{state:?}"
        );
    }
}

#[test]
fn installed_header_wraps_without_overlapping_actions() {
    let mut row = runtime(RuntimeRowState::Override);
    row.name = Runtime::Copilot;
    row.display_name = "GitHub Copilot CLI".into();
    row.effective_source = Some(RuntimeCommandSource::Override);
    let mut cx = gpui::TestAppContext::single();
    let (_temp, pane) = test_pane(vec![row], false, &mut cx);
    let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
    open_test_page(pane, Runtime::Copilot, &mut window);
    for rem in [16., 20.8] {
        for width in [320., 480., 760.] {
            pane.update(&mut window, |_, window, _| {
                window.set_rem_size(px(rem));
                window.refresh();
            })
            .unwrap();
            window.simulate_resize(gpui::size(px(width), px(1200.)));
            window.run_until_parked();
            let header = window.debug_bounds("AGENT_HEADER_copilot").unwrap();
            let identity = window.debug_bounds("AGENT_IDENTITY_copilot").unwrap();
            let actions = window.debug_bounds("AGENT_ACTIONS_copilot").unwrap();
            assert!(header.left() >= px(0.) && header.right() <= px(width));
            if width == 320. {
                assert!(identity.size.height > px(26. * rem / 16.));
                assert!(identity.bottom() <= actions.top());
            }
            for bounds in [identity, actions] {
                assert!(
                    header.left() <= bounds.left()
                        && header.right() >= bounds.right()
                        && header.top() <= bounds.top()
                        && header.bottom() >= bounds.bottom(),
                    "width {width}, rem {rem}: {header:?} {bounds:?}"
                );
            }
            assert!(
                identity.right() <= actions.left() || identity.bottom() <= actions.top(),
                "width {width}, rem {rem}: {identity:?} {actions:?}"
            );
            let button = window.debug_bounds("AGENT_SET_DEFAULT_copilot").unwrap();
            let toggle = window.debug_bounds("AGENT_TOGGLE_copilot").unwrap();
            assert!(
                identity.left() <= button.left()
                    && identity.right() >= button.right()
                    && identity.top() <= button.top()
                    && identity.bottom() >= button.bottom()
            );
            assert!(button.right() < toggle.left() || button.bottom() <= toggle.top());
        }
    }
}

#[test]
fn agent_preferences_persist_explicit_enable_disable_and_clear_default() {
    let mut settings = AppSettings {
        default_runtime: "codex".into(),
        ..AppSettings::default()
    };
    assert!(update_agent_enabled_preferences(
        &mut settings,
        Runtime::Codex,
        false
    ));
    assert!(settings.disabled_agents.contains(Runtime::Codex.key()));
    assert!(!settings.enabled_agents.contains(Runtime::Codex.key()));
    assert!(settings.default_runtime.is_empty());
    assert!(update_agent_enabled_preferences(
        &mut settings,
        Runtime::Codex,
        true
    ));
    assert!(!settings.disabled_agents.contains(Runtime::Codex.key()));
    assert!(settings.enabled_agents.contains(Runtime::Codex.key()));
    assert!(update_default_runtime(&mut settings, Some(Runtime::Codex)));
    assert_eq!(settings.default_runtime, "codex");
    assert!(!update_default_runtime(&mut settings, Some(Runtime::Codex)));
}

#[test]
fn unavailable_default_waits_for_discovery_then_clears() {
    let catalog = vec![RuntimeCatalogEntry {
        installed_version: None,
        available_version: None,
        min_version: None,
        below_minimum: false,
        name: Runtime::Codex,
        capabilities: runner_core::protocol::runtime::RuntimeCatalogEntry::for_runtime(
            Runtime::Codex,
        )
        .map(|entry| entry.capabilities)
        .unwrap_or_default(),
        display_name: "Codex".into(),
        command: "codex".into(),
        native_fork: true,
        description: String::new(),
        install_url: String::new(),
        default_enabled: true,
        available: false,
        default_model: None,
        default_effort: None,
        models: Vec::new(),
        efforts: Vec::new(),
    }];
    let mut settings = AppSettings {
        default_runtime: "codex".into(),
        ..AppSettings::default()
    };
    assert!(!reconcile_default_runtime(
        &mut settings,
        &status(runtime(RuntimeRowState::Checking), true),
        &catalog,
    ));
    assert!(reconcile_default_runtime(
        &mut settings,
        &status(runtime(RuntimeRowState::NotFound), false),
        &catalog,
    ));
    assert!(settings.default_runtime.is_empty());
}

#[cfg(debug_assertions)]
#[test]
fn mounted_agents_placeholders_follow_locale_without_reloading_drafts() {
    use crate::i18n::{self, Language, Locale};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    i18n::testing::with_locale(Locale::English, || {
        for (state, path, english, chinese) in [
            (
                RuntimeRowState::Checking,
                None,
                "Auto — detecting…",
                "自动 — 正在检测…",
            ),
            (
                RuntimeRowState::Detected,
                Some("/tmp/{path}/codex"),
                "Auto — /tmp/{path}/codex",
                "自动 — /tmp/{path}/codex",
            ),
            (
                RuntimeRowState::NotFound,
                None,
                "Auto — codex not found on PATH",
                "自动 — PATH 中未找到 codex",
            ),
        ] {
            let mut cx = gpui::TestAppContext::single();
            let mut row = runtime(state);
            row.detected_path = path.map(str::to_owned);
            let (_temp, pane) = test_pane_with_settings(
                vec![row],
                false,
                AppSettings {
                    language: Language::English,
                    ..AppSettings::default()
                },
                &mut cx,
            );
            let (store, field) = pane
                .read_with(&cx, |pane, _| {
                    (
                        pane.app_store.clone(),
                        pane.overrides[&Runtime::Codex].clone(),
                    )
                })
                .unwrap();
            let peer = cx.add_window(|window, cx| {
                AgentsPane::new(WeakEntity::new_invalid(), store.clone(), window, cx)
            });
            field.update(&mut cx, |field, cx| {
                field.set_text("/tmp/draft {count} 中文", cx);
                field.select_all(cx);
            });
            let caret = field.read_with(&cx, |field, _| field.caret_offset());
            let mut visual = gpui::VisualTestContext::from_window(pane.into(), &cx);
            open_test_page(pane, Runtime::Codex, &mut visual);
            for (language, expected) in [
                (Language::SimplifiedChinese, chinese),
                (Language::English, english),
            ] {
                peer.update(&mut visual, |root, _, cx| {
                    root.app_store.update(cx, |store, cx| {
                        store.update_settings(
                            |settings| {
                                settings.language = language;
                                true
                            },
                            false,
                            cx,
                        )
                    })
                })
                .unwrap();
                visual.run_until_parked();
                field.read_with(&visual, |field, _| {
                    assert_eq!(field.placeholder(), expected);
                    assert_eq!(field.text(), "/tmp/draft {count} 中文");
                    assert_eq!(field.caret_offset(), caret);
                });
                pane.read_with(&visual, |pane, _| {
                    assert_eq!(
                        pane.overrides[&Runtime::Codex].entity_id(),
                        field.entity_id()
                    )
                })
                .unwrap();
            }
        }
    });
}

#[cfg(debug_assertions)]
#[test]
fn chinese_agent_defaults_translate_effort_and_keep_model_names_literal() {
    crate::i18n::testing::with_locale(crate::i18n::Locale::SimplifiedChinese, || {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let mut row = runtime(RuntimeRowState::Detected);
        row.default_model = Some("xhigh".into());
        row.default_effort = Some("xhigh".into());
        assert_eq!(runtime_default_value(row.default_model.as_deref()), "xhigh");
        assert_eq!(
            runtime_default_effort_value(row.default_effort.as_deref()),
            "极高"
        );
        assert_eq!(runtime_default_effort_value(None), "运行时默认");
        let mut cx = gpui::TestAppContext::single();
        let (_temp, pane) = test_pane(vec![row], false, &mut cx);
        let mut visual = gpui::VisualTestContext::from_window(pane.into(), &cx);
        open_test_page(pane, Runtime::Codex, &mut visual);
        assert!(visual.debug_bounds("AGENT_PROPERTY_VALUE 极高").is_some());
        assert!(visual.debug_bounds("AGENT_PROPERTY_VALUE xhigh").is_some());
    });
}

#[test]
fn override_validation_preserves_backend_copy() {
    let error = OverrideValidationError {
        code: "not_executable".into(),
        message: "Codex executable is not executable: /tmp/codex".into(),
    };
    assert_eq!(
        override_validation_error_message(&error),
        "Codex executable is not executable: /tmp/codex"
    );
}
#[test]
fn launch_card_switches_follow_the_runtime_table() {
    for name in [
        Runtime::Codex,
        Runtime::ClaudeCode,
        Runtime::Pi,
        Runtime::Antigravity,
        Runtime::Cursor,
    ] {
        let mut cx = gpui::TestAppContext::single();
        let mut row = runtime(RuntimeRowState::Detected);
        row.name = name;
        let (_temp, pane) = test_pane(vec![row], false, &mut cx);
        pane.update(&mut cx, |pane, _, cx| {
            pane.selected_runtime = Some(name);
            pane.launch_previews.insert(
                (name, false),
                runner_core::protocol::runtime::RuntimeCommandPreview {
                    lines: vec![runner_core::protocol::runtime::RuntimeCommandLine {
                        command: "chat command".into(),
                        note: runner_core::protocol::runtime::RuntimeCommandNote::None,
                        option: None,
                    }],
                    custom_flags_warning: None,
                },
            );
            cx.notify();
        })
        .unwrap();
        cx.run_until_parked();
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        assert!(window
            .debug_bounds(Box::leak(format!("AGENT_LAUNCH_{name}").into_boxed_str()))
            .is_some());
        for option in [
            runner_core::protocol::runtime::RuntimeLaunchOption::Bypass,
            runner_core::protocol::runtime::RuntimeLaunchOption::RemotePluginCatalog,
            runner_core::protocol::runtime::RuntimeLaunchOption::StatusHooks,
        ] {
            assert_eq!(
                window
                    .debug_bounds(Box::leak(
                        format!("AGENT_LAUNCH_SWITCH_{name}_{option:?}").into_boxed_str()
                    ))
                    .is_some(),
                option.supported(name)
            );
        }
        assert!(window
            .debug_bounds(Box::leak(
                format!("AGENT_LAUNCH_COMMAND_{name}").into_boxed_str()
            ))
            .is_some());
    }
}

#[test]
fn launch_copy_shows_feedback_and_ignores_unavailable_previews() {
    use gpui::Focusable as _;
    use runner_core::protocol::runtime::{
        RuntimeCommandLine, RuntimeCommandNote, RuntimeCommandPreview,
    };
    use std::time::Duration;

    let mut cx = gpui::TestAppContext::single();
    let mut codex = runtime(RuntimeRowState::Detected);
    codex.name = Runtime::Codex;
    let mut claude = runtime(RuntimeRowState::Detected);
    claude.name = Runtime::ClaudeCode;
    claude.display_name = "Claude Code".into();
    let (_temp, pane) = test_pane(vec![codex, claude], false, &mut cx);
    pane.update(&mut cx, |pane, _, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("initial".into()));
        pane.selected_runtime = Some(Runtime::Codex);
        cx.notify();
    })
    .unwrap();
    cx.run_until_parked();
    let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);

    let copy = window.debug_bounds("AGENT_LAUNCH_COPY").unwrap();
    window.simulate_click(copy.center(), gpui::Modifiers::default());
    window.run_until_parked();
    pane.read_with(&window, |pane, cx| {
        assert!(!pane.launch_copy.read(cx).is_copied());
        assert_eq!(pane.launch_copy.read(cx).value(), None);
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("initial")
        );
    })
    .unwrap();

    pane.update(&mut window, |pane, _, cx| {
        pane.launch_previews.insert(
            (Runtime::Codex, false),
            RuntimeCommandPreview {
                lines: vec![
                    RuntimeCommandLine {
                        command: "codex -m <model>".into(),
                        note: RuntimeCommandNote::FromChat,
                        option: None,
                    },
                    RuntimeCommandLine {
                        command: "<first message>".into(),
                        note: RuntimeCommandNote::FirstTurn,
                        option: None,
                    },
                ],
                custom_flags_warning: None,
            },
        );
        pane.launch_previews.insert(
            (Runtime::Codex, true),
            RuntimeCommandPreview {
                lines: vec![RuntimeCommandLine {
                    command: "codex --ask-for-approval never --sandbox danger-full-access".into(),
                    note: RuntimeCommandNote::AlwaysInMissions,
                    option: None,
                }],
                custom_flags_warning: None,
            },
        );
        cx.notify();
    })
    .unwrap();
    window.run_until_parked();

    let copy = window.debug_bounds("AGENT_LAUNCH_COPY").unwrap();
    window.simulate_click(copy.center(), gpui::Modifiers::default());
    window.run_until_parked();
    pane.read_with(&window, |pane, cx| {
        assert!(pane.launch_copy.read(cx).is_copied());
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("codex -m <model>\n<first message>")
        );
    })
    .unwrap();

    window
        .executor()
        .advance_clock(Duration::from_millis(1_499));
    window.run_until_parked();
    pane.read_with(&window, |pane, cx| {
        assert!(pane.launch_copy.read(cx).is_copied());
    })
    .unwrap();
    window.executor().advance_clock(Duration::from_millis(1));
    window.run_until_parked();
    pane.read_with(&window, |pane, cx| {
        assert!(!pane.launch_copy.read(cx).is_copied());
    })
    .unwrap();

    window.simulate_click(copy.center(), gpui::Modifiers::default());
    window.run_until_parked();
    let mission_tab = window
        .debug_bounds("AGENT_LAUNCH_TAB_codex_mission")
        .unwrap();
    window.simulate_click(mission_tab.center(), gpui::Modifiers::default());
    window.run_until_parked();
    pane.read_with(&window, |pane, cx| {
        assert!(!pane.launch_copy.read(cx).is_copied());
        assert_eq!(
            pane.launch_copy.read(cx).value(),
            Some("codex --ask-for-approval never --sandbox danger-full-access")
        );
    })
    .unwrap();

    pane.update(&mut window, |pane, win, cx| {
        let focus = pane.launch_copy.read(cx).focus_handle(cx);
        focus.focus(win, cx);
    })
    .unwrap();
    window.run_until_parked();
    window.simulate_keystrokes("space");
    window.run_until_parked();
    pane.read_with(&window, |pane, cx| {
        assert!(pane.launch_copy.read(cx).is_copied());
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("codex --ask-for-approval never --sandbox danger-full-access")
        );
    })
    .unwrap();

    pane.update(&mut window, |pane, _, cx| {
        pane.selected_runtime = Some(Runtime::ClaudeCode);
        pane.sync_launch_copy(true, cx);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("marker".into()));
        cx.notify();
    })
    .unwrap();
    window.run_until_parked();
    let copy = window.debug_bounds("AGENT_LAUNCH_COPY").unwrap();
    window.simulate_click(copy.center(), gpui::Modifiers::default());
    window.run_until_parked();
    pane.read_with(&window, |pane, cx| {
        assert!(!pane.launch_copy.read(cx).is_copied());
        assert_eq!(pane.launch_copy.read(cx).value(), None);
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("marker")
        );
    })
    .unwrap();
}

#[test]
fn antigravity_launch_card_renders_bypass_switch_and_preview_note() {
    use runner_core::protocol::runtime::{
        RuntimeCommandLine, RuntimeCommandNote, RuntimeCommandPreview, RuntimeLaunchOption,
    };

    assert_eq!(
        super::launch::preview_note(&RuntimeCommandNote::AntigravityBypass),
        "Bypass permissions"
    );
    crate::i18n::testing::with_locale(crate::i18n::Locale::SimplifiedChinese, || {
        assert_eq!(
            super::launch::preview_note(&RuntimeCommandNote::AntigravityBypass),
            "跳过权限确认"
        );
    });

    let mut cx = gpui::TestAppContext::single();
    let mut agy = runtime(RuntimeRowState::Detected);
    agy.name = Runtime::Antigravity;
    agy.display_name = "Antigravity CLI".into();
    agy.launch_options.bypass = true;
    let (_temp, pane) = test_pane(vec![agy], false, &mut cx);
    pane.update(&mut cx, |pane, _, cx| {
        pane.selected_runtime = Some(Runtime::Antigravity);
        pane.launch_previews.insert(
            (Runtime::Antigravity, false),
            RuntimeCommandPreview {
                lines: vec![
                    RuntimeCommandLine {
                        command: "agy".into(),
                        note: RuntimeCommandNote::None,
                        option: None,
                    },
                    RuntimeCommandLine {
                        command: "  --dangerously-skip-permissions".into(),
                        note: RuntimeCommandNote::AntigravityBypass,
                        option: Some(RuntimeLaunchOption::Bypass),
                    },
                ],
                custom_flags_warning: None,
            },
        );
        pane.launch_previews.insert(
            (Runtime::Antigravity, true),
            RuntimeCommandPreview {
                lines: vec![
                    RuntimeCommandLine {
                        command: "agy".into(),
                        note: RuntimeCommandNote::None,
                        option: None,
                    },
                    RuntimeCommandLine {
                        command: "  --dangerously-skip-permissions".into(),
                        note: RuntimeCommandNote::AlwaysInMissions,
                        option: None,
                    },
                ],
                custom_flags_warning: None,
            },
        );
        cx.notify();
    })
    .unwrap();
    cx.run_until_parked();
    let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
    assert!(window
        .debug_bounds("AGENT_LAUNCH_SWITCH_antigravity_Bypass")
        .is_some());
    assert!(window
        .debug_bounds("AGENT_LAUNCH_SWITCH_antigravity_RemotePluginCatalog")
        .is_none());
    assert!(window
        .debug_bounds("AGENT_LAUNCH_SWITCH_antigravity_StatusHooks")
        .is_none());
    pane.read_with(&window, |pane, cx| {
        assert_eq!(
            pane.launch_copy.read(cx).value(),
            Some("agy\n  --dangerously-skip-permissions")
        );
    })
    .unwrap();
}

#[test]
fn custom_flags_renders_for_agents_with_and_without_switches_and_matches_copy() {
    use runner_core::protocol::runtime::{
        RuntimeCommandLine, RuntimeCommandNote, RuntimeCommandPreview,
    };

    for runtime_name in [Runtime::Codex, Runtime::Cursor, Runtime::Trae] {
        let mut cx = gpui::TestAppContext::single();
        let mut row = runtime(RuntimeRowState::Detected);
        row.name = runtime_name;
        let (_temp, pane) = test_pane(vec![row], false, &mut cx);
        pane.update(&mut cx, |pane, _, cx| {
            pane.selected_runtime = Some(runtime_name);
            pane.launch_previews.insert(
                (runtime_name, false),
                RuntimeCommandPreview {
                    lines: vec![
                        RuntimeCommandLine {
                            command: runtime_name.key().into(),
                            note: RuntimeCommandNote::None,
                            option: None,
                        },
                        RuntimeCommandLine {
                            command: "  --search".into(),
                            note: RuntimeCommandNote::CustomFlags,
                            option: None,
                        },
                    ],
                    custom_flags_warning: None,
                },
            );
            pane.sync_launch_copy(false, cx);
            cx.notify();
        })
        .unwrap();
        cx.run_until_parked();

        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        let row_bounds = window
            .debug_bounds(Box::leak(
                format!("AGENT_CUSTOM_FLAGS_ROW_{runtime_name}").into_boxed_str(),
            ))
            .expect("custom flags row should render");
        assert!(window
            .debug_bounds(Box::leak(
                format!("AGENT_CUSTOM_FLAGS_{runtime_name}").into_boxed_str(),
            ))
            .is_some());
        assert!(window
            .debug_bounds(Box::leak(
                format!("AGENT_CUSTOM_FLAGS_MESSAGE_{runtime_name}").into_boxed_str(),
            ))
            .is_none());
        if runtime_name == Runtime::Codex {
            let switch_bounds = window
                .debug_bounds("AGENT_LAUNCH_SWITCH_codex_StatusHooks")
                .unwrap();
            assert!(
                row_bounds.top() > switch_bounds.bottom(),
                "custom flags should sit below the switches"
            );
        }
        let preview_bounds = window
            .debug_bounds(Box::leak(
                format!("AGENT_LAUNCH_COMMAND_{runtime_name}").into_boxed_str(),
            ))
            .unwrap();
        assert!(
            row_bounds.bottom() <= preview_bounds.top() + px(1.),
            "custom flags should sit above Command preview"
        );
        pane.read_with(&window, |pane, cx| {
            assert_eq!(
                pane.launch_copy.read(cx).value(),
                Some(format!("{}\n  --search", runtime_name.key()).as_str())
            );
        })
        .unwrap();
    }

    for (locale, expected_note, expected_caption, expected_description) in [
        (
            crate::i18n::Locale::English,
            "Custom flags",
            "Missions always start in Bypass. A role's own arguments come after custom flags. Changes take effect when a session next starts.",
            "Added to every Claude Code launch, including role sessions, resumes and forks. Quote a value that contains spaces, as in a shell.",
        ),
        (
            crate::i18n::Locale::SimplifiedChinese,
            "自定义参数",
            "任务始终以 Bypass 模式启动。角色自身的参数位于自定义参数之后。更改在会话下次启动时生效。",
            "添加到每次启动 Claude Code 的命令中，包括角色会话、恢复和分叉。包含空格的参数值请像在 shell 中一样加引号。",
        ),
    ] {
        crate::i18n::testing::with_locale(locale, || {
            assert_eq!(
                super::launch::preview_note(&RuntimeCommandNote::CustomFlags).as_ref(),
                expected_note
            );
            assert_eq!(crate::i18n::text("ui.launch_caption"), expected_caption);
            let description = super::launch::custom_flags_description("Claude Code");
            assert!(!description.contains("{agent}"), "{description}");
            assert_eq!(description, expected_description);
        });
    }
}

#[test]
fn custom_flags_placeholders_are_accepted_for_every_agent() {
    let mut cx = gpui::TestAppContext::single();
    let temp = tempfile::tempdir().unwrap();
    let store = test_store(temp.path(), &mut cx);
    let client = store.read_with(&cx, |store, _| store.client.clone());
    for entry in runner_core::protocol::runtime_metadata::runtime_list() {
        let placeholder = custom_flags_placeholder(entry.name);
        let result = client
            .runtime_set_custom_flags(entry.name, placeholder)
            .unwrap();
        assert!(
            result.is_ok(),
            "placeholder for {} was refused: {result:?}",
            entry.name
        );
    }
}

#[test]
fn custom_flags_enter_blur_refusal_and_escape_flow() {
    use gpui::Focusable as _;
    crate::i18n::testing::with_locale(crate::i18n::Locale::English, || {
        let mut cx = gpui::TestAppContext::single();
        let (_temp, pane) = test_pane(vec![runtime(RuntimeRowState::Detected)], false, &mut cx);
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        window.update(|win, _| win.activate_window());
        open_test_page(pane, Runtime::Codex, &mut window);
        window.run_until_parked();

        // Save valid custom flags on Enter and verify preview + Copy update.
        pane.update(&mut window, |pane, win, cx| {
            let field = pane.custom_flags[&Runtime::Codex].clone();
            field.update(cx, |field, field_cx| {
                field.focus_handle().focus(win, field_cx);
                field.reset("--search", field_cx);
            });
        })
        .unwrap();
        window.simulate_keystrokes("enter");
        window.run_until_parked();
        pane.read_with(&window, |pane, cx| {
            assert_eq!(pane.saved_custom_flags(Runtime::Codex), "--search");
            assert_eq!(
                pane.custom_flags[&Runtime::Codex].read(cx).text(),
                "--search"
            );
            assert!(!pane.custom_flags_validation.contains_key(&Runtime::Codex));
            let preview = pane
                .launch_previews
                .get(&(Runtime::Codex, false))
                .expect("chat preview should refresh");
            assert!(preview.lines.iter().any(|line| {
                line.command == "  --search"
                    && line.note == runner_core::protocol::runtime::RuntimeCommandNote::CustomFlags
            }));
            assert!(pane
                .launch_copy
                .read(cx)
                .value()
                .is_some_and(|text| text.contains("  --search")));
        })
        .unwrap();

        // Refused input on blur keeps draft + red error, leaving saved flags & preview intact.
        pane.update(&mut window, |pane, win, cx| {
            let field = pane.custom_flags[&Runtime::Codex].clone();
            field.update(cx, |field, field_cx| {
                field.focus_handle().focus(win, field_cx);
                field.reset("--model gpt-5", field_cx);
            });
        })
        .unwrap();
        window.run_until_parked();
        pane.update(&mut window, |pane, win, cx| {
            pane.launch_copy.read(cx).focus_handle(cx).focus(win, cx);
        })
        .unwrap();
        window.run_until_parked();
        pane.read_with(&window, |pane, cx| {
            assert_eq!(pane.saved_custom_flags(Runtime::Codex), "--search");
            assert_eq!(
                pane.custom_flags[&Runtime::Codex].read(cx).text(),
                "--model gpt-5"
            );
            let validation = pane
                .custom_flags_validation
                .get(&Runtime::Codex)
                .expect("error validation should be set");
            assert!(validation.is_error());
            assert_eq!(
                validation.message().as_deref(),
                Some("Runner already sets --model.")
            );
            let preview = &pane.launch_previews[&(Runtime::Codex, false)];
            assert!(preview
                .lines
                .iter()
                .any(|line| line.command == "  --search"));
            assert!(!preview
                .lines
                .iter()
                .any(|line| line.command.contains("gpt-5")));
        })
        .unwrap();
        assert!(window
            .debug_bounds("AGENT_CUSTOM_FLAGS_MESSAGE_codex")
            .is_some());

        // Escape restores saved flags and clears the refused draft & error.
        pane.update(&mut window, |pane, win, cx| {
            let field = pane.custom_flags[&Runtime::Codex].clone();
            field.update(cx, |field, field_cx| {
                field.focus_handle().focus(win, field_cx);
            });
        })
        .unwrap();
        window.simulate_keystrokes("escape");
        window.run_until_parked();
        pane.read_with(&window, |pane, cx| {
            assert_eq!(
                pane.custom_flags[&Runtime::Codex].read(cx).text(),
                "--search"
            );
            assert!(!pane.custom_flags_validation.contains_key(&Runtime::Codex));
        })
        .unwrap();
    });
}

#[test]
fn custom_flags_reopen_discards_refused_draft_while_staying_keeps_it() {
    crate::i18n::testing::with_locale(crate::i18n::Locale::English, || {
        let mut cx = gpui::TestAppContext::single();
        let codex = runtime(RuntimeRowState::Detected);
        let mut claude = runtime(RuntimeRowState::Detected);
        claude.name = Runtime::ClaudeCode;
        claude.display_name = "Claude Code".into();
        let (_temp, pane) = test_pane(vec![codex, claude], false, &mut cx);
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        window.update(|win, _| win.activate_window());
        open_test_page(pane, Runtime::Codex, &mut window);
        window.run_until_parked();

        // Save `--search`.
        pane.update(&mut window, |pane, win, cx| {
            let field = pane.custom_flags[&Runtime::Codex].clone();
            field.update(cx, |field, field_cx| {
                field.focus_handle().focus(win, field_cx);
                field.reset("--search", field_cx);
            });
        })
        .unwrap();
        window.simulate_keystrokes("enter");
        window.run_until_parked();

        // Submit refused `--model gpt-x`; staying on the page + background refresh keeps draft & error.
        pane.update(&mut window, |pane, win, cx| {
            let field = pane.custom_flags[&Runtime::Codex].clone();
            field.update(cx, |field, field_cx| {
                field.focus_handle().focus(win, field_cx);
                field.reset("--model gpt-x", field_cx);
            });
        })
        .unwrap();
        window.simulate_keystrokes("enter");
        window.run_until_parked();
        pane.update(&mut window, |pane, _, cx| {
            pane.refresh(cx);
        })
        .unwrap();
        window.run_until_parked();
        pane.read_with(&window, |pane, cx| {
            assert_eq!(pane.saved_custom_flags(Runtime::Codex), "--search");
            assert_eq!(
                pane.custom_flags[&Runtime::Codex].read(cx).text(),
                "--model gpt-x"
            );
            assert!(pane
                .custom_flags_validation
                .get(&Runtime::Codex)
                .is_some_and(|v| v.is_error()));
        })
        .unwrap();

        // Navigating back to the agent list and reopening Codex restores saved `--search` with no error.
        pane.update(&mut window, |pane, _, cx| {
            pane.show_list(cx);
        })
        .unwrap();
        window.run_until_parked();
        open_test_page(pane, Runtime::Codex, &mut window);
        window.run_until_parked();
        pane.read_with(&window, |pane, cx| {
            assert_eq!(
                pane.custom_flags[&Runtime::Codex].read(cx).text(),
                "--search"
            );
            assert!(!pane.custom_flags_validation.contains_key(&Runtime::Codex));
        })
        .unwrap();

        // Refusing again and switching directly to another agent then back also restores saved flags.
        pane.update(&mut window, |pane, win, cx| {
            let field = pane.custom_flags[&Runtime::Codex].clone();
            field.update(cx, |field, field_cx| {
                field.focus_handle().focus(win, field_cx);
                field.reset("--model gpt-x", field_cx);
            });
        })
        .unwrap();
        window.simulate_keystrokes("enter");
        window.run_until_parked();
        pane.update(&mut window, |pane, _, cx| {
            pane.open_runtime(Runtime::ClaudeCode, cx);
        })
        .unwrap();
        window.run_until_parked();
        pane.update(&mut window, |pane, _, cx| {
            pane.open_runtime(Runtime::Codex, cx);
        })
        .unwrap();
        window.run_until_parked();
        pane.read_with(&window, |pane, cx| {
            assert_eq!(
                pane.custom_flags[&Runtime::Codex].read(cx).text(),
                "--search"
            );
            assert!(!pane.custom_flags_validation.contains_key(&Runtime::Codex));
        })
        .unwrap();
    });
}

#[test]
fn custom_flags_breadcrumb_click_saves_focused_valid_edit() {
    use gpui::Focusable as _;
    use runner_core::protocol::runtime::RuntimeCommandNote;

    crate::i18n::testing::with_locale(crate::i18n::Locale::English, || {
        let mut cx = gpui::TestAppContext::single();
        let codex = runtime(RuntimeRowState::Detected);
        let (_temp, pane) = test_pane(vec![codex], false, &mut cx);
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        window.update(|win, _| win.activate_window());
        open_test_page(pane, Runtime::Codex, &mut window);
        window.run_until_parked();

        // Save initial `--search`.
        pane.update(&mut window, |pane, win, cx| {
            let field = pane.custom_flags[&Runtime::Codex].clone();
            field.update(cx, |field, field_cx| {
                field.focus_handle().focus(win, field_cx);
                field.reset("--search", field_cx);
            });
        })
        .unwrap();
        window.simulate_keystrokes("enter");
        window.run_until_parked();
        pane.update(&mut window, |pane, win, cx| {
            pane.launch_copy.read(cx).focus_handle(cx).focus(win, cx);
        })
        .unwrap();
        window.run_until_parked();

        // Focus and enter a valid dirty edit without pressing Enter or blurring first.
        let edited = r#"--add-dir "/tmp/extra dir""#;
        pane.update(&mut window, |pane, win, cx| {
            let field = pane.custom_flags[&Runtime::Codex].clone();
            field.update(cx, |field, field_cx| {
                field.focus_handle().focus(win, field_cx);
                field.reset(edited, field_cx);
            });
        })
        .unwrap();
        window.run_until_parked();
        pane.read_with(&window, |pane, _| {
            assert!(pane.custom_flags_focused.contains(&Runtime::Codex));
        })
        .unwrap();

        // Real click on the Agents breadcrumb leaves the page and commits the valid edit.
        let breadcrumb = window
            .debug_bounds("AGENTS_BREADCRUMB")
            .expect("breadcrumb should render");
        window.simulate_click(breadcrumb.center(), gpui::Modifiers::default());
        window.run_until_parked();
        pane.read_with(&window, |pane, _| {
            assert_eq!(pane.selected_runtime, None);
            assert_eq!(pane.saved_custom_flags(Runtime::Codex), edited);
        })
        .unwrap();

        // Reopen Codex and confirm both saved value and command preview reflect the committed edit.
        open_test_page(pane, Runtime::Codex, &mut window);
        window.run_until_parked();
        pane.read_with(&window, |pane, cx| {
            assert_eq!(pane.custom_flags[&Runtime::Codex].read(cx).text(), edited);
            assert!(!pane.custom_flags_validation.contains_key(&Runtime::Codex));
            let preview = pane
                .launch_previews
                .get(&(Runtime::Codex, false))
                .expect("chat preview should be loaded");
            assert!(preview.lines.iter().any(|line| {
                line.note == RuntimeCommandNote::CustomFlags
                    && line.command.contains("--add-dir")
                    && line.command.contains("/tmp/extra dir")
            }));
            assert!(pane
                .launch_copy
                .read(cx)
                .value()
                .is_some_and(|text| text.contains("--add-dir")));
        })
        .unwrap();

        // Unsubmitted invalid edit on breadcrumb click is refused and discarded on reopen.
        pane.update(&mut window, |pane, win, cx| {
            let field = pane.custom_flags[&Runtime::Codex].clone();
            field.update(cx, |field, field_cx| {
                field.focus_handle().focus(win, field_cx);
                field.reset("--model gpt-x", field_cx);
            });
        })
        .unwrap();
        window.run_until_parked();
        let breadcrumb = window
            .debug_bounds("AGENTS_BREADCRUMB")
            .expect("breadcrumb should render");
        window.simulate_click(breadcrumb.center(), gpui::Modifiers::default());
        window.run_until_parked();
        open_test_page(pane, Runtime::Codex, &mut window);
        window.run_until_parked();
        pane.read_with(&window, |pane, cx| {
            assert_eq!(pane.saved_custom_flags(Runtime::Codex), edited);
            assert_eq!(pane.custom_flags[&Runtime::Codex].read(cx).text(), edited);
            assert!(!pane.custom_flags_validation.contains_key(&Runtime::Codex));
        })
        .unwrap();
    });
}

#[cfg(unix)]
#[test]
fn custom_flags_warning_saves_keeps_normal_border_and_recomputes_on_reload() {
    use std::os::unix::fs::PermissionsExt as _;

    crate::i18n::testing::with_locale(crate::i18n::Locale::English, || {
        let mut cx = gpui::TestAppContext::single();
        let (temp, pane) = test_pane(vec![runtime(RuntimeRowState::Detected)], false, &mut cx);
        let script = temp.path().join("fake-codex.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"codex-cli 0.50.0\"; exit 0; fi\necho \"Usage: codex [OPTIONS]\"\necho \"  --search  Enable search\"\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        let client = pane
            .read_with(&cx, |pane, cx| pane.app_store.read(cx).client.clone())
            .unwrap();
        let status = client
            .runtime_set_override(Runtime::Codex, &script.to_string_lossy())
            .unwrap()
            .unwrap();
        pane.update(&mut cx, |pane, _, cx| {
            pane.apply_status(status, cx);
        })
        .unwrap();

        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        window.update(|win, _| win.activate_window());
        open_test_page(pane, Runtime::Codex, &mut window);
        window.run_until_parked();

        pane.update(&mut window, |pane, win, cx| {
            let field = pane.custom_flags[&Runtime::Codex].clone();
            field.update(cx, |field, field_cx| {
                field.focus_handle().focus(win, field_cx);
                field.reset("--bogus", field_cx);
            });
        })
        .unwrap();
        window.simulate_keystrokes("enter");
        window.run_until_parked();

        pane.read_with(&window, |pane, cx| {
            assert_eq!(pane.saved_custom_flags(Runtime::Codex), "--bogus");
            let validation = pane
                .custom_flags_validation
                .get(&Runtime::Codex)
                .expect("amber warning should show");
            assert!(validation.is_warning());
            assert!(!validation.is_error());
            assert_eq!(
                validation.message().as_deref(),
                Some("codex --help doesn't list --bogus.")
            );
            assert!(!pane.custom_flags[&Runtime::Codex]
                .read(cx)
                .validation()
                .is_error());
            assert!(pane.launch_previews[&(Runtime::Codex, false)]
                .lines
                .iter()
                .any(|line| line.command == "  --bogus"));
        })
        .unwrap();
        assert!(window
            .debug_bounds("AGENT_CUSTOM_FLAGS_MESSAGE_codex")
            .is_some());

        // Reloading after the agent's `--help` gains `--bogus` recomputes and clears the warning.
        std::fs::write(
            &script,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"codex-cli 0.51.0\"; exit 0; fi\necho \"Usage: codex [OPTIONS]\"\necho \"  --search  Enable search\"\necho \"  --bogus   Newly added flag\"\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        let _ = client
            .runtime_set_override(Runtime::Codex, &script.to_string_lossy())
            .unwrap()
            .unwrap();
        pane.update(&mut window, |pane, _, cx| {
            pane.refresh(cx);
        })
        .unwrap();
        window.run_until_parked();

        pane.read_with(&window, |pane, _| {
            assert_eq!(pane.saved_custom_flags(Runtime::Codex), "--bogus");
            assert!(!pane.custom_flags_validation.contains_key(&Runtime::Codex));
        })
        .unwrap();
    });
}
