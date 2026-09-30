use super::logic::crew_missions;
use super::logic::crew_summary;
use super::logic::crew_table_columns;
use super::logic::last_mission_label;
use super::logic::mission_duration;
use super::logic::missions_footer;
use super::logic::missions_header;
use super::logic::move_item;
use super::logic::picture_cells;
use super::logic::runtime_counts;
use super::logic::short_date;
use super::logic::slot_command_summary;
use super::logic::slot_runtime_options;
use super::logic::slot_setup;
use super::logic::suggest_slot_handle;
use super::logic::validate_slot_handle;
use super::logic::CrewColumn;
use super::logic::PictureCell;
use super::*;
use crate::surfaces::profile_page::PROFILE_COLUMN_WIDTH;
use chrono::{TimeZone, Utc};
use runner_backend::model::{CodexSpeed, Mission, MissionStatus, Role, Runtime, Slot};
use runner_backend::ops::crew::CrewMemberPreview;
use runner_backend::ops::mission::MissionSummary;

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
            codex_speed_override: None,
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
            codex_speed: None,
            created_at: now,
            updated_at: now,
        },
    }
}

fn mission(
    id: &str,
    crew_id: &str,
    status: MissionStatus,
    started: chrono::DateTime<Utc>,
    minutes: Option<i64>,
) -> Mission {
    Mission {
        id: id.into(),
        crew_id: crew_id.into(),
        project_id: None,
        title: format!("Mission {id}"),
        status,
        goal_override: None,
        cwd: None,
        started_at: started,
        stopped_at: minutes.map(|minutes| started + chrono::Duration::minutes(minutes)),
        pinned_at: None,
        archived_at: None,
    }
}

