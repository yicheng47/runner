use super::*;

impl NativeRoot {
    pub(crate) fn terminal_style(&self, cx: &App) -> crate::terminal::element::TerminalStyle {
        let mut style = terminal_style_for(self.settings(cx), cx);
        if let Some(palette) = self.remote_client.remote_terminal_palette(cx) {
            style.palette = palette;
        }
        style
    }

    pub(crate) fn render_collapsed_titlebar_spacer(&self) -> Option<AnyElement> {
        (self.front_page_sidebar_collapsed() && cfg!(target_os = "macos")).then(|| {
            div()
                .debug_selector(|| "WINDOW_NAVIGATION_SPACE".into())
                .flex_none()
                .w(rems(92. / 16.))
                .h(rems(28. / 16.))
                .into_any_element()
        })
    }

    pub(crate) fn page_navigation_state(&self) -> (bool, bool) {
        let in_settings = self.route == AppRoute::Settings || self.remote_client.is_active();
        let can_go_back =
            !in_settings && self.runtime_navigation_index.is_some_and(|index| index > 0);
        let can_go_forward = !in_settings
            && self
                .runtime_navigation_index
                .is_some_and(|index| index + 1 < self.runtime_navigation_history.len());
        (can_go_back, can_go_forward)
    }

    pub(crate) fn workspace_titlebar_padding(&self, window: &Window, cx: &App) -> f32 {
        let zoom = self.settings(cx).app_zoom;
        if self.front_page_sidebar_collapsed() && cfg!(target_os = "macos") {
            // The card's scaled inset and unscaled edge already consume part of the window anchor.
            platform_ui::navigation_left(window, zoom) - 8. * zoom - 1.
        } else {
            16. * zoom
        }
    }

    pub(crate) fn sync_theme(&self, window: &Window, cx: &mut Context<Self>) {
        let system_is_light = matches!(
            window.appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        );
        let variant = theme::resolve_variant(
            self.settings(cx).app_theme,
            system_is_light,
            self.settings(cx).light_app_theme,
            self.settings(cx).dark_app_theme,
        );
        let previous = theme::active_variant();
        if previous != variant {
            theme::set_active_variant(variant);
            self.app_store
                .update(cx, |store, cx| store.theme_changed(previous, cx));
        }
        self.apply_terminal_palette(cx);
    }

    pub(crate) fn apply_terminal_palette(&self, cx: &App) {
        self.app_store
            .read(cx)
            .bridge
            .set_palette(terminal_style_for(self.settings(cx), cx).palette);
    }

    pub(crate) fn save_settings(&self, cx: &App) {
        self.app_store.read(cx).save_settings();
    }

    pub(crate) fn update_app_settings(
        &self,
        cx: &mut Context<Self>,
        persist: bool,
        update: impl FnOnce(&mut AppSettings) -> bool,
    ) -> bool {
        self.app_store.update(cx, |store, store_cx| {
            store.update_settings(update, persist, store_cx)
        })
    }

    pub(crate) fn set_sidebar_collapsed(
        &mut self,
        collapsed: bool,
        persist: bool,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_active() {
            self.remote_client.set_sidebar_collapsed(collapsed, cx);
            return;
        }

        let changed = self.sidebar_collapsed != collapsed;
        self.sidebar_collapsed = collapsed;
        if collapsed {
            self.usage_open = false;
            self.update_popover.close_without_focus();
        }
        self.mission_workspace
            .update(cx, |workspace, workspace_cx| {
                workspace.set_sidebar_collapsed(collapsed, workspace_cx)
            });
        self.update_app_settings(cx, persist, |settings| {
            if settings.sidebar_collapsed == collapsed {
                return false;
            }
            settings.sidebar_collapsed = collapsed;
            true
        });
        if changed {
            cx.notify();
        }
    }

    pub(crate) fn note_window_state(&mut self, window: &Window) {
        self.window_state = window_state::snapshot(window, Some(self.window_state));
    }

