use super::*;

impl NativeRoot {
    pub(crate) fn ensure_active_tab_attached(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.report_current_subjects(cx);
        self.ensure_owned_active_tab_attached(window, cx)
    }

    pub(crate) fn ensure_owned_active_tab_attached(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if self.remote_client.is_active() {
            return Ok(());
        }
        if self.closing {
            return Ok(());
        }
        self.refresh_chat_secondaries(cx);
        let Some(layout) = self.tabs.active().cloned() else {
            self.attached.clear();
            return Ok(());
        };
        let mut errors = Vec::new();
        if let Err(error) = self.resume_visible_drawer_shell_on_launch(&layout, window, cx) {
            errors.push(error.to_string());
        }
        let active_ids = layout.all_session_ids();
        let ended_ids = self
            .tabs
            .tabs()
            .iter()
            .flat_map(PaneLayout::all_session_ids)
            .filter(|id| {
                self.session_entry(id, cx)
                    .is_some_and(|entry| entry.status != SessionStatus::Running)
            });
        let owned_ids = active_ids
            .iter()
            .filter(|session_id| {
                !self.chat_secondaries.contains_key(*session_id)
                    || self
                        .app_store
                        .read(cx)
                        .bridge
                        .session(session_id)
                        .is_some_and(|terminal| !terminal.control().desktop())
            })
            .cloned()
            .chain(ended_ids)
            .collect::<HashSet<_>>();
        // Leaving a tab closes find in its terminals, including the ended
        // sessions kept attached below.
        for (session_id, chat) in &self.attached {
            if !active_ids.contains(session_id) {
                chat.terminal_find.update(cx, |find, cx| find.leave(cx));
            }
        }
        self.attached
            .retain(|session_id, _| owned_ids.contains(session_id));
        for session_id in active_ids {
            if let Err(error) = self.ensure_attached(&layout, &session_id, window, cx) {
                errors.push(error.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(errors.join("\n"))
        }
    }

    pub(crate) fn resume_visible_drawer_shell_on_launch(
        &mut self,
        layout: &PaneLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let Some(session_id) = layout.active_drawer_shell() else {
            return Ok(());
        };
        let Some(status) = self.session_entry(session_id, cx).map(|entry| entry.status) else {
            return Ok(());
        };
        let visible = self.route == AppRoute::Chat
            && layout.drawer_open()
            && !self.chat_secondary_state(session_id, cx).secondary
            && !self.chat_transitions.contains_key(session_id);
        let core = self.core(cx);
        if chat_lifecycle::take_visible_drawer_launch_claim(visible, status, || {
            core.session_take_resume_on_launch(session_id)
        })? {
            self.resume_drawer_shell_on_launch(session_id, window, cx);
        }
        Ok(())
    }

    pub(crate) fn ensure_attached(
        &mut self,
        _layout: &PaneLayout,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if self.chat_secondary_state(session_id, cx).secondary
            && self
                .app_store
                .read(cx)
                .bridge
                .session(session_id)
                .is_none_or(|terminal| terminal.control().desktop())
        {
            return Ok(());
        }
        let status = self
            .session_entry(session_id, cx)
            .map(|entry| entry.status)
            .with_context(|| format!("direct chat not found: {session_id}"))?;
        if self.attached.contains_key(session_id) {
            return Ok(());
        }
        if let Some(attached) = AttachedChat::for_session(
            session_id,
            status,
            &self.app_store,
            window,
            cx,
            |this, result, _| match result {
                Ok(()) => this.chat_error = None,
                Err(error) => this.chat_error = Some(error.into()),
            },
        ) {
            self.attached.insert(session_id.to_owned(), attached);
        }
        Ok(())
    }

    pub(crate) fn estimated_terminal_size(
        &self,
        layout: &PaneLayout,
        pane_id: &str,
        window: &Window,
        cx: &App,
    ) -> (u16, u16) {
        let (width_fraction, height_fraction) =
            pane_fractions(&layout.root, pane_id).unwrap_or((1., 1.));
        let bounds = window.bounds().size;
        let sidebar_width = if self.front_page_sidebar_collapsed() {
            0.
        } else {
            self.front_page_sidebar_width(cx) * self.settings(cx).app_zoom
        };
        let chat_panel_width = if self.settings(cx).chat_panel_shown() {
            self.settings(cx).chat_panel_width * self.settings(cx).app_zoom
        } else {
            0.
        };
        let pane_width =
            (f32::from(bounds.width) - sidebar_width - chat_panel_width).max(200.) * width_fraction;
        let pane_height = (f32::from(bounds.height) - 16. * self.settings(cx).app_zoom).max(160.)
            * height_fraction
            - (PANE_HEADER_HEIGHT + 8.) * self.settings(cx).app_zoom;
        let font_size = self.settings(cx).terminal_font_size as f32 * self.settings(cx).app_zoom;
        let cell_width = font_size * 0.6;
        let line_height =
            crate::terminal::element::row_height(font_size, self.settings(cx).terminal_line_height);
        (
            (pane_width / cell_width).floor().max(2.) as u16,
            (pane_height / line_height).floor().max(2.) as u16,
        )
    }

    pub(crate) fn estimated_drawer_terminal_size(
        &self,
        layout: &PaneLayout,
        window: &Window,
        cx: &App,
    ) -> (u16, u16) {
        let bounds = window.bounds().size;
        let sidebar_width = if self.front_page_sidebar_collapsed() {
            0.
        } else {
            self.front_page_sidebar_width(cx) * self.settings(cx).app_zoom
        };
        let chat_panel_width = if self.settings(cx).chat_panel_shown() {
            self.settings(cx).chat_panel_width * self.settings(cx).app_zoom
        } else {
            0.
        };
        let width = (f32::from(bounds.width) - sidebar_width - chat_panel_width).max(200.);
        let height =
            (layout.drawer_height() - (32. + 5. + 24.)).max(80.) * self.settings(cx).app_zoom;
        let font_size = self.settings(cx).terminal_font_size as f32 * self.settings(cx).app_zoom;
        let cell_width = font_size * 0.6;
        let line_height =
            crate::terminal::element::row_height(font_size, self.settings(cx).terminal_line_height);
        (
            (width / cell_width).floor().max(2.) as u16,
            (height / line_height).floor().max(2.) as u16,
        )
    }

    pub(crate) fn active_tab_is_terminal(&self, cx: &App) -> bool {
        self.front_page_layout().is_some_and(|layout| {
            tab_is_terminal(layout, |session_id| {
                self.front_page_session(session_id, cx)
                    .and_then(|entry| Runtime::parse(&entry.agent_runtime))
            })
        })
    }

    pub(crate) fn control_tab_session_ids(&self, layout: &PaneLayout, cx: &App) -> Vec<String> {
        layout
            .all_session_ids()
            .into_iter()
            .filter(|id| {
                self.session_entry(id, cx)
                    .is_some_and(|entry| entry.status == SessionStatus::Running)
            })
            .collect()
    }

    pub(crate) fn tab_control_covered(&self, layout: &PaneLayout, cx: &App) -> bool {
        (layout.root.leaves().len() > 1 || layout.all_session_ids().len() > 1)
            && self
                .control_tab_session_ids(layout, cx)
                .iter()
                .filter_map(|id| self.attached.get(id))
                .any(|chat| !chat.terminal.control().desktop())
    }

    pub(crate) fn active_focused_session_id(&self) -> Option<String> {
        self.front_page_layout()
            .and_then(PaneLayout::focused_session_id)
            .map(str::to_owned)
    }

    pub(crate) fn session_lifecycle_disabled(&self, session_id: &str, cx: &App) -> bool {
        if self.remote_client.is_active() {
            return self.remote_client.session_pending(session_id);
        }

        self.stopping_sessions.contains(session_id)
            || self.sidebar_archiving_session(session_id, cx)
            || fork_materializing(&self.forking_sessions, session_id)
            || self.chat_transitions.contains_key(session_id)
    }

    pub(crate) fn session_is_interactive(&self, session_id: &str, cx: &App) -> bool {
        if self.remote_client.is_active() {
            return self
                .remote_client
                .remote_terminal_interactive(session_id, cx);
        }

        self.route == AppRoute::Chat
            && !self.tabs.active().is_some_and(|layout| {
                layout.all_session_ids().iter().any(|id| id == session_id)
                    && self.tab_control_covered(layout, cx)
            })
            && self
                .attached
                .get(session_id)
                .is_none_or(|chat| chat.terminal.control().desktop())
            && !self.chat_secondary_state(session_id, cx).secondary
            && self
                .session_entry(session_id, cx)
                .is_some_and(|entry| entry.status == SessionStatus::Running)
            && !self.session_lifecycle_disabled(session_id, cx)
    }

    pub(crate) fn cached_session_is_interactive(&self, session_id: &str, cx: &App) -> bool {
        if self.remote_client.is_active() {
            return self
                .remote_client
                .remote_terminal_interactive(session_id, cx);
        }

        self.route == AppRoute::Chat
            && !self.tabs.active().is_some_and(|layout| {
                layout.all_session_ids().iter().any(|id| id == session_id)
                    && self.tab_control_covered(layout, cx)
            })
            && self
                .attached
                .get(session_id)
                .is_none_or(|chat| chat.terminal.control().desktop())
            && !self.cached_chat_secondary_state(session_id).secondary
            && self
                .session_entry(session_id, cx)
                .is_some_and(|entry| entry.status == SessionStatus::Running)
            && !self.session_lifecycle_disabled(session_id, cx)
    }

    pub(crate) fn focus_active_terminal(&self, window: &mut Window, cx: &mut App) {
        if self.focus_visible_launcher(window, cx) {
            return;
        }
        if self.remote_client.is_active() {
            if let Some(id) = self
                .front_page_layout()
                .and_then(PaneLayout::focused_session_id)
            {
                self.remote_client.focus_remote_terminal(id, window, cx);
            }
            return;
        }

        let Some(session_id) = self.active_focused_session_id() else {
            self.chat_focus.focus(window, cx);
            return;
        };
        if self.session_is_interactive(&session_id, cx) {
            if let Some(chat) = self.attached.get(&session_id) {
                chat.terminal_focus.focus(window, cx);
                return;
            }
        }
        self.chat_focus.focus(window, cx);
    }

    pub(crate) fn activate_tab(
        &mut self,
        tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_active() {
            self.select_remote_item(
                crate::surfaces::remote_client::RemoteItem::Tab(tab_id.into()),
                cx,
            );
            return;
        }

        if !self.tabs.activate(tab_id) {
            return;
        }
        self.sync_active_project_from_active_tab(cx);
        match self.ensure_active_tab_attached(window, cx) {
            Ok(()) => {
                self.chat_error = None;
                self.remember_active_role(cx);
                self.sync_active_chat_detail(cx);
                self.mark_active_tab_viewed(window, cx);
                self.focus_active_terminal(window, cx);
            }
            Err(error) => self.chat_error = Some(error.to_string().into()),
        }
        cx.notify();
    }

    pub(crate) fn focus_pane(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        if self.remote_client.is_active() {
            if let Some(layout) = self.front_page_layout_mut() {
                layout.focus_pane(pane_id);
            }
            cx.notify();
            return;
        }

        let role_id = self.tabs.active().and_then(|layout| {
            layout
                .root
                .leaves()
                .into_iter()
                .find(|leaf| leaf.id == pane_id)
                .and_then(|leaf| leaf.session_id.as_deref())
                .map(|session_id| {
                    self.session_entry(session_id, cx)
                        .and_then(|entry| entry.role_id.clone())
                })
        });
        if self
            .tabs
            .active_mut()
            .is_some_and(|layout| layout.focus_pane(pane_id))
        {
            if let Some(role_id) = role_id {
                self.last_focused_role_id = role_id;
            }
            self.sync_active_chat_detail(cx);
            self.report_current_subjects(cx);
            checkpoint_window_layout_deferred(cx);
            cx.notify();
        }
    }

    pub(crate) fn focus_previous_chat_pane(
        &mut self,
        _: &FocusPreviousPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_chat_pane(-1, window, cx);
    }

    pub(crate) fn focus_next_chat_pane(
        &mut self,
        _: &FocusNextPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_chat_pane(1, window, cx);
    }

    pub(super) fn cycle_chat_pane(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.front_page_is_chat() {
            return;
        }
        let Some(layout) = self.front_page_layout() else {
            return;
        };
        let leaves = layout
            .root
            .leaves()
            .into_iter()
            .map(|leaf| (leaf.id.clone(), leaf.session_id.clone()))
            .collect::<Vec<_>>();
        if leaves.len() < 2 {
            return;
        }
        let current = leaves
            .iter()
            .position(|(pane_id, _)| pane_id == &layout.focused_pane_id)
            .unwrap_or(0);
        let Some(next) = adjacent_pane_index(current, leaves.len(), direction) else {
            return;
        };
        let (pane_id, session_id) = &leaves[next];
        if let Some(session_id) = session_id {
            self.focus_terminal(pane_id, session_id, window, cx);
        } else {
            self.focus_pane(pane_id, cx);
        }
    }

    pub(crate) fn focus_terminal(
        &mut self,
        pane_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_active() {
            self.focus_pane(pane_id, cx);
            self.remote_client
                .focus_remote_terminal(session_id, window, cx);
            return;
        }

        self.focus_pane(pane_id, cx);
        self.last_focused_role_id = self
            .session_entry(session_id, cx)
            .and_then(|entry| entry.role_id.clone());
        if self.session_is_interactive(session_id, cx) {
            if let Some(chat) = self.attached.get(session_id) {
                chat.terminal_focus.focus(window, cx);
            } else {
                self.chat_focus.focus(window, cx);
            }
        } else {
            self.chat_focus.focus(window, cx);
        }
        self.mark_active_tab_viewed(window, cx);
        self.record_current_runtime_location();
    }

    pub(crate) fn focus_drawer_terminal(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_active() {
            self.remote_client
                .focus_remote_terminal(session_id, window, cx);
            return;
        }

        if self.session_is_interactive(session_id, cx) {
            if let Some(chat) = self.attached.get(session_id) {
                chat.terminal_focus.focus(window, cx);
            } else {
                self.drawer_focus.focus(window, cx);
            }
        } else {
            self.drawer_focus.focus(window, cx);
        }
    }

    pub(crate) fn on_key_down(
        &mut self,
        session_id: &str,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .attached
            .get(session_id)
            .is_some_and(|chat| !chat.terminal.control().desktop())
        {
            cx.stop_propagation();
            if event.keystroke.key == "enter" && !event.is_held {
                let group = self
                    .tabs
                    .active()
                    .map(|layout| self.control_tab_session_ids(layout, cx))
                    .unwrap_or_default();
                let chats = if self
                    .tabs
                    .active()
                    .is_some_and(|layout| self.tab_control_covered(layout, cx))
                {
                    group
                        .iter()
                        .filter_map(|id| self.attached.get(id))
                        .collect::<Vec<_>>()
                } else {
                    self.attached.get(session_id).into_iter().collect()
                };
                crate::terminal::control::take_back(&chats, cx);
            }
            return;
        }
        if !self.session_is_interactive(session_id, cx) {
            return;
        }
        let Some(chat) = self.attached.get(session_id) else {
            return;
        };
        let option_is_meta = self.settings(cx).option_as_meta.is_meta(cx);
        match crate::terminal::input::handle_key(
            &chat.terminal,
            chat.terminal_input.read(cx).is_composing(),
            &event.keystroke,
            option_is_meta,
            cx,
        ) {
            Ok(true) => {
                self.chat_error = None;
                cx.stop_propagation();
                cx.notify();
            }
            Ok(false) => {}
            Err(error) => {
                self.chat_error = Some(error.to_string().into());
                cx.stop_propagation();
                cx.notify();
            }
        }
    }

    pub(crate) fn on_terminal_copy(
        &mut self,
        session_id: &str,
        _: &Copy,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let copied = self
            .attached
            .get(session_id)
            .is_some_and(|chat| crate::terminal::input::copy_selection(&chat.terminal, cx));
        if !copied {
            #[cfg(any(windows, target_os = "linux"))]
            cx.propagate();
        }
    }

    pub(crate) fn on_paste(
        &mut self,
        session_id: &str,
        _: &Paste,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.chat_secondary_state(session_id, cx).secondary {
            return;
        }
        let item = cx.read_from_clipboard();
        let Some(paste) = runner_app::terminal_paste::resolve_terminal_paste(item.as_ref(), || {
            self.core(cx)
                .session_clipboard_file_paths()
                .unwrap_or_default()
        }) else {
            return;
        };
        let runtime = self
            .session_entry(session_id, cx)
            .and_then(|entry| Runtime::parse(&entry.agent_runtime));
        let Some(chat) = self.attached.get(session_id) else {
            return;
        };
        if let Err(error) = crate::terminal::input::paste(&chat.terminal, &paste, runtime) {
            self.chat_error = Some(error.to_string().into());
        }
    }

    /// `pane_id` is `None` for a terminal in the drawer.
    pub(crate) fn on_file_drop(
        &mut self,
        pane_id: Option<&str>,
        session_id: &str,
        paths: &gpui::ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.chat_secondary_state(session_id, cx).secondary {
            return;
        }
        let Some(text) = runner_app::terminal_paste::dropped_paths_text(paths) else {
            return;
        };
        match pane_id {
            Some(pane_id) => self.focus_terminal(pane_id, session_id, window, cx),
            None => self.focus_drawer_terminal(session_id, window, cx),
        }
        let Some(chat) = self.attached.get(session_id) else {
            return;
        };
        if let Err(error) = chat.terminal.paste(&text) {
            self.chat_error = Some(error.to_string().into());
        }
        cx.notify();
    }

    pub(crate) fn resume_chat(
        &mut self,
        pane_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote = self.front_page_client(cx).is_remote();
        if !remote && self.chat_secondary_state(session_id, cx).secondary {
            return;
        }
        if !remote && self.chat_transitions.contains_key(session_id) {
            return;
        }
        let layout = self.front_page_layout().cloned();
        if layout.is_none() && !remote {
            self.chat_error = Some(crate::i18n::message("ui.no_active_tab"));
            cx.notify();
            return;
        }
        let size = if remote {
            let size = self
                .remote_client
                .remote_terminal_size(session_id)
                .or_else(|| {
                    layout
                        .as_ref()
                        .map(|layout| self.estimated_terminal_size(layout, pane_id, window, cx))
                })
                .unwrap_or((INITIAL_COLS, INITIAL_ROWS));
            (size.0.max(20), size.1.max(8))
        } else {
            self.attached
                .get(session_id)
                .map(|chat| chat.terminal.size())
                .unwrap_or_else(|| {
                    self.estimated_terminal_size(layout.as_ref().unwrap(), pane_id, window, cx)
                })
        };
        if !remote {
            self.begin_chat_transition(
                session_id,
                chat_lifecycle::TransitionKind::Resuming,
                None,
                window,
                cx,
            );
            self.chat_error = None;
            self.app_store.update(cx, |store, store_cx| {
                store.remove_session_activity(session_id, store_cx)
            });
            self.session_exit_codes.remove(session_id);
        }
        let Some(operation) =
            WorkspaceAction::prepare(self, format!("session:{session_id}"), &["resume"], cx)
        else {
            return;
        };
        let core = operation.client.clone();
        let target = session_id.to_owned();
        let resume_target = target.clone();
        let resume = cx.background_spawn(async move {
            core.resume(&resume_target, size)
                .map_err(|error| error.to_string())
        });
        let origin_epoch = self.remote_client.ui.epoch;
        cx.spawn_in(window, async move |weak, cx| {
            let result = resume.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let Some(result) = WorkspaceAction::complete(
                    this,
                    operation,
                    result,
                    |session| crate::surfaces::front_page::actions::Completion {
                        session: Some(session),
                        ..Default::default()
                    },
                    cx,
                ) else {
                    return;
                };
                let result = result.map(drop);
                let layout = layout.as_ref().unwrap();
                if this.remote_client.ui.epoch != origin_epoch {
                    this.chat_transitions.remove(&target);
                    cx.notify();
                    return;
                }
                match result {
                    Ok(()) => {
                        this.refresh_sessions(cx);
                        if let Err(error) = this.ensure_attached(layout, &target, window, cx) {
                            this.chat_transitions.remove(&target);
                            this.chat_error = Some(error.to_string().into());
                        }
                        this.sync_active_chat_detail(cx);
                    }
                    Err(error) => {
                        this.chat_transitions.remove(&target);
                        this.chat_error = Some(error.into());
                        this.refresh_sessions(cx);
                        this.sync_active_chat_detail(cx);
                        if this.route == AppRoute::Chat
                            && this.active_focused_session_id().as_deref() == Some(target.as_str())
                        {
                            this.chat_focus.focus(window, cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn resume_drawer_shell_on_launch(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resume_drawer_shell_impl(session_id, true, window, cx);
    }

    pub(super) fn resume_drawer_shell_impl(
        &mut self,
        session_id: &str,
        launch_claim: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote = self.front_page_client(cx).is_remote();
        if !remote && self.chat_transitions.contains_key(session_id) {
            return;
        }
        let layout = self.front_page_layout().cloned();
        if layout.is_none() && !remote {
            return;
        }
        let size = if remote {
            let size = self
                .remote_client
                .remote_terminal_size(session_id)
                .unwrap_or((INITIAL_COLS, INITIAL_ROWS));
            (size.0.max(20), size.1.max(8))
        } else {
            self.attached
                .get(session_id)
                .map(|chat| chat.terminal.size())
                .unwrap_or_else(|| {
                    self.estimated_drawer_terminal_size(layout.as_ref().unwrap(), window, cx)
                })
        };
        if !remote {
            self.begin_chat_transition(
                session_id,
                chat_lifecycle::TransitionKind::Resuming,
                None,
                window,
                cx,
            );
            self.chat_error = None;
            self.session_exit_codes.remove(session_id);
        }
        let Some(operation) =
            WorkspaceAction::prepare(self, format!("session:{session_id}"), &["resume"], cx)
        else {
            return;
        };
        let core = operation.client.clone();
        let target = session_id.to_owned();
        let resume_target = target.clone();
        let resume = cx.background_spawn(async move {
            core.resume_session(&resume_target, size, launch_claim)
                .map_err(|error| error.to_string())
        });
        let origin_epoch = self.remote_client.ui.epoch;
        cx.spawn_in(window, async move |weak, cx| {
            let result = resume.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let Some(result) = WorkspaceAction::complete(
                    this,
                    operation,
                    result,
                    |session| crate::surfaces::front_page::actions::Completion {
                        session: Some(session),
                        ..Default::default()
                    },
                    cx,
                ) else {
                    return;
                };
                let result = result.map(drop);
                let layout = layout.as_ref().unwrap();
                if this.remote_client.ui.epoch != origin_epoch {
                    this.chat_transitions.remove(&target);
                    cx.notify();
                    return;
                }
                match result {
                    Ok(()) => {
                        this.refresh_sessions(cx);
                        if let Err(error) = this.ensure_attached(layout, &target, window, cx) {
                            this.chat_transitions.remove(&target);
                            this.chat_error = Some(error.to_string().into());
                        }
                        this.focus_drawer_terminal(&target, window, cx);
                    }
                    Err(error) => {
                        this.chat_transitions.remove(&target);
                        this.chat_error = Some(error.into());
                        this.refresh_sessions(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