fn summary(mission: Mission) -> MissionSummary {
    MissionSummary {
        session_statuses: Vec::new(),
        mission,
        crew_name: "crew".into(),
        pending_ask_count: 0,
        any_session_live: false,
        all_sessions_live: false,
        activity: None,
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
fn the_crew_picture_lays_out_one_to_five_slots() {
    let tile = 100.;
    let gap = 5.;
    let half = 47.5;
    let cell = |left: f32, top: f32, size: f32| PictureCell { left, top, size };
    assert!(picture_cells(0, tile).is_empty(), "nothing for no slots");
    assert_eq!(picture_cells(1, tile), [cell(0., 0., tile)]);
    let middle = (tile - half) / 2.;
    assert_eq!(
        picture_cells(2, tile),
        [cell(0., middle, half), cell(half + gap, middle, half)],
        "two sit side by side, centred vertically"
    );
    assert_eq!(
        picture_cells(3, tile),
        [
            cell(0., 0., half),
            cell(half + gap, 0., half),
            cell(middle, half + gap, half),
        ],
        "three are two over one, the third centred"
    );
    let grid = [
        cell(0., 0., half),
        cell(half + gap, 0., half),
        cell(0., half + gap, half),
        cell(half + gap, half + gap, half),
    ];
    assert_eq!(picture_cells(4, tile), grid);
    assert_eq!(
        picture_cells(5, tile),
        grid,
        "five show the first four in a grid"
    );
    for cells in (1..=5).map(|slots| picture_cells(slots, 25.)) {
        for cell in cells {
            assert!(cell.left + cell.size <= 25. + f32::EPSILON);
            assert!(cell.top + cell.size <= 25. + f32::EPSILON);
        }
    }
}

#[test]
fn slot_setup_marks_what_the_slot_overrides() {
    let mut slot = slot_with_role(None, None, None);
    slot.role.model = Some("opus[1m]".into());
    slot.role.effort = Some("xhigh".into());
    let setup = slot_setup(&slot);
    assert_eq!(setup.runtime, "codex");
    assert_eq!(setup.model.as_deref(), Some("opus[1m]"));
    assert_eq!(setup.effort.as_deref(), Some("xhigh"));
    assert!(!setup.overrides_any(), "inherited values are not overrides");

    slot.role.codex_speed = Some(CodexSpeed::Fast);
    let setup = slot_setup(&slot);
    assert_eq!(setup.speed, Some(CodexSpeed::Fast));
    assert!(!setup.speed_overridden);
    slot.slot.codex_speed_override = Some(CodexSpeed::Standard);
    let setup = slot_setup(&slot);
    assert_eq!(setup.speed, Some(CodexSpeed::Standard));
    assert!(setup.speed_overridden && setup.overrides_any());

    slot.slot.effort_override = Some("high".into());
    let setup = slot_setup(&slot);
    assert_eq!(setup.effort.as_deref(), Some("high"));
    assert!(setup.effort_overridden && !setup.model_overridden);

    let mut other = slot_with_role(Some("claude-code"), None, None);
    other.role.model = Some("gpt-5".into());
    let setup = slot_setup(&other);
    assert!(setup.runtime_overridden);
    assert_eq!(
        (setup.model, setup.effort),
        (None, None),
        "another runtime starts from its own defaults, not the role's"
    );

    let mut pinned = slot_with_role(Some("codex"), None, None);
    pinned.role.model = Some("opus[1m]".into());
    let setup = slot_setup(&pinned);
    assert!(
        setup.runtime_overridden && setup.overrides_any(),
        "a pin to the role's own runtime still pins the slot"
    );
    assert!(setup.own_runtime);
    assert_eq!(
        setup.model.as_deref(),
        Some("opus[1m]"),
        "a pin to the role's runtime keeps the role's model"
    );
}

#[test]
fn the_runtime_select_leads_with_the_roles_default() {
    let options = slot_runtime_options(&[], "codex", "claude-code");
    assert_eq!(options[0].value, "", "the blank choice pins nothing");
    assert_eq!(options[0].label.as_ref(), "Role default (Codex)");
}

#[test]
fn slot_runtime_select_restores_role_and_override_in_selector_order() {
    let page = crew_page_harness("slot-runtime-order");
    let mut runtimes = runner_backend::ops::runtime::runtime_catalog(&page.core)
        .unwrap()
        .into_iter()
        .filter(|entry| !matches!(entry.name, Runtime::Codex | Runtime::Pi))
        .map(|mut entry| {
            entry.available = true;
            entry
        })
        .collect();

    crate::surfaces::roles::logic::ensure_runtime_present(&page.core, &mut runtimes, "codex");
    crate::surfaces::roles::logic::ensure_runtime_present(&page.core, &mut runtimes, "pi");
    let options = slot_runtime_options(&runtimes, "codex", "pi");
    assert_eq!(
        options
            .iter()
            .map(|option| option.value.as_str())
            .collect::<Vec<_>>(),
        [
            "",
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
fn list_cells_count_runtimes_in_slot_order_and_summarize_the_crew() {
    let member = |handle: &str, runtime: &str, lead: bool| CrewMemberPreview {
        slot_handle: handle.into(),
        role_handle: handle.into(),
        runtime: runtime.into(),
        lead,
    };
    assert_eq!(
        runtime_counts(&[
            member("lead", "claude-code", true),
            member("coder", "codex", false),
            member("impl", "claude-code", false),
            member("notes", "pi", false),
        ]),
        [
            ("claude-code".to_owned(), 2),
            ("codex".to_owned(), 1),
            ("pi".to_owned(), 1)
        ]
    );
    assert_eq!(crew_summary(5, Some("lead")), "5 slots · lead @lead");
    assert_eq!(crew_summary(1, Some("coder")), "1 slot · lead @coder");
    assert_eq!(crew_summary(0, None), "No slots yet");
}

#[test]
fn the_last_mission_cell_reads_running_latest_or_a_dash() {
    let now = Utc.with_ymd_and_hms(2026, 9, 26, 12, 0, 0).unwrap();
    let early = Utc.with_ymd_and_hms(2026, 9, 18, 16, 2, 0).unwrap();
    let late = Utc.with_ymd_and_hms(2026, 9, 23, 9, 40, 0).unwrap();
    let done = mission("a", "crew", MissionStatus::Completed, early, Some(30));
    let latest = mission("b", "crew", MissionStatus::Aborted, late, Some(9));
    let running = mission("c", "crew", MissionStatus::Running, early, None);

    assert_eq!(last_mission_label(&[], &now), ("—".into(), false));
    assert_eq!(
        last_mission_label(&[&done, &latest], &now),
        ("Sep 23, 09:40".into(), false)
    );
    assert_eq!(
        last_mission_label(&[&done, &latest, &running], &now),
        ("1 mission running".into(), true)
    );
    let second = mission("d", "crew", MissionStatus::Running, late, None);
    assert_eq!(
        last_mission_label(&[&running, &second], &now).0,
        "2 missions running"
    );
}

#[test]
fn narrow_crew_tables_drop_missions_then_runtimes_then_last_mission() {
    let all = [
        CrewColumn::Runtimes,
        CrewColumn::Missions,
        CrewColumn::LastMission,
    ];
    assert_eq!(crew_table_columns(1040.), all);
    assert_eq!(
        crew_table_columns(720.),
        [CrewColumn::Runtimes, CrewColumn::LastMission]
    );
    assert_eq!(crew_table_columns(560.), [CrewColumn::LastMission]);
    assert!(crew_table_columns(336.).is_empty());
}

#[test]
fn the_missions_card_counts_this_crews_missions_newest_first() {
    let day = |day: u32| Utc.with_ymd_and_hms(2026, 9, day, 10, 0, 0).unwrap();
    let missions = vec![
        summary(mission(
            "old",
            "crew",
            MissionStatus::Completed,
            day(2),
            Some(72),
        )),
        summary(mission(
            "new",
            "crew",
            MissionStatus::Aborted,
            day(9),
            Some(18),
        )),
        summary(mission(
            "other",
            "elsewhere",
            MissionStatus::Running,
            day(5),
            None,
        )),
        summary(mission(
            "mid",
            "crew",
            MissionStatus::Completed,
            day(5),
            Some(123),
        )),
    ];
    let crew = crew_missions(&missions, "crew");
    assert_eq!(
        crew.iter()
            .map(|mission| mission.id.as_str())
            .collect::<Vec<_>>(),
        ["new", "mid", "old"]
    );
    assert_eq!(missions_header(&crew), "3 run · none live");
    assert_eq!(
        missions_header(&crew_missions(&missions, "elsewhere")),
        "1 run · 1 live"
    );
    assert!(crew_missions(&missions, "never-ran").is_empty());

    assert_eq!(mission_duration(crew[0], day(20)), "18m");
    assert_eq!(mission_duration(crew[1], day(20)), "2h 03m");
    assert_eq!(mission_duration(crew[2], day(20)), "1h 12m");
    assert_eq!(short_date(&day(9), &day(20)), "Sep 9");
    let last_year = Utc.with_ymd_and_hms(2025, 12, 30, 9, 0, 0).unwrap();
    assert_eq!(short_date(&last_year, &day(20)), "Dec 30, 2025");

    assert_eq!(
        missions_footer(4, false),
        None,
        "four or fewer need no footer"
    );
    assert_eq!(
        missions_footer(14, false).as_deref(),
        Some("Show all 14 missions")
    );
    assert_eq!(missions_footer(14, true).as_deref(), Some("Show less"));
}

struct CrewPageHarness {
    visual: gpui::VisualTestContext,
    host: gpui::WindowHandle<crate::NativeRoot>,
    core: crate::AppCore,
    store: gpui::Entity<crate::AppStore>,
    _cx: gpui::TestAppContext,
    _temp: tempfile::TempDir,
    _theme: crate::theme_snapshot::ThemeGuard,
}

fn crew_page_harness(label: &str) -> CrewPageHarness {
    use crate::theme_snapshot::ThemeGuard;
    use crate::*;
    use gpui::{px, size, TestAppContext, VisualTestContext};
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
    let window_store = store.clone();
    let host = cx.add_window(|window, cx| {
        NativeRoot::new(
            label,
            temp.path().join("logs"),
            None,
            None,
            window_store,
            window,
            cx,
        )
    });
    let visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1440.), px(900.)));
    CrewPageHarness {
        visual,
        host,
        core,
        store,
        _cx: cx,
        _temp: temp,
        _theme: theme,
    }
}

impl CrewPageHarness {
    fn role(&self, handle: &str, runtime: Runtime) -> Role {
        runner_backend::ops::role::role_create(
            &self.core,
            runner_backend::ops::role::CreateRoleInput {
                handle: handle.into(),
                display_name: format!("Role {handle}"),
                runtime,
                command: runtime.to_string(),
                args: Vec::new(),
                working_dir: None,
                system_prompt: Some(format!("You are {handle}.\n\nShip it.")),
                env: Default::default(),
                model: Some("opus".into()),
                effort: Some("high".into()),
                codex_speed: None,
                permission_mode: runner_backend::router::runtime::PermissionMode::Default,
            },
        )
        .unwrap()
    }

    /// A crew whose slots fill in order from `handles`, the first leading.
    fn crew(&self, name: &str, conventions: Option<&str>, handles: &[&str]) -> String {
        let crew = runner_backend::ops::crew::crew_create(
            &self.core,
            runner_backend::ops::crew::CreateCrewInput {
                name: name.into(),
                system_prompt_addendum: conventions.map(str::to_owned),
            },
        )
        .unwrap();
        for handle in handles {
            let role = runner_backend::ops::role::role_get_by_handle(&self.core, handle)
                .unwrap_or_else(|_| self.role(handle, Runtime::Codex));
            runner_backend::ops::slot::slot_create(
                &self.core,
                runner_backend::ops::slot::CreateSlotInput {
                    crew_id: crew.id.clone(),
                    role_id: role.id,
                    slot_handle: (*handle).into(),
                    runtime_override: None,
                    model_override: None,
                },
            )
            .unwrap();
        }
        crew.id
    }

    fn open_crew(&mut self, crew_id: &str) {
        let crew_id = crew_id.to_owned();
        self.host
            .update(&mut self.visual, |root, window, cx| {
                root.open_crew_editor(crew_id, window, cx)
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

    fn update<T>(
        &mut self,
        update: impl FnOnce(&mut crate::NativeRoot, &mut Window, &mut Context<crate::NativeRoot>) -> T,
    ) -> T {
        let result = self.host.update(&mut self.visual, update).unwrap();
        self.visual.run_until_parked();
        result
    }

    fn slots(&self, crew_id: &str) -> Vec<SlotWithRole> {
        runner_backend::ops::slot::slot_list(&self.core, crew_id).unwrap()
    }

    fn popup_slot(&mut self) -> Option<String> {
        self.read(|root| {
            root.crew_surfaces
                .editor
                .popup
                .as_ref()
                .map(|popup| popup.slot_id.clone())
        })
    }
}

#[test]
fn crew_page_columns_split_wide_and_stack_at_the_minimum_width() {
    use gpui::{px, size};

    let mut page = crew_page_harness("crew-page-layout");
    let crew = page.crew(
        "Peer coding crew",
        Some(&"This crew works as a two-person coding loop.\n\n".repeat(30)),
        &["lead", "implementer", "coder-codex", "reviewer", "notes"],
    );
    page.open_crew(&crew);
    // 640 wide is the smallest window Runner restores to.
    for width in [640., 900., 1100., 1440.] {
        page.visual.simulate_resize(size(px(width), px(900.)));
        page.visual.run_until_parked();
        let scroll = page.visual.debug_bounds("CREW_EDITOR_SCROLL").unwrap();
        let container = page.visual.debug_bounds("CREW_EDITOR_CONTAINER").unwrap();
        let header = page.visual.debug_bounds("CREW_EDITOR_HEADER").unwrap();
        let profile = page.visual.debug_bounds("CREW_PAGE_PROFILE").unwrap();
        let cards = page.visual.debug_bounds("CREW_PAGE_CARDS").unwrap();
        let left_slack = container.left() - scroll.left();
        let right_slack = scroll.right() - container.right();
        assert!(
            (left_slack - right_slack).abs() <= px(1.),
            "{width}: container is off-centre, slack {left_slack:?} vs {right_slack:?}"
        );
        for (name, bounds) in [("profile", profile), ("cards", cards)] {
            assert!(
                bounds.left() >= header.left() - px(1.)
                    && bounds.right() <= header.right() + px(1.),
                "{width}: {name} {bounds:?} leaves the page {header:?}"
            );
        }
        if width >= 1100. {
            assert!(
                profile.right() + px(1.) < cards.left(),
                "{width}: the cards should sit beside the profile, {profile:?} vs {cards:?}"
            );
            assert_eq!(profile.top(), cards.top(), "{width}: columns share a top");
        }
        let actions = page.visual.debug_bounds("CREW_PAGE_ACTIONS").unwrap();
        assert!(
            actions.size.width <= px(PROFILE_COLUMN_WIDTH + 1.),
            "{width}: Start mission and Edit keep the column's width: {actions:?}"
        );
        if width >= 1100. {
            assert!(
                (profile.size.width - px(PROFILE_COLUMN_WIDTH)).abs() <= px(1.),
                "{width}: the profile keeps its fixed column beside the cards: {profile:?}"
            );
        }
        if width <= 640. {
            assert!(
                cards.top() >= profile.bottom(),
                "{width}: the cards should wrap under the profile, {profile:?} vs {cards:?}"
            );
        }
        if cards.top() >= profile.bottom() {
            let row = page.visual.debug_bounds("CREW_SLOT_ROW").unwrap();
            assert!(
                (profile.size.width - cards.size.width).abs() <= px(1.),
                "{width}: a stacked profile spans the page like the cards: {profile:?} vs {cards:?}"
            );
            assert!(
                (row.right() - profile.right()).abs() <= px(1.),
                "{width}: a stacked slot row spans the page: {row:?} in {profile:?}"
            );
        }
    }
    assert!(page.visual.debug_bounds("CREW_SLOT_LEGEND").is_none());
}

#[test]
fn clicking_a_slot_opens_its_popup_and_esc_or_a_click_outside_closes_it() {
    use gpui::{point, px};

    let mut page = crew_page_harness("crew-slot-popup");
    let crew = page.crew("Pair", None, &["lead", "reviewer"]);
    page.open_crew(&crew);
    let lead = page.slots(&crew)[0].slot.id.clone();
    let row = page.visual.debug_bounds("CREW_SLOT_ROW").unwrap();
    page.click("CREW_SLOT_ROW");
    assert_eq!(page.popup_slot(), Some(lead.clone()));
    let popup = page.visual.debug_bounds("CREW_SLOT_POPUP").unwrap();
    assert!(
        popup.left() >= row.right(),
        "the popup opens beside its row: {popup:?} vs {row:?}"
    );

    page.visual.simulate_keystrokes("escape");
    page.visual.run_until_parked();
    assert_eq!(page.popup_slot(), None, "Esc closes the popup");

    page.click("CREW_SLOT_ROW");
    assert_eq!(page.popup_slot(), Some(lead));
    page.visual
        .simulate_click(point(px(1400.), px(880.)), gpui::Modifiers::default());
    page.visual.run_until_parked();
    assert_eq!(page.popup_slot(), None, "a click outside closes the popup");
}

#[test]
fn an_override_saves_to_the_slot_only_and_reset_restores_the_role() {
    let mut page = crew_page_harness("crew-slot-override");
    let crew = page.crew("Pair", None, &["lead", "reviewer"]);
    page.open_crew(&crew);
    page.click("CREW_SLOT_ROW");
    page.click("CREW_SLOT_EDIT_OVERRIDES");
    let (model, effort) = page.read(|root| {
        let form = root
            .crew_surfaces
            .editor
            .popup
            .as_ref()
            .unwrap()
            .edit
            .as_ref()
            .unwrap();
        (form.model.clone(), form.effort.clone())
    });
    assert_eq!(effort, "", "an unset effort inherits");
    page.update(|_, _, cx| model.update(cx, |input, cx| input.set_text("sonnet", cx)));
    page.click("CREW_SLOT_OVERRIDE_SAVE");

    let slot = page.slots(&crew)[0].clone();
    assert_eq!(slot.slot.model_override.as_deref(), Some("sonnet"));
    assert_eq!(slot.slot.runtime_override, None);
    assert_eq!(slot.slot.effort_override, None);
    let role = runner_backend::ops::role::role_get_by_handle(&page.core, "lead").unwrap();
    assert_eq!(
        role.model.as_deref(),
        Some("opus"),
        "the role never changes"
    );
    assert!(
        page.read(|root| root
            .crew_surfaces
            .editor
            .popup
            .as_ref()
            .unwrap()
            .edit
            .is_none()),
        "saving returns the popup to its summary"
    );
    assert!(page.visual.debug_bounds("CREW_SLOT_LEGEND").is_some());

    page.click("CREW_SLOT_EDIT_OVERRIDES");
    page.click("SLOT_RESET_MODEL");
    page.click("CREW_SLOT_OVERRIDE_SAVE");
    let slot = page.slots(&crew)[0].clone();
    assert_eq!(
        slot.slot.model_override, None,
        "Reset restores the role's model"
    );
    assert_eq!(
        runner_backend::ops::role::role_get_by_handle(&page.core, "lead")
            .unwrap()
            .model
            .as_deref(),
        Some("opus")
    );
}

#[test]
fn codex_slot_speed_edit_shows_credit_note_saves_and_resets() {
    let mut page = crew_page_harness("crew-slot-speed");
    let crew = page.crew("Speed", None, &["coder"]);
    page.open_crew(&crew);
    page.click("CREW_SLOT_ROW");
    page.click("CREW_SLOT_EDIT_OVERRIDES");
    assert!(page.visual.debug_bounds("CREW_SLOT_SPEED_EDIT").is_some());
    page.click("CREW_SLOT_SPEED_EDIT");
    page.click("STYLED_SELECT_OPTION_2");
    assert!(page.visual.debug_bounds("CREW_SLOT_SPEED_NOTE").is_some());
    page.click("CREW_SLOT_OVERRIDE_SAVE");
    assert_eq!(
        page.slots(&crew)[0].slot.codex_speed_override,
        Some(CodexSpeed::Fast)
    );
    assert!(page.visual.debug_bounds("CREW_SLOT_SPEED_ROW").is_some());
    assert_eq!(
        runner_backend::ops::role::role_get_by_handle(&page.core, "coder")
            .unwrap()
            .codex_speed,
        None
    );
    assert!(page.visual.debug_bounds("CREW_SLOT_SPEED_VIEW").is_some());
    assert!(page
        .visual
        .debug_bounds("CREW_SLOT_SPEED_VIEW_NOTE")
        .is_some());

    page.click("CREW_SLOT_EDIT_OVERRIDES");
    page.click("SLOT_RESET_SPEED");
    page.click("CREW_SLOT_OVERRIDE_SAVE");
    assert_eq!(page.slots(&crew)[0].slot.codex_speed_override, None);
    assert_eq!(
        page.read(|root| root.crew_surfaces.editor.slots[0].slot.codex_speed_override),
        None
    );

    page.click("CREW_SLOT_EDIT_OVERRIDES");
    page.update(|root, _, cx| root.select_slot_override_speed("fast", cx));
    page.update(|root, _, cx| root.select_slot_override_runtime("claude-code".into(), cx));
    assert_eq!(
        page.read(|root| root
            .crew_surfaces
            .editor
            .popup
            .as_ref()
            .unwrap()
            .edit
            .as_ref()
            .unwrap()
            .speed),
        None
    );
    page.click("CREW_SLOT_OVERRIDE_SAVE");
    assert_eq!(page.slots(&crew)[0].slot.codex_speed_override, None);
}

#[test]
fn mission_slot_runtime_menu_selection_does_not_reenter_the_select() {
    let mut page = crew_page_harness("crew-slot-runtime-menu");
    let crew = page.crew("Runtime", None, &["coder"]);
    page.open_crew(&crew);
    page.click("CREW_SLOT_ROW");
    page.click("CREW_SLOT_EDIT_OVERRIDES");
    page.click("CREW_SLOT_RUNTIME_EDIT");
    page.click("STYLED_SELECT_OPTION_1");
    assert!(page.read(|root| root
        .crew_surfaces
        .editor
        .popup
        .as_ref()
        .unwrap()
        .edit
        .as_ref()
        .unwrap()
        .runtime
        .is_some()));
}

#[test]
fn non_codex_slot_override_hides_speed() {
    let mut page = crew_page_harness("crew-slot-no-speed");
    page.role("claude", Runtime::ClaudeCode);
    let crew = page.crew("Claude", None, &["claude"]);
    page.open_crew(&crew);
    page.click("CREW_SLOT_ROW");
    page.click("CREW_SLOT_EDIT_OVERRIDES");
    assert!(page.visual.debug_bounds("CREW_SLOT_SPEED_EDIT").is_none());
}

#[test]
fn set_as_lead_and_remove_act_on_the_popups_slot() {
    use gpui::Bounds;

    let mut page = crew_page_harness("crew-slot-actions");
    let crew = page.crew("Pair", None, &["lead", "reviewer"]);
    page.open_crew(&crew);
    let reviewer = page.slots(&crew)[1].slot.id.clone();
    let open = reviewer.clone();
    page.update(|root, window, cx| root.toggle_slot_popup(open, Bounds::default(), window, cx));
    // The row measures itself as it draws; one more frame puts the popup beside it.
    page.update(|_, _, cx| cx.notify());
    page.click("CREW_SLOT_SET_LEAD");
    assert_eq!(page.popup_slot(), None);
    let slots = page.slots(&crew);
    assert!(
        slots[1].slot.lead && !slots[0].slot.lead,
        "the reviewer leads now"
    );

    let open = reviewer.clone();
    page.update(|root, window, cx| root.toggle_slot_popup(open, Bounds::default(), window, cx));
    page.update(|_, _, cx| cx.notify());
    page.click("CREW_SLOT_REMOVE");
    assert_eq!(page.popup_slot(), None);
    assert_eq!(
        page.read(|root| {
            root.crew_surfaces
                .slot_remove_confirm
                .as_ref()
                .map(|confirm| confirm.slot.slot.id.clone())
        }),
        Some(reviewer),
        "Remove asks first"
    );
}

#[test]
fn reorder_still_saves_through_slot_reorder() {
    let mut page = crew_page_harness("crew-slot-reorder");
    let crew = page.crew("Trio", None, &["lead", "coder", "reviewer"]);
    page.open_crew(&crew);
    let lead = page.slots(&crew)[0].slot.id.clone();
    page.update(|root, _, cx| root.commit_slot_reorder(&lead, 2, cx));
    let order = page
        .slots(&crew)
        .into_iter()
        .map(|slot| slot.slot.slot_handle)
        .collect::<Vec<_>>();
    assert_eq!(order, ["coder", "reviewer", "lead"]);
    assert!(page.slots(&crew)[2].slot.lead, "the lead keeps its flag");
}

#[test]
fn edit_in_place_saves_name_and_conventions_together_and_cancel_discards() {
    let mut page = crew_page_harness("crew-edit-in-place");
    let crew = page.crew("Pair", Some("Old conventions."), &["lead"]);
    page.open_crew(&crew);
    page.click("CREW_EDIT");
    assert!(page.visual.debug_bounds("CREW_EDIT_IN_PLACE").is_some());
    assert!(page.visual.debug_bounds("CREW_EDITING_TAG").is_some());
    assert!(page
        .visual
        .debug_bounds("CREW_CONVENTIONS_EDITOR")
        .is_some());
    assert!(page.visual.debug_bounds("CREW_EDIT_DIRTY").is_none());
    let clean_slot = page.visual.debug_bounds("CREW_SLOT_ROW").unwrap();
    let (name, conventions) = page.read(|root| {
        let form = root.crew_surfaces.editor.edit.as_ref().unwrap();
        (form.name.clone(), form.conventions.clone())
    });
    page.update(|_, _, cx| {
        name.update(cx, |input, cx| input.set_text("Release crew", cx));
        conventions.update(cx, |input, cx| {
            input.set_text("## Branch\n\nOne branch.", cx)
        });
    });
    assert!(page.visual.debug_bounds("CREW_EDIT_DIRTY").is_some());
    assert_eq!(
        page.visual.debug_bounds("CREW_SLOT_ROW").unwrap().top(),
        clean_slot.top(),
        "the unsaved note appears in reserved space; the slots below never move"
    );
    page.click("CREW_EDIT_SAVE");
    let saved = runner_backend::ops::crew::crew_get(&page.core, &crew).unwrap();
    assert_eq!(saved.name, "Release crew");
    assert_eq!(
        saved.system_prompt_addendum.as_deref(),
        Some("## Branch\n\nOne branch.")
    );
    assert!(page.read(|root| root.crew_surfaces.editor.edit.is_none()));

    page.click("CREW_EDIT");
    let name = page.read(|root| {
        root.crew_surfaces
            .editor
            .edit
            .as_ref()
            .unwrap()
            .name
            .clone()
    });
    page.update(|_, _, cx| name.update(cx, |input, cx| input.set_text("Draft", cx)));
    page.click("CREW_EDIT_CANCEL");
    assert!(page.read(|root| root.crew_surfaces.editor.edit.is_none()));
    assert_eq!(
        runner_backend::ops::crew::crew_get(&page.core, &crew)
            .unwrap()
            .name,
        "Release crew",
        "Cancel writes nothing"
    );
}

#[test]
fn the_missions_card_shows_four_expands_in_place_and_collapses_for_another_crew() {
    let mut page = crew_page_harness("crew-missions-card");
    let crew = page.crew("Release crew", None, &["lead"]);
    let other = page.crew("Other crew", None, &["reviewer"]);
    let missions = (1..=6)
        .map(|day| {
            summary(mission(
                &format!("m{day}"),
                &crew,
                MissionStatus::Completed,
                Utc.with_ymd_and_hms(2026, 9, day, 10, 0, 0).unwrap(),
                Some(20),
            ))
        })
        .collect::<Vec<_>>();
    let store = page.store.clone();
    page.update(|_, _, cx| store.update(cx, |store, _| store.missions = missions));
    page.open_crew(&crew);
    assert!(!page.read(|root| root.crew_surfaces.editor.missions_expanded));
    let collapsed = page.visual.debug_bounds("CREW_MISSION_ROWS").unwrap();
    page.click("CREW_MISSIONS_TOGGLE");
    assert!(page.read(|root| root.crew_surfaces.editor.missions_expanded));
    let expanded = page.visual.debug_bounds("CREW_MISSION_ROWS").unwrap();
    let row = collapsed.size.height / 4.;
    assert!(
        (expanded.size.height - row * 6.).abs() <= gpui::px(2.),
        "six rows expanded from four: {collapsed:?} vs {expanded:?}"
    );
    page.click("CREW_MISSIONS_TOGGLE");
    assert!(!page.read(|root| root.crew_surfaces.editor.missions_expanded));

    page.click("CREW_MISSIONS_TOGGLE");
    page.open_crew(&other);
    page.open_crew(&crew);
    assert!(
        !page.read(|root| root.crew_surfaces.editor.missions_expanded),
        "the card collapses whenever a crew opens"
    );
}

#[test]
fn crew_list_table_fits_the_minimum_window_and_counts_its_rows() {
    use crate::surfaces::AppRoute;
    use gpui::{px, size};

    let mut page = crew_page_harness("crew-list-layout");
    for index in 0..9 {
        page.crew(&format!("Crew {index}"), None, &[&format!("role-{index}")]);
    }
    page.update(|root, window, cx| root.open_crews(window, cx));
    assert_eq!(page.read(|root| root.route.clone()), AppRoute::Crews);
    for width in [640., 1100., 1440.] {
        page.visual.simulate_resize(size(px(width), px(700.)));
        page.visual.run_until_parked();
        let rows = page.visual.debug_bounds("CREW_TABLE_ROWS").unwrap();
        let header = page.visual.debug_bounds("CREW_TABLE_HEADER").unwrap();
        let actions = page.visual.debug_bounds("CREW_ROW_ACTIONS").unwrap();
        let new_crew = page.visual.debug_bounds("PAGINATED_LIST_ACTION").unwrap();
        assert!(
            new_crew.right() <= rows.right() + px(1.) && new_crew.size.width > px(40.),
            "{width}: + New crew {new_crew:?} is pushed off the page {rows:?}"
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
    let crews = runner_backend::ops::crew::crew_list(&page.core, 1, 100, "")
        .unwrap()
        .total_count as usize;
    assert!(crews >= 9);
    let (filtered, total, searching) = page.read(|root| {
        let list = &root.crew_surfaces.list;
        (list.filtered_count, list.total_count, list.searching())
    });
    assert_eq!((filtered, total, searching), (crews, crews, false));
    assert_eq!(
        runner_app::ui::count_label(filtered, total, "crews", searching),
        format!("{crews} crews")
    );
    assert_eq!(
        page.read(|root| root.crew_surfaces.list.items.len()),
        runner_app::ui::list::PAGE_SIZE,
        "the table pages at eight"
    );
}

#[test]
fn a_same_runtime_pin_shows_as_an_override_survives_an_edit_and_resets() {
    let mut page = crew_page_harness("crew-slot-pin");
    let crew = page.crew("Solo", None, &[]);
    let role = page.role("pin-coder", Runtime::Codex);
    runner_backend::ops::slot::slot_create(
        &page.core,
        runner_backend::ops::slot::CreateSlotInput {
            crew_id: crew.clone(),
            role_id: role.id,
            slot_handle: "pin-coder".into(),
            runtime_override: Some(Runtime::Codex),
            model_override: None,
        },
    )
    .unwrap();
    page.open_crew(&crew);
    assert!(
        page.visual.debug_bounds("CREW_SLOT_LEGEND").is_some(),
        "the pin reads as an override"
    );
    page.click("CREW_SLOT_ROW");
    page.click("CREW_SLOT_EDIT_OVERRIDES");
    assert!(page.visual.debug_bounds("SLOT_RESET_RUNTIME").is_some());
    page.click("CREW_SLOT_OVERRIDE_SAVE");
    assert_eq!(
        page.slots(&crew)[0].slot.runtime_override.as_deref(),
        Some("codex"),
        "saving without touching the runtime keeps the pin"
    );

    page.click("CREW_SLOT_EDIT_OVERRIDES");
    page.click("SLOT_RESET_RUNTIME");
    page.click("CREW_SLOT_OVERRIDE_SAVE");
    assert_eq!(
        page.slots(&crew)[0].slot.runtime_override,
        None,
        "Reset clears the pin"
    );
}

#[test]
fn the_popups_text_actions_answer_enter_and_space() {
    use crate::surfaces::AppRoute;
    use gpui::Bounds;

    let mut page = crew_page_harness("crew-slot-keys");
    let crew = page.crew("Pair", None, &["lead", "reviewer"]);
    page.open_crew(&crew);
    let slots = page.slots(&crew);
    let (lead, reviewer) = (slots[0].slot.id.clone(), slots[1].slot.id.clone());
    let open = |page: &mut CrewPageHarness, slot_id: &str| {
        let slot_id = slot_id.to_owned();
        page.update(|root, window, cx| {
            root.toggle_slot_popup(slot_id, Bounds::default(), window, cx)
        });
        // The row measures itself as it draws; one more frame puts the popup beside it.
        page.update(|_, _, cx| cx.notify());
    };
    let press = |page: &mut CrewPageHarness, focus: fn(&SlotPopup) -> FocusHandle, key: &str| {
        page.update(|root, window, cx| {
            focus(root.crew_surfaces.editor.popup.as_ref().unwrap()).focus(window, cx)
        });
        page.visual.simulate_keystrokes(key);
        page.visual.run_until_parked();
    };

    open(&mut page, &reviewer);
    press(&mut page, |popup| popup.remove_focus.clone(), "enter");
    assert_eq!(
        page.read(|root| {
            root.crew_surfaces
                .slot_remove_confirm
                .as_ref()
                .map(|confirm| confirm.slot.slot.id.clone())
        }),
        Some(reviewer),
        "Enter on Remove asks to remove the slot"
    );
    page.update(|root, _, _| root.crew_surfaces.slot_remove_confirm = None);

    open(&mut page, &lead);
    page.click("CREW_SLOT_EDIT_OVERRIDES");
    let model = page.read(|root| {
        let popup = root.crew_surfaces.editor.popup.as_ref().unwrap();
        popup.edit.as_ref().unwrap().model.clone()
    });
    page.update(|_, _, cx| model.update(cx, |input, cx| input.set_text("sonnet", cx)));
    press(
        &mut page,
        |popup| popup.edit.as_ref().unwrap().reset_focus[1].clone(),
        "space",
    );
    assert_eq!(
        page.update(|_, _, cx| model.read(cx).text().to_owned()),
        "",
        "Space on Reset clears the model override"
    );

    page.update(|root, _, cx| {
        root.crew_surfaces.editor.popup.as_mut().unwrap().edit = None;
        cx.notify();
    });
    press(&mut page, |popup| popup.open_role_focus.clone(), "enter");
    assert_eq!(
        page.read(|root| root.route.clone()),
        AppRoute::RoleDetail("lead".into()),
        "Enter on Open role opens the role"
    );
}

#[test]
fn the_edit_popup_fits_the_minimum_window_and_its_footer_stays_reachable() {
    use gpui::{px, size};

    let mut page = crew_page_harness("crew-slot-popup-min");
    let crew = page.crew("Pair", None, &["lead", "reviewer"]);
    page.open_crew(&crew);
    // Runner's smallest window.
    page.visual.simulate_resize(size(px(640.), px(480.)));
    page.visual.run_until_parked();
    page.click("CREW_SLOT_ROW");
    page.click("CREW_SLOT_EDIT_OVERRIDES");
    let model = page.read(|root| {
        let popup = root.crew_surfaces.editor.popup.as_ref().unwrap();
        popup.edit.as_ref().unwrap().model.clone()
    });
    // A different runtime adds its note, the tallest setup the popup shows.
    page.update(|root, _, cx| root.select_slot_override_runtime("claude-code".into(), cx));
    page.update(|_, _, cx| model.update(cx, |input, cx| input.set_text("sonnet", cx)));
    let popup = page.visual.debug_bounds("CREW_SLOT_POPUP").unwrap();
    let body = page.visual.debug_bounds("CREW_SLOT_POPUP_BODY").unwrap();
    let save = page.visual.debug_bounds("CREW_SLOT_OVERRIDE_SAVE").unwrap();
    assert!(
        popup.top() >= px(0.) && popup.bottom() <= px(480.),
        "the popup stays inside the window: {popup:?}"
    );
    assert!(
        save.top() >= popup.top() && save.bottom() <= popup.bottom(),
        "Save stays on screen: {save:?} in {popup:?}"
    );
    assert!(
        body.size.height < popup.size.height,
        "the setup scrolls between the header and footer: {body:?} in {popup:?}"
    );
    page.click("CREW_SLOT_OVERRIDE_SAVE");
    let slot = page.slots(&crew)[0].clone();
    assert_eq!(slot.slot.runtime_override.as_deref(), Some("claude-code"));
    assert_eq!(slot.slot.model_override.as_deref(), Some("sonnet"));
}

#[test]
fn a_lowercased_handle_undoes_to_before_the_keystroke_and_redoes() {
    let mut page = crew_page_harness("crew-add-slot-undo");
    let crew = page.crew("Pair", None, &["lead", "reviewer"]);
    page.open_crew(&crew);
    page.update(|root, window, cx| root.open_add_slot(window, cx));
    let handle = page.read(|root| {
        root.crew_surfaces
            .add_slot
            .as_ref()
            .unwrap()
            .slot_handle
            .clone()
    });
    let text =
        |page: &mut CrewPageHarness| page.update(|_, _, cx| handle.read(cx).text().to_owned());
    let suggested = text(&mut page);
    page.update(|_, window, cx| handle.read(cx).focus_handle().focus(window, cx));

    page.visual.simulate_input("A");
    page.visual.run_until_parked();
    assert_eq!(text(&mut page), format!("{suggested}a"));

    let (undo, redo) = if cfg!(windows) {
        ("ctrl-z", "ctrl-y")
    } else {
        ("cmd-z", "cmd-shift-z")
    };
    page.visual.simulate_keystrokes(undo);
    page.visual.run_until_parked();
    assert_eq!(
        text(&mut page),
        suggested,
        "one undo takes back the keystroke"
    );
    page.visual.simulate_keystrokes(redo);
    page.visual.run_until_parked();
    assert_eq!(text(&mut page), format!("{suggested}a"));
}

#[test]
fn new_crew_entry_points_use_a_page_and_cancel_returns_to_the_list() {
    use crate::surfaces::AppRoute;
    for entry in ["NEW_CREW", "EMPTY_NEW_CREW"] {
        let mut page = crew_page_harness("new-crew-entry");
        page.update(|root, window, cx| root.open_crews(window, cx));
        let count = runner_backend::ops::crew::crew_list(&page.core, 1, 20, "")
            .unwrap()
            .total_count;
        if entry == "EMPTY_NEW_CREW" {
            page.update(|root, _, cx| {
                let list = &mut root.crew_surfaces.list;
                list.items.clear();
                list.total_count = 0;
                list.filtered_count = 0;
                cx.notify();
            });
        }
        page.click(entry);
        assert_eq!(page.read(|root| root.route.clone()), AppRoute::NewCrew);
        for selector in [
            "CREW_NEW_TAG",
            "EMPTY_PROFILE_TILE",
            "CREW_EDIT_IN_PLACE",
            "CREW_CONVENTIONS_CARD",
        ] {
            assert!(
                page.visual.debug_bounds(selector).is_some(),
                "missing {selector}"
            );
        }
        assert!(page.visual.debug_bounds("CREW_MISSIONS_CARD").is_none());
        assert!(page.visual.debug_bounds("CREW_DETAILS").is_none());
        assert!(page.visual.debug_bounds("CREW_EDIT_DIRTY").is_none());
        page.update(|root, window, cx| {
            assert!(root.render_crew_overlays(window, cx).is_empty());
            root.open_add_slot(window, cx);
            assert!(root.crew_surfaces.add_slot.is_none());
        });
        page.click("CREW_EDIT_SAVE");
        assert_eq!(
            runner_backend::ops::crew::crew_list(&page.core, 1, 20, "")
                .unwrap()
                .total_count,
            count
        );
        page.click("CREW_EDIT_CANCEL");
        assert_eq!(page.read(|root| root.route.clone()), AppRoute::Crews);
    }
}

#[test]
fn new_crew_saves_conventions_opens_view_mode_and_allows_slots() {
    use crate::surfaces::AppRoute;
    let mut page = crew_page_harness("new-crew-submit");
    page.role("new-crew-coder", Runtime::Codex);
    page.update(|root, window, cx| {
        root.open_create_crew(window, cx);
        let form = root.crew_surfaces.create.as_ref().unwrap();
        form.name.update(cx, |input, cx| input.set_text("Pair", cx));
        form.conventions.update(cx, |input, cx| {
            input.set_text("# Reviews\n\nUse the feed.", cx)
        });
    });
    page.click("CREW_EDIT_SAVE");
    let item = runner_backend::ops::crew::crew_list(&page.core, 1, 20, "")
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.crew.name == "Pair")
        .unwrap();
    let crew = runner_backend::ops::crew::crew_get(&page.core, &item.crew.id).unwrap();
    assert_eq!(
        crew.system_prompt_addendum.as_deref(),
        Some("# Reviews\n\nUse the feed.")
    );
    assert_eq!(
        page.read(|root| root.route.clone()),
        AppRoute::CrewEditor(crew.id.clone())
    );
    assert!(page.read(
        |root| root.crew_surfaces.create.is_none() && root.crew_surfaces.editor.edit.is_none()
    ));
    page.update(|root, window, cx| root.open_add_slot(window, cx));
    assert!(page.read(|root| root.crew_surfaces.add_slot.is_some()));
}

#[test]
fn crew_edit_tabs_through_actions_slots_and_conventions() {
    let mut page = crew_page_harness("crew-edit-tab-order");
    let crew = page.crew("Pair", Some("Review the diff."), &["lead", "reviewer"]);
    page.open_crew(&crew);
    page.click("CREW_EDIT");
    for index in 0..2 {
        page.visual.simulate_keystrokes("tab");
        page.update(|root, window, _| {
            let form = root.crew_surfaces.editor.edit.as_ref().unwrap();
            assert!(form.action_focus[index].is_focused(window));
        });
    }
    page.visual.simulate_keystrokes("tab");
    let add_slot_focus = page.update(|_, window, cx| window.focused(cx).unwrap());
    let mut slot_focus = Vec::new();
    for _ in 0..2 {
        page.visual.simulate_keystrokes("tab");
        slot_focus.push(page.update(|_, window, cx| window.focused(cx).unwrap()));
    }
    for index in 0..2 {
        page.visual.simulate_keystrokes("tab");
        page.update(|root, window, _| {
            let form = root.crew_surfaces.editor.edit.as_ref().unwrap();
            assert!(form.mode_focus[index].is_focused(window));
        });
    }
    page.visual.simulate_keystrokes("tab");
    page.update(|root, window, cx| {
        let form = root.crew_surfaces.editor.edit.as_ref().unwrap();
        assert!(form.conventions.read(cx).focus_handle().is_focused(window));
    });
    for (index, focus) in slot_focus.iter().enumerate() {
        page.update(|_, window, cx| focus.focus(window, cx));
        page.visual.simulate_keystrokes("enter");
        page.visual.run_until_parked();
        assert_eq!(
            page.popup_slot(),
            Some(page.slots(&crew)[index].slot.id.clone())
        );
        page.visual.simulate_keystrokes("escape");
        page.visual.run_until_parked();
    }
    page.update(|_, window, cx| add_slot_focus.focus(window, cx));
    page.visual.simulate_keystrokes("enter");
    page.visual.run_until_parked();
    assert!(page.read(|root| root.crew_surfaces.add_slot.is_some()));
}

#[test]
fn new_crew_tabs_skip_disabled_create_and_reach_the_breadcrumb() {
    use crate::surfaces::AppRoute;
    let mut page = crew_page_harness("new-crew-breadcrumb-keyboard");
    page.update(|root, window, cx| root.open_create_crew(window, cx));
    page.visual.simulate_keystrokes("tab");
    page.update(|root, window, _| {
        assert!(root.crew_surfaces.create.as_ref().unwrap().action_focus[1].is_focused(window));
    });
    page.visual.simulate_keystrokes("shift-tab shift-tab enter");
    page.visual.run_until_parked();
    assert_eq!(page.read(|root| root.route.clone()), AppRoute::Crews);
}

#[test]
fn add_slot_create_role_opens_its_page_and_cancel_returns_to_the_crew() {
    use crate::surfaces::AppRoute;
    let mut page = crew_page_harness("new-role-from-slot");
    let crew = page.crew("Pair", None, &["coder"]);
    page.open_crew(&crew);
    page.update(|root, window, cx| root.open_add_slot(window, cx));
    page.click("ADD_SLOT_CREATE_ROLE");
    assert_eq!(page.read(|root| root.route.clone()), AppRoute::NewRole);
    assert!(page.read(|root| root.crew_surfaces.add_slot.is_none()));
    page.click("ROLE_EDIT_CANCEL");
    assert_eq!(
        page.read(|root| root.route.clone()),
        AppRoute::CrewEditor(crew)
    );
}

#[test]
fn new_crew_layout_fits_the_minimum_window_in_both_themes() {
    let mut page = crew_page_harness("new-crew-small");
    page.update(|root, window, cx| root.open_create_crew(window, cx));
    page.visual
        .simulate_resize(gpui::size(gpui::px(640.), gpui::px(480.)));
    for variant in [
        theme::ThemeVariant::Carbon,
        theme::ThemeVariant::RunnerLight,
    ] {
        theme::set_active_variant(variant);
        page.update(|_, _, cx| cx.notify());
        let profile = page.visual.debug_bounds("CREW_PAGE_PROFILE").unwrap();
        let cards = page.visual.debug_bounds("CREW_PAGE_CARDS").unwrap();
        assert!(profile.right() <= gpui::px(640.));
        assert!(cards.right() <= gpui::px(640.));
        assert!(cards.top() >= profile.bottom());
    }
}

#[test]
fn new_crew_enter_is_ime_safe_and_conventions_enter_does_not_submit() {
    use gpui::EntityInputHandler;
    let mut page = crew_page_harness("new-crew-keyboard");
    page.update(|root, window, cx| root.open_create_crew(window, cx));
    page.update(|root, window, cx| {
        root.crew_surfaces
            .create
            .as_ref()
            .unwrap()
            .name
            .update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "Pair", Some(4..4), window, cx)
            });
    });
    page.visual.simulate_keystrokes("enter escape");
    page.visual.run_until_parked();
    assert!(page.read(|root| root.crew_surfaces.create.is_some()));
    page.update(|root, window, cx| {
        let form = root.crew_surfaces.create.as_ref().unwrap();
        form.name.update(cx, |input, cx| {
            input.replace_text_in_range(None, "Keyboard crew", window, cx)
        });
        form.conventions.read(cx).focus_handle().focus(window, cx);
    });
    page.visual.simulate_input("Conventions");
    page.visual.simulate_keystrokes("enter");
    page.visual.run_until_parked();
    assert!(page.read(|root| root.crew_surfaces.create.is_some()));
    page.update(|root, window, cx| {
        root.crew_surfaces
            .create
            .as_ref()
            .unwrap()
            .name
            .read(cx)
            .focus_handle()
            .focus(window, cx)
    });
    page.visual.simulate_keystrokes("enter");
    page.visual.run_until_parked();
    assert!(page.read(|root| matches!(root.route, crate::surfaces::AppRoute::CrewEditor(_))));
}