    pub(crate) fn schedule_window_state_checkpoint(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.note_window_state(window);
        self.window_state_save_generation = self.window_state_save_generation.wrapping_add(1);
        let generation = self.window_state_save_generation;
        cx.spawn_in(window, async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(WINDOW_STATE_SAVE_DELAY_MS))
                .await;
            let _ = weak.update_in(cx, |this, _, cx| {
                if this.window_state_save_generation == generation {
                    checkpoint_window_layout_deferred(cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn save_main_window_state(&mut self, window: &Window, cx: &App) {
        if self.window_label != "main" {
            return;
        }
        self.note_window_state(window);
        if let Err(error) =
            window_state::save(&self.app_store.read(cx).app_data_dir, self.window_state)
        {
            eprintln!("Runner window-state save failed: {error:#}");
        }
    }

    pub(crate) fn leave_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_remote_dialog();
        if self.shortcut_recording_active() {
            self.finish_shortcut_recording(window, cx);
        }
        self.save_settings(cx);
        match self.settings_return_route.clone() {
            AppRoute::Chat | AppRoute::Settings => {
                self.set_route(AppRoute::Chat, cx);
                match self.ensure_active_tab_attached(window, cx) {
                    Ok(()) => {
                        self.mark_active_tab_viewed(window, cx);
                        self.focus_active_terminal(window, cx);
                    }
                    Err(error) => self.chat_error = Some(error.to_string().into()),
                }
                cx.notify();
            }
            AppRoute::Launcher => {
                self.set_route(AppRoute::Launcher, cx);
                self.focus_visible_launcher(window, cx);
                cx.notify();
            }
            AppRoute::Roles => self.open_roles(window, cx),
            AppRoute::NewRole => self.open_create_role(window, cx),
            AppRoute::RoleDetail(handle) => self.open_role_detail(handle, window, cx),
            AppRoute::Crews => self.open_crews(window, cx),
            AppRoute::NewCrew => self.open_create_crew(window, cx),
            AppRoute::CrewEditor(crew_id) => self.open_crew_editor(crew_id, window, cx),
            AppRoute::Usage => self.open_usage(window, cx),
            AppRoute::MissionControl => {
                self.set_route(AppRoute::MissionControl, cx);
                if let Some(project_id) = self.active_project_id(cx) {
                    self.refresh_mission_control(project_id, cx);
                }
                window.focus(&self.root_focus, cx);
                cx.notify();
            }
            AppRoute::Mission(mission_id) => self.open_mission(mission_id, window, cx),
            AppRoute::ArchivedChat => {
                self.set_route(AppRoute::ArchivedChat, cx);
                self.chat_focus.focus(window, cx);
                cx.notify();
            }
        }
    }

    pub(crate) fn leave_archived_mission(
        &mut self,
        mission_id: &str,
        was_archived: Option<bool>,
        is_archived: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(route) =
            route_after_mission_archived(&self.route, mission_id, was_archived, is_archived)
        else {
            return;
        };
        self.land_on_recent_tab(cx);
        if self.remote_client.is_local() && self.tabs.active().is_none() {
            self.reset_launcher_page(cx);
        }
        self.set_route(route, cx);
        self.sync_active_project_from_active_tab(cx);
        match self.ensure_active_tab_attached(window, cx) {
            Ok(()) => self.settle_active_tab(window, cx),
            Err(error) => self.chat_error = Some(error.to_string().into()),
        }
        cx.notify();
    }

    pub(crate) fn toggle_window_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.route == AppRoute::Settings && self.remote_client.is_local() {
            return;
        }
        let collapsed = !self.front_page_sidebar_collapsed();
        self.set_sidebar_collapsed(collapsed, true, cx);
        self.sidebar_preview_peeking = false;
        if !collapsed {
            self.sidebar_preview_open = false;
            match self.remote_client.remote_mission_workspace() {
                Some(workspace) => {
                    workspace.update(cx, |workspace, cx| {
                        workspace.focus_active_mission_terminal(window, cx)
                    });
                }
                _ => {
                    if self.remote_client.is_local() && matches!(self.route, AppRoute::Mission(_)) {
                        self.mission_workspace.update(cx, |workspace, cx| {
                            workspace.focus_active_mission_terminal(window, cx);
                        });
                    } else if self.route == AppRoute::Chat {
                        self.focus_active_terminal(window, cx);
                    }
                }
            }
        }
        cx.notify();
    }

    pub(super) fn toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route == AppRoute::Settings && self.remote_client.is_local() {
            return;
        }
        let collapsed = !self.front_page_sidebar_collapsed();
        self.set_sidebar_collapsed(collapsed, true, cx);
        if !collapsed {
            self.sidebar_preview_open = false;
        }
        self.sidebar_preview_peeking = false;
        cx.notify();
    }

    pub(crate) fn switch_sidebar_view(
        &mut self,
        _: &SwitchSidebarView,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route == AppRoute::Settings && self.remote_client.is_local() {
            return;
        }
        self.toggle_sidebar_view(cx);
        self.set_sidebar_collapsed(false, true, cx);
        self.sidebar_preview_open = false;
        self.sidebar_preview_peeking = false;
        cx.notify();
    }

    pub(super) fn toggle_terminal_drawer_action(
        &mut self,
        _: &ToggleTerminalDrawer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_terminal_drawer(window, cx);
    }

    pub(super) fn select_sidebar_shortcut(
        &mut self,
        index: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_sidebar_shortcut_rows(cx);
        self.front_page_sidebar().update(cx, |sidebar, sidebar_cx| {
            sidebar.select_shortcut_row(index, window, sidebar_cx)
        });
    }

    pub(super) fn select_tab_1(
        &mut self,
        _: &SelectTab1,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_sidebar_shortcut(1, window, cx);
    }

    pub(super) fn select_tab_2(
        &mut self,
        _: &SelectTab2,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_sidebar_shortcut(2, window, cx);
    }

    pub(super) fn select_tab_3(
        &mut self,
        _: &SelectTab3,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_sidebar_shortcut(3, window, cx);
    }

    pub(super) fn select_tab_4(
        &mut self,
        _: &SelectTab4,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_sidebar_shortcut(4, window, cx);
    }

    pub(super) fn select_tab_5(
        &mut self,
        _: &SelectTab5,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_sidebar_shortcut(5, window, cx);
    }

    pub(super) fn select_tab_6(
        &mut self,
        _: &SelectTab6,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_sidebar_shortcut(6, window, cx);
    }

    pub(super) fn select_tab_7(
        &mut self,
        _: &SelectTab7,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_sidebar_shortcut(7, window, cx);
    }

    pub(super) fn select_tab_8(
        &mut self,
        _: &SelectTab8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_sidebar_shortcut(8, window, cx);
    }

    pub(super) fn select_tab_9(
        &mut self,
        _: &SelectTab9,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_sidebar_shortcut(9, window, cx);
    }

    pub(super) fn show_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.route == AppRoute::Settings && self.remote_client.is_local() {
            return;
        }
        self.remote_client
            .refresh_remote_palette(&self.command_palette, cx);
        self.command_palette
            .update(cx, |palette, palette_cx| palette.open(window, palette_cx));
    }

    pub(super) fn open_command_palette(
        &mut self,
        _: &CommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_command_palette(window, cx);
    }

    pub(super) fn search_files(
        &mut self,
        _: &crate::SearchFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.route {
            AppRoute::Chat => self.search_files_in_chat(window, cx),
            AppRoute::Mission(_) => self.mission_workspace.update(cx, |workspace, cx| {
                workspace.search_files(window, cx);
            }),
            _ => cx.propagate(),
        }
    }

    pub(super) fn open_settings(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route == AppRoute::Settings {
            return;
        }
        self.enter_settings_route(None, window, cx);
    }

    pub(super) fn navigate_previous_page(
        &mut self,
        _: &NavigatePreviousPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_runtime_page(-1, window, cx);
    }

    pub(super) fn navigate_next_page(
        &mut self,
        _: &NavigateNextPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_runtime_page(1, window, cx);
    }

    pub(super) fn zoom_in(&mut self, _: &ZoomIn, window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(nudge_zoom(self.settings(cx).app_zoom, 1), window, cx);
    }

    pub(super) fn zoom_out(&mut self, _: &ZoomOut, window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(nudge_zoom(self.settings(cx).app_zoom, -1), window, cx);
    }

    pub(super) fn zoom_reset(
        &mut self,
        _: &ZoomReset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_zoom(1., window, cx);
    }

    pub(crate) fn set_zoom(&mut self, zoom: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.update_app_settings(cx, true, |settings| {
            if settings.app_zoom == zoom {
                return false;
            }
            settings.app_zoom = zoom;
            true
        });
        window.set_rem_size(px(16. * zoom));
        platform_ui::sync_traffic_lights(window, zoom);
        cx.notify();
    }

    pub(super) fn toggle_fullscreen(
        &mut self,
        _: &ToggleFullscreen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.toggle_fullscreen();
        cx.notify();
    }
}
