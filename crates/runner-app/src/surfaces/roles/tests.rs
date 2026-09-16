use super::logic::resolve_slot_runtime_layers;
use super::logic::runtime_default_effort_label;
use super::logic::runtime_model_placeholder;
use super::logic::validate_role_handle;
use super::*;
use runner_backend::model::Runtime;

#[test]
fn legacy_slot_pins_reach_validation_as_raw_names() {
    for name in ["qoder", "Runtime-Needle"] {
        let layers = resolve_slot_runtime_layers("codex", Some(name), None, None);
        assert!(layers.runtime_pinned);
        assert_eq!(layers.runtime, name);
        let raw_override = layers.runtime_pinned.then_some(layers.runtime.as_str());
        let error = runner_backend::ops::slot::validate_runtime_override(raw_override).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("unknown runtime '{name}' — valid runtimes: codex, claude-code, copilot, trae")
        );
    }
}

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
        default_enabled: true,
        available: true,
        default_model: default_model.map(str::to_owned),
        default_effort: default_effort.map(str::to_owned),
        models: Vec::new(),
        efforts: Vec::new(),
    }
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
fn slot_runtime_layers_leave_blank_overrides_to_inherit_role_defaults() {
    assert_eq!(
        resolve_slot_runtime_layers("codex", None, None, None),
        RuntimeLayerResolution {
            runtime: "codex".into(),
            runtime_pinned: false,
            model: None,
            effort: None,
        }
    );
}

#[test]
fn same_runtime_pin_keeps_blank_model_and_effort_overrides() {
    assert_eq!(
        resolve_slot_runtime_layers("codex", Some("codex"), None, None),
        RuntimeLayerResolution {
            runtime: "codex".into(),
            runtime_pinned: true,
            model: None,
            effort: None,
        }
    );
}

#[test]
fn different_runtime_uses_runtime_defaults_unless_overridden() {
    assert_eq!(
        resolve_slot_runtime_layers("codex", Some("claude-code"), None, None),
        RuntimeLayerResolution {
            runtime: "claude-code".into(),
            runtime_pinned: true,
            model: None,
            effort: None,
        }
    );
    assert_eq!(
        resolve_slot_runtime_layers("codex", Some("claude-code"), Some("opus"), Some("max"),),
        RuntimeLayerResolution {
            runtime: "claude-code".into(),
            runtime_pinned: true,
            model: Some("opus".into()),
            effort: Some("max".into()),
        }
    );
}

#[test]
fn trae_does_not_offer_a_mode_it_cannot_write() {
    use super::logic::{permission_mode_description, permission_modes};
    use runner_backend::router::runtime::PermissionMode;

    // TRAE CLI has no auto-approve middle ground, so Auto would write
    // nothing and read back as Default (#599).
    assert_eq!(
        permission_modes("trae"),
        &[PermissionMode::Default, PermissionMode::Bypass]
    );
    assert!(!permission_modes("trae").contains(&PermissionMode::Auto));
    assert!(permission_mode_description("trae", PermissionMode::Auto).is_empty());

    // Codex keeps its own Auto — it maps to a real flag pair.
    assert!(permission_modes("codex").contains(&PermissionMode::Auto));
    assert!(permission_modes("claude-code").contains(&PermissionMode::Auto));

    // Every offered mode describes itself.
    for runtime in ["claude-code", "codex", "trae", "copilot"] {
        for mode in permission_modes(runtime) {
            assert!(
                !permission_mode_description(runtime, *mode).is_empty(),
                "{runtime} {mode:?} has no description"
            );
        }
    }
}

#[test]
fn copilot_offers_only_the_three_supported_permission_modes_with_the_approved_copy() {
    use super::logic::{permission_mode_description, permission_modes};
    use runner_backend::router::runtime::PermissionMode;
    assert_eq!(
        permission_modes("copilot"),
        [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Bypass
        ]
    );
    assert!(permission_mode_description("copilot", PermissionMode::Auto).is_empty());
    assert_eq!(permission_mode_description("copilot", PermissionMode::Default), "Copilot's own manual mode: read-only tools run, writes and shell commands ask. Governed by defaultPermissionMode in ~/.copilot/settings.json.");
    assert_eq!(permission_mode_description("copilot", PermissionMode::AcceptEdits), "File creates and edits run without asking; shell commands, URLs and paths outside the cwd still prompt.");
    assert_eq!(permission_mode_description("copilot", PermissionMode::Bypass), "Every tool, path and URL is allowed. Same flag for the app-wide mission permission mode; chats never carry it (#596).");
}

