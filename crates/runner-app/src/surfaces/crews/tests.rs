use super::logic::crew_name_refresh;
use super::logic::crew_name_state;
use super::logic::move_item;
use super::logic::slot_command_summary;
use super::logic::suggest_slot_handle;
use super::logic::validate_slot_handle;
use super::logic::CrewNameRefresh;
use super::*;
use chrono::Utc;
use runner_backend::model::{Role, Slot};

fn slot_with_role(
    runtime_override: Option<&str>,
    model_override: Option<&str>,
    effort_override: Option<&str>,
) -> SlotWithRole {
    let now = Utc::now();
    SlotWithRole {
        slot: Slot {
            id: "slot".into(),
            crew_id: "crew".into(),
            role_id: "role".into(),
            slot_handle: "coder".into(),
            position: 0,
            lead: true,
            runtime_override: runtime_override.map(str::to_owned),
            model_override: model_override.map(str::to_owned),
            effort_override: effort_override.map(str::to_owned),
            added_at: now,
        },
        role: Role {
            id: "role".into(),
            handle: "coder".into(),
            display_name: "Coder".into(),
            runtime: "codex".into(),
            command: "codex".into(),
            args: vec!["--quiet".into()],
            working_dir: None,
            system_prompt: None,
            env: Default::default(),
            model: None,
            effort: None,
            created_at: now,
            updated_at: now,
        },
    }
}

#[test]
fn slot_handle_validation_matches_the_shipped_contract() {
    for valid in ["", "a", "0", "coder-2", "coder_2", &"a".repeat(32)] {
        assert_eq!(validate_slot_handle(valid), None, "{valid}");
    }
    for invalid in ["Coder", "-coder", "_coder", "coder!", &"a".repeat(33)] {
        assert!(validate_slot_handle(invalid).is_some(), "{invalid}");
    }
}

#[test]
fn slot_handle_suggestions_take_the_first_available_suffix() {
    let taken = HashSet::from([
        "coder".to_owned(),
        "coder-2".to_owned(),
        "coder-3".to_owned(),
    ]);

    assert_eq!(suggest_slot_handle("reviewer", &taken), "reviewer");
    assert_eq!(suggest_slot_handle("coder", &taken), "coder-4");
}

#[test]
fn crew_name_state_tracks_saved_reverted_and_empty_edits() {
    assert_eq!(crew_name_state("Crew", "Crew"), (false, false, false));
    assert_eq!(crew_name_state("New crew", "Crew"), (true, true, false));
    assert_eq!(crew_name_state(" Crew ", "Crew"), (true, false, false));
    assert_eq!(crew_name_state("  ", "Crew"), (true, false, true));
}

#[test]
fn crew_name_refresh_preserves_live_edits_without_orphaning_the_field() {
    assert_eq!(
        crew_name_refresh("Draft", "Crew", true),
        CrewNameRefresh::Preserve
    );
    assert_eq!(
        crew_name_refresh("Old", "New", false),
        CrewNameRefresh::Reset
    );
    assert_eq!(
        crew_name_refresh("Saved", "Saved", true),
        CrewNameRefresh::MarkClean
    );
}

#[test]
fn slot_command_summary_applies_runtime_and_model_effort_layers() {
    assert_eq!(
        slot_command_summary(&slot_with_role(None, None, None)),
        "codex --quiet"
    );
    assert_eq!(
        slot_command_summary(&slot_with_role(Some("codex"), Some("gpt-5"), Some("high"))),
        "codex --quiet (model gpt-5 · effort high)"
    );
    assert_eq!(
        slot_command_summary(&slot_with_role(Some("claude-code"), None, None)),
        "claude (runtime defaults)"
    );
    assert_eq!(
        slot_command_summary(&slot_with_role(
            Some("claude-code"),
            Some("opus"),
            Some("max")
        )),
        "claude (runtime defaults · model opus · effort max)"
    );
}

#[test]
fn move_item_reorders_slots_in_both_directions() {
    assert_eq!(move_item(&["a", "b", "c", "d"], 1, 3), ["a", "c", "d", "b"]);
    assert_eq!(move_item(&["a", "b", "c", "d"], 3, 1), ["a", "d", "b", "c"]);
}

#[test]
fn move_item_ignores_same_or_invalid_positions() {
    assert_eq!(move_item(&[1, 2, 3], 1, 1), [1, 2, 3]);
    assert_eq!(move_item(&[1, 2, 3], 8, 1), [1, 2, 3]);
    assert_eq!(move_item(&[1, 2, 3], 1, 8), [1, 2, 3]);
}

#[test]
fn crew_editor_sections_stay_inside_the_centered_container() {
    use crate::surfaces::AppRoute;
    use crate::theme_snapshot::ThemeGuard;
    use crate::*;
    use gpui::{px, size, TestAppContext, VisualTestContext};
    use runner_backend::model::Crew;
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
    let now = Utc::now();
    let crew = Crew {
        id: "crew".into(),
        name: "Peer coding crew".into(),
        purpose: Some("Run a focused coder/reviewer loop for a single implementation task.".into()),
        goal: Some(
            "Definition of done = the requested task is implemented and reviewed clean.".repeat(3),
        ),
        system_prompt_addendum: Some("This crew works as a two-person coding loop. ".repeat(12)),
        created_at: now,
        updated_at: now,
    };
    let host = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "crew-editor-layout".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = AppRoute::CrewEditor("crew".into());
        let name = cx.new(|cx| TextField::new(cx.focus_handle(), "Peer coding crew", "", false));
        root.crew_surfaces.editor = CrewEditorState {
            crew_id: "crew".into(),
            crew: Some(crew),
            slots: vec![slot_with_role(None, None, None)],
            loaded: true,
            name: Some(name),
            original_name: "Peer coding crew".into(),
            ..Default::default()
        };
        root
    });
    let mut visual = VisualTestContext::from_window(host.into(), &cx);
    for width in [900., 1100., 1440.] {
        visual.simulate_resize(size(px(width), px(900.)));
        visual.run_until_parked();
        let scroll = visual.debug_bounds("CREW_EDITOR_SCROLL").unwrap();
        let container = visual.debug_bounds("CREW_EDITOR_CONTAINER").unwrap();
        let sections = visual.debug_bounds("CREW_EDITOR_SECTIONS").unwrap();
        let left_slack = container.left() - scroll.left();
        let right_slack = scroll.right() - container.right();
        assert!(
            (left_slack - right_slack).abs() <= px(1.),
            "{width}: container is off-centre, slack {left_slack:?} vs {right_slack:?}"
        );
        assert!(
            sections.right() <= container.right() - px(31.),
            "{width}: sections end at {:?}, past the container's padded edge {:?}",
            sections.right(),
            container.right()
        );
    }
}
