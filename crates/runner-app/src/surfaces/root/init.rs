use super::*;

impl NativeRoot {
    pub(crate) fn new(
        window_label: String,
        log_dir: PathBuf,
        initial_route_path: Option<String>,
        initial_window_state: Option<window_state::WindowState>,
        app_store: Entity<AppStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let core = app_store.read(cx).client.clone();
        let settings = app_store.read(cx).settings.clone();
        let sessions = app_store.read(cx).sessions.clone();
        let nodes = app_store.read(cx).nodes.clone();
        let store_revisions = app_store.read(cx).revisions;
        let startup_error = app_store.read(cx).error.clone();

        let (chat_event_tx, mut chat_event_rx) =
            futures::channel::mpsc::unbounded::<runner_core::protocol::ClientEvent>();
        let mut chat_events = core.subscribe();
        cx.background_spawn(async move {
            loop {
                match chat_events.recv().await {
                    Ok(event)
                        if matches!(
                            event.name.as_str(),
                            "session/exit"
                                | "session/status"
                                | "session/spawned"
                                | "session/fork-started"
                                | "session/updated"
                                | "session/archived"
                                | "session/warning"
                                | "session/input-error"
                        ) =>
                    {
                        if chat_event_tx.unbounded_send(event).is_err() {
                            break;
                        }
                    }
                    Ok(_) | Err(runner_core::protocol::EventError::Lagged(_)) => {}
                    Err(runner_core::protocol::EventError::Closed) => break,
                }
            }
        })
        .detach();
        cx.spawn_in(window, async move |weak, cx| {
            while let Some(event) = chat_event_rx.next().await {
                if weak
                    .update_in(cx, |this, window, cx| {
                        if matches!(
                            event.name.as_str(),
                            "session/spawned" | "session/exit" | "session/archived"
                        ) {
                            this.refresh_agents_live_sessions(cx);
                        }
                        this.handle_chat_lifecycle_event(event, window, cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (runtime_event_tx, mut runtime_event_rx) =
            futures::channel::mpsc::unbounded::<runner_core::protocol::ClientEvent>();
        let mut app_events = core.subscribe();
        let runtime_client = core.clone();
        cx.background_spawn(async move {
            loop {
                match app_events.recv().await {
                    Ok(event)
                        if matches!(
                            event.name.as_str(),
                            "runtime/changed"
                                | "runtime/task-changed"
                                | "usage/updated"
                                | "claude_accounts/changed"
                        ) =>
                    {
                        if runtime_event_tx.unbounded_send(event).is_err() {
                            break;
                        }
                    }
                    Err(runner_core::protocol::EventError::Lagged(_)) => {
                        if let Ok(tasks) = runtime_client.runtime_task_list() {
                            let event = runner_core::protocol::ClientEvent {
                                name: "runtime/task-changed".into(),
                                payload: serde_json::to_value(tasks).unwrap(),
                            };
                            if runtime_event_tx.unbounded_send(event).is_err() {
                                break;
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(runner_core::protocol::EventError::Closed) => break,
                }
            }
        })
        .detach();
        cx.spawn_in(window, async move |weak, cx| {
            while let Some(event) = runtime_event_rx.next().await {
                let mut events = vec![event];
                while let Ok(pending) = runtime_event_rx.try_recv() {
                    events.push(pending);
                }
                let runtime_changed = events.iter().any(|event| event.name == "runtime/changed");
                if weak
                    .update_in(cx, |this, window, cx| {
                        for event in &events {
                            if event.name == "runtime/task-changed" {
                                if let Ok(tasks) = serde_json::from_value(event.payload.clone()) {
                                    this.apply_agent_tasks(tasks, window, cx);
                                }
                            }
                        }
                        if runtime_changed {
                            this.refresh_start_chat_runtimes(cx);
                            this.refresh_role_form_runtimes(cx);
                            this.refresh_add_slot_runtimes(cx);
                            this.refresh_slot_override_runtimes(cx);
                            this.refresh_agents_pane(cx);
                            this.usage_installed =
                                crate::surfaces::app_shell::usage_installed(this.core(cx));
                            let runtimes = this
                                .core(cx)
                                .runtime_status_list()
                                .ok()
                                .into_iter()
                                .flat_map(|status| status.runtimes)
                                .collect::<Vec<_>>();
                            let supported = runtimes
                                .iter()
                                .filter(|runtime| !runtime.below_minimum)
                                .map(|runtime| runtime.name)
                                .collect::<Vec<_>>();
                            this.clear_unblocked_gates(
                                &surfaces::front_page::FrontPageTarget::Local,
                                &supported,
                            );
                            this.agent_tasks.minimums = runtimes
                                .into_iter()
                                .filter(|runtime| runtime.below_minimum)
                                .filter_map(|runtime| {
                                    Some((
                                        runtime.name,
                                        runner_core::protocol::RuntimeVersionGate {
                                            runtime: runtime.name,
                                            installed_version: runtime.installed_version?,
                                            min_version: runtime.min_version?,
                                        },
                                    ))
                                })
                                .collect();
                            this.agent_updates =
                                crate::surfaces::app_shell::agents_with_updates(this.core(cx));
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        window.set_rem_size(px(16. * settings.app_zoom));
        let mut tabs = TabSet::from_rows(&nodes);
        let route_path = initial_route_path
            .as_deref()
            .unwrap_or_default()
            .trim_matches('/');
        let initial_route = if let Some(mission_id) = route_path
            .strip_prefix("missions/")
            .filter(|id| !id.is_empty())
        {
            AppRoute::Mission(mission_id.to_owned())
        } else if let Some(session_id) = route_path
            .strip_prefix("chats/")
            .filter(|id| !id.is_empty())
        {
            if tabs.activate_session(session_id) {
                AppRoute::Chat
            } else {
                AppRoute::Roles
            }
        } else if let Some(handle) = route_path
            .strip_prefix("roles/")
            .or_else(|| route_path.strip_prefix("runners/"))
            .filter(|handle| !handle.is_empty())
        {
            AppRoute::RoleDetail(handle.to_owned())
        } else if let Some(crew_id) = route_path
            .strip_prefix("crews/")
            .filter(|crew_id| !crew_id.is_empty())
        {
            AppRoute::CrewEditor(crew_id.to_owned())
        } else if route_path == "chats" {
            AppRoute::Chat
        } else if matches!(route_path, "roles" | "runners") {
            AppRoute::Roles
        } else if route_path == "crews" {
            AppRoute::Crews
        } else if route_path == "usage" {
            AppRoute::Usage
        } else if route_path == "mission-control" {
            AppRoute::MissionControl
        } else if route_path == "settings" {
            AppRoute::Settings
        } else if window_label == "main" {
            AppRoute::Chat
        } else {
            AppRoute::Roles
        };

        let root_focus = cx.focus_handle();
        let chat_focus = cx.focus_handle();
        let drawer_focus = cx.focus_handle();
        let active_chat_detail = tabs
            .active()
            .and_then(PaneLayout::focused_session_id)
            .and_then(|session_id| core.session_get(session_id).ok().flatten());
        let initial_session_key = active_chat_detail
            .as_ref()
            .and_then(|entry| entry.agent_session_key.clone());
        let session_key_copy = cx.new(|copy_cx| {
            CopyValueButton::new(
                copy_cx.focus_handle(),
                initial_session_key,
                crate::i18n::message("ui.copy_session_key"),
            )
        });
        let archived_session_key_copy = cx.new(|copy_cx| {
            CopyValueButton::new(
                copy_cx.focus_handle(),
                None,
                crate::i18n::message("ui.copy_session_key"),
            )
        });
        let last_focused_role_id = tabs
            .active()
            .and_then(PaneLayout::focused_session_id)
            .and_then(|session_id| sessions.iter().find(|entry| entry.session_id == session_id))
            .and_then(|entry| entry.role_id.clone());
        let active_project_id = tabs.active_tab_id().and_then(|tab_id| {
            let tab = nodes.iter().find(|node| node.id == tab_id)?;
            let parent_id = tab.parent_id.as_deref()?;
            nodes
                .iter()
                .find(|node| {
                    node.id == parent_id
                        && node.node_type == runner_core::protocol::node::NodeType::Project
                })
                .and_then(|node| node.ref_id.clone())
        });
        let sidebar_collapsed = settings.sidebar_collapsed;
        let sidebar_visibility = SidebarVisibilityTransition::new(!sidebar_collapsed);
        let chat_panel_visibility = SidebarVisibilityTransition::new(settings.chat_panel_shown());
        let role_surfaces = RoleSurfaces::new(cx.entity(), settings.roles_page_size, cx);
        let crew_surfaces = CrewSurfaces::new(cx.entity(), settings.crews_page_size, cx);
        let sidebar_shell = cx.entity().downgrade();
        let sidebar_store = app_store.clone();
        let sidebar = cx.new(move |sidebar_cx| {
            Sidebar::new(sidebar_shell, sidebar_store, active_project_id, sidebar_cx)
        });
        let mission_shell = cx.entity().downgrade();
        let mission_store = app_store.clone();
        let mission_window_label = window_label.clone();
        let mission_workspace = cx.new(|workspace_cx| {
            MissionWorkspace::new(
                mission_window_label,
                mission_shell,
                mission_store,
                window,
                workspace_cx,
            )
        });
        let palette_shell = cx.entity().downgrade();
        let palette_store = app_store.clone();
        let command_palette = cx.new(move |palette_cx| {
            CommandPaletteState::new(palette_shell, palette_store, palette_cx)
        });
        let settings_page = SettingsState::new(cx.entity(), &settings, cx);
        let runtime_navigation_history = match &initial_route {
            AppRoute::Chat => active_chat_detail
                .as_ref()
                .map(|session| vec![RuntimeLocation::Chat(session.session_id.clone())])
                .unwrap_or_default(),
            AppRoute::Mission(mission_id) => vec![RuntimeLocation::Mission(mission_id.clone())],
            AppRoute::Roles
            | AppRoute::NewRole
            | AppRoute::RoleDetail(_)
            | AppRoute::Crews
            | AppRoute::NewCrew
            | AppRoute::CrewEditor(_)
            | AppRoute::Usage
            | AppRoute::MissionControl => vec![RuntimeLocation::Entity(initial_route.clone())],
            _ => Vec::new(),
        };
        let runtime_navigation_index = (!runtime_navigation_history.is_empty()).then_some(0);
        let updater = global_updater(cx);
        let root_entity = cx.entity().downgrade();
        let composition = cx.new(|_| surfaces::render_cache::ShellComposition::new(root_entity));
        let remote_client =
            surfaces::remote_client::RemoteClient::new(app_store.clone(), root_focus.clone(), cx);
        let mut root = Self {
            remote_client,
            composition,
            render_regions: HashMap::new(),
            window_label: window_label.clone(),
            log_dir,
            closing: false,
            app_store: app_store.clone(),
            store_revisions,
            tabs,
            attached: HashMap::new(),
            root_focus,
            chat_focus,
            drawer_focus,
            sidebar,
            launchers: HashMap::new(),
            start_chat_modal: None,
            project_modal: None,
            project_delete_confirm: None,
            project_delete_busy: false,
            stopping_sessions: HashSet::new(),
            chat_transitions: HashMap::new(),
            next_chat_transition_generation: 0,
            session_exit_codes: HashMap::new(),
            chat_error: None,
            daemon_retrying: false,
            chat_warning: None,
            active_chat_detail,
            archived_chat_detail: None,
            session_key_copy,
            archived_session_key_copy,
            pane_action_menus: HashMap::new(),
            split_menus: HashMap::new(),
            pane_bounds: HashMap::new(),
            maximized_panes: HashMap::new(),
            pane_drop: None,
            pane_rename: None,
            _pane_rename_focus_subscription: None,
            _pane_close_focus_subscription: None,
            terminal_close_confirm: None,
            pending_pane_closes: HashMap::new(),
            pending_tab_close: None,
            tab_landed_at: None,
            update_popover: crate::surfaces::update_ui::UpdatePopoverState::new(cx),
            quit_dialog: None,
            agent_tasks: surfaces::background_tasks::BackgroundTasks {
                tasks: core.runtime_task_list().unwrap_or_default(),
                minimums: core
                    .runtime_status_list()
                    .ok()
                    .into_iter()
                    .flat_map(|status| status.runtimes)
                    .filter(|runtime| runtime.below_minimum)
                    .filter_map(|runtime| {
                        Some((
                            runtime.name,
                            runner_core::protocol::RuntimeVersionGate {
                                runtime: runtime.name,
                                installed_version: runtime.installed_version?,
                                min_version: runtime.min_version?,
                            },
                        ))
                    })
                    .collect(),
                ..Default::default()
            },
            agent_update: None,
            fork_confirm: None,
            forking_sessions: HashMap::new(),
            last_focused_role_id,
            split_sizes_dirty: false,
            drawer_resizing: false,
            sidebar_resizing: false,
            chat_panel_resizing: false,
            resizing_split: None,
            chat_secondaries: HashMap::new(),
            dismissed_duplicate_chats: HashSet::new(),
            error: startup_error,
            settings_page,
            route: initial_route.clone(),
            settings_return_route: initial_route,
            usage_open: false,
            usage_refresh_task: None,
            usage_checked: None,
            mission_usage: crate::surfaces::app_shell::MissionUsageState::new(cx),
            mission_control: crate::surfaces::mission_control::MissionControlState::default(),
            claude_account_picker: None,
            usage_anchor: None,
            usage_installed: crate::surfaces::app_shell::usage_installed(&core),
            agent_updates: crate::surfaces::app_shell::agents_with_updates(&core),
            runtime_navigation_history,
            runtime_navigation_index,
            sidebar_collapsed,
            sidebar_visibility,
            chat_panel_visibility,
            chat_panel_prompt_session: None,
            chat_panel_prompt_expanded: false,
            chat_surfaces: crate::surfaces::side_panel::PanelSurfaces::new(Default::default(), cx),
            chat_panel_tab: None,
            chat_panel_open: None,
            chat_panels_by_tab: HashMap::new(),
            chat_files: Default::default(),
            panel_chat_by_tab: HashMap::new(),
            command_palette,
            sidebar_preview_open: false,
            sidebar_preview_peeking: false,
            sidebar_preview_trigger_hovered: false,
            sidebar_preview_dwell: 0,
            sidebar_preview_dwell_generation: 0,
            titlebar_drag_armed: false,
            window_state: initial_window_state
                .unwrap_or_else(|| window_state::snapshot(window, None)),
            window_state_save_generation: 0,
            toasts: ToastHost::default(),
            role_surfaces,
            crew_surfaces,
            mission_workspace,
            _appearance_subscription: None,
            _activation_subscription: None,
            _bounds_subscription: None,
            _project_cwd_subscription: None,
            _updater_subscription: cx.observe_in(&updater, window, |root, _, window, cx| {
                root.handle_updater_change(window, cx);
            }),
            _store_subscription: cx.observe_in(&app_store, window, |this, _, window, cx| {
                this.leave_other_computers_when_signed_out(window, cx);
                this.handle_app_store_update(cx);
            }),
        };
        root._appearance_subscription = Some(cx.observe_window_appearance(window, |_, _, cx| {
            cx.notify();
        }));
        root._bounds_subscription = Some(cx.observe_window_bounds(window, |this, window, cx| {
            platform_ui::sync_traffic_lights(window, this.settings(cx).app_zoom);
            this.schedule_window_state_checkpoint(window, cx);
            this.mission_workspace.update(cx, |workspace, cx| {
                workspace.schedule_mission_grid_hint(window, cx)
            });
            cx.notify();
        }));
        root._activation_subscription =
            Some(cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.app_store.update(cx, |store, cx| store.sync_locale(cx));
                }
                this.sync_sidebar_window_activation(window, cx)
            }));
        platform_ui::sync_traffic_lights(window, root.settings(cx).app_zoom);
        root.sync_theme(window, cx);
        // Subscribed before the window's first reports, so a runnerd that
        // reconnects after they fail still reaches the re-register.
        root.start_focus_map_listener(window, cx);
        match root.route.clone() {
            AppRoute::Mission(mission_id) => root.open_mission(mission_id, window, cx),
            AppRoute::NewRole => root.open_create_role(window, cx),
            AppRoute::NewCrew => root.open_create_crew(window, cx),
            AppRoute::Roles => {
                root.load_role_page(cx);
                window.focus(&root.root_focus, cx);
            }
            AppRoute::RoleDetail(handle) => {
                root.load_role_detail(handle, cx);
                window.focus(&root.root_focus, cx);
            }
            AppRoute::Crews => {
                root.load_crew_page(cx);
                window.focus(&root.root_focus, cx);
            }
            AppRoute::Usage => {
                root.refresh_mission_usage(cx);
                window.focus(&root.root_focus, cx);
            }
            AppRoute::MissionControl => {
                if let Some(project_id) = root.active_project_id(cx) {
                    root.refresh_mission_control(project_id, cx);
                }
                window.focus(&root.root_focus, cx);
            }
            AppRoute::CrewEditor(crew_id) => {
                root.load_crew_editor(crew_id, cx);
                window.focus(&root.root_focus, cx);
            }
            AppRoute::Settings => {
                root.enter_settings_pane(SettingsPane::General, window, cx);
                window.focus(&root.root_focus, cx);
            }
            AppRoute::Chat | AppRoute::Launcher | AppRoute::ArchivedChat => {}
        }
        root.sync_window_activation(window, cx);
        if root.route == AppRoute::Chat {
            if let Err(error) = root.ensure_active_tab_attached(window, cx) {
                root.error = Some(error.to_string().into());
            }
            root.focus_active_terminal(window, cx);
        }
        root
    }
}