#[test]
fn role_detail_columns_stay_inside_the_centered_container() {
    use crate::surfaces::AppRoute;
    use crate::theme_snapshot::ThemeGuard;
    use crate::*;
    use chrono::Utc;
    use gpui::{px, size, TestAppContext, VisualTestContext};
    use runner_backend::model::Role;
    use runner_backend::{db, event_bus, events, mcp, router, session, shell_path, windows};
    use std::sync::{Arc, Mutex, RwLock};

    let _theme = ThemeGuard::new();
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
    let now = Utc::now();
    let role = Role {
        id: "01K000DEFAULT000RUNNERREVW01".into(),
        handle: "reviewer".into(),
        display_name: "Reviewer".into(),
        runtime: "codex".into(),
        command: "codex".into(),
        args: ["--ask-for-approval", "on-request", "--sandbox", "workspace-write"]
            .map(str::to_owned)
            .to_vec(),
        working_dir: None,
        system_prompt: Some(
            "You are a reviewer in a two-person peer coding loop. Your job is to read the task, inspect the coder's local working-tree diff, and push back when something is wrong, missing, risky, or out of scope.\n\n"
                .repeat(4),
        ),
        env: Default::default(),
        model: None,
        effort: None,
        created_at: now,
        updated_at: now,
    };
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "role-detail-layout".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = AppRoute::RoleDetail("reviewer".into());
        root.role_surfaces.detail = RoleDetailState {
            handle: "reviewer".into(),
            role: Some(role),
            activity: None,
            crews: Vec::new(),
            loaded: true,
            loading: false,
            error: None,
        };
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    for width in [900., 1100., 1440.] {
        visual.simulate_resize(size(px(width), px(900.)));
        visual.run_until_parked();
        let scroll = visual.debug_bounds("ROLE_DETAIL_SCROLL").unwrap();
        let container = visual.debug_bounds("ROLE_DETAIL_CONTAINER").unwrap();
        let header = visual.debug_bounds("ROLE_DETAIL_HEADER").unwrap();
        let body = visual.debug_bounds("ROLE_DETAIL_BODY").unwrap();
        let main = visual.debug_bounds("ROLE_DETAIL_MAIN").unwrap();
        let aside = visual.debug_bounds("ROLE_DETAIL_ASIDE").unwrap();
        let left_slack = container.left() - scroll.left();
        let right_slack = scroll.right() - container.right();
        assert!(
            (left_slack - right_slack).abs() <= px(1.),
            "{width}: container is off-centre, slack {left_slack:?} vs {right_slack:?}"
        );
        assert!(
            (body.right() - header.right()).abs() <= px(1.),
            "{width}: body row {:?} does not end with the header {:?}",
            body.right(),
            header.right()
        );
        assert!(
            aside.right() <= header.right() + px(1.),
            "{width}: aside ends at {:?}, past the header's {:?}",
            aside.right(),
            header.right()
        );
        assert!(
            main.right() + px(1.) < aside.left(),
            "{width}: columns overlap: main ends {:?}, aside starts {:?}",
            main.right(),
            aside.left()
        );
    }
    assert!(
        visual.debug_bounds("ENTITY_SIDEBAR_TOGGLE").is_none(),
        "the shell must not add an open-sidebar cluster while the sidebar is open"
    );
    for route in [
        AppRoute::RoleDetail("reviewer".into()),
        AppRoute::Roles,
        AppRoute::Crews,
        AppRoute::CrewEditor("crew".into()),
    ] {
        host.update(&mut visual, |root, _, cx| {
            root.set_sidebar_collapsed(true, false, cx);
            root.route = route.clone();
            cx.notify();
        })
        .unwrap();
        visual.run_until_parked();
        let toggle = visual
            .debug_bounds("ENTITY_SIDEBAR_TOGGLE")
            .unwrap_or_else(|| {
                panic!("{route:?}: a collapsed sidebar leaves no open-sidebar cluster")
            });
        let padding = host
            .update(&mut visual, |root, window, cx| {
                root.workspace_titlebar_padding(window, cx)
            })
            .unwrap();
        let column = visual.debug_bounds("APP_CONTENT_COLUMN").unwrap();
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
