//! Chat lifecycle and input: attaching sessions to terminals, tab
//! activation/focus, key/scroll/paste routing, chat start/resume, and
//! split resizing.
use super::*;
use crate::*;

#[derive(Clone, Debug, Eq, PartialEq)]
enum PaneRenameChange {
    Unchanged,
    Persist(Option<String>),
}

fn pane_rename_change(original: &str, value: &str) -> PaneRenameChange {
    let value = value.trim();
    if value == original.trim() {
        PaneRenameChange::Unchanged
    } else if value.is_empty() {
        PaneRenameChange::Persist(None)
    } else {
        PaneRenameChange::Persist(Some(value.to_owned()))
    }
}

impl NativeRoot {
    pub(crate) fn open_archived_chat(
        &mut self,
        chat: DirectSessionEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_sidebar_transients(cx);
        let session_key = chat.agent_session_key.clone();
        self.archived_chat_detail = Some(chat);
        self.archived_session_key_copy
            .update(cx, |copy, copy_cx| copy.set_value(session_key, copy_cx));
        self.set_route(AppRoute::ArchivedChat, cx);
        self.chat_focus.focus(window);
        cx.notify();
    }

    pub(crate) fn handle_chat_lifecycle_event(
        &mut self,
        event: runner_backend::events::AppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The sidebar also refreshes exit/updated rows; repeat it here so chat-local
        // lifecycle state and row data land together regardless of subscriber order.
        let session_id = event
            .payload
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        match event.name {
            "session/exit" => {
                if let Some(session_id) = session_id {
                    self.attached.remove(&session_id);
                    let exit_code = event
                        .payload
                        .get("exit_code")
                        .and_then(serde_json::Value::as_i64)
                        .and_then(|code| i32::try_from(code).ok());
                    self.session_exit_codes
                        .insert(session_id.clone(), exit_code);
                    self.chat_transitions.remove(&session_id);
                    self.stopping_sessions.remove(&session_id);
                    self.refresh_sessions(cx);
                    self.sync_active_chat_detail(cx);
                    if self.route == AppRoute::Chat
                        && self.active_focused_session_id().as_deref() == Some(session_id.as_str())
                    {
                        self.chat_focus.focus(window);
                    }
                }
            }
            "session/archived" => {
                if let Some(session_id) = session_id {
                    self.attached.remove(&session_id);
                    self.session_exit_codes.remove(&session_id);
                    self.chat_transitions.remove(&session_id);
                    self.stopping_sessions.remove(&session_id);
                    self.refresh_sessions(cx);
                    self.sync_active_chat_detail(cx);
                }
            }
            "session/spawned" => {
                if let Some(session_id) = session_id {
                    self.attached.remove(&session_id);
                    if let Some(transition) = self.chat_transitions.get_mut(&session_id) {
                        transition.baseline_seq = 0;
                    }
                    self.refresh_sessions(cx);
                    if let Some(layout) =
                        self.tabs.active().cloned().filter(|layout| {
                            layout.session_ids().iter().any(|id| id == &session_id)
                        })
                    {
                        if let Err(error) = self.ensure_attached(&layout, &session_id, window, cx) {
                            self.chat_error = Some(error.to_string());
                        }
                    }
                    self.sync_active_chat_detail(cx);
                }
            }
            "session/warning" => {
                let direct = event
                    .payload
                    .get("mission_id")
                    .is_some_and(serde_json::Value::is_null);
                let visible = session_id.as_deref().is_some_and(|session_id| {
                    self.tabs.active().is_some_and(|layout| {
                        layout.session_ids().iter().any(|id| id == session_id)
                    })
                });
                if direct && visible {
                    self.chat_warning = event
                        .payload
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                }
            }
            "session/updated" => {
                self.refresh_sessions(cx);
                self.sync_active_chat_detail(cx);
            }
            _ => {}
        }
    }

    pub(crate) fn sync_active_chat_detail(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.active_focused_session_id() else {
            return;
        };
        self.active_chat_detail =
            runner_backend::ops::session::session_get(self.core(cx), &session_id)
                .ok()
                .flatten();
        let session_key = self
            .active_chat_detail
            .as_ref()
            .and_then(|entry| entry.agent_session_key.clone());
        self.session_key_copy
            .update(cx, |copy, copy_cx| copy.set_value(session_key, copy_cx));
    }

    pub(crate) fn ensure_active_chat_detail(&mut self, cx: &mut Context<Self>) {
        let active = self.active_focused_session_id();
        let loaded = self
            .active_chat_detail
            .as_ref()
            .map(|entry| entry.session_id.as_str());
        if active.as_deref() != loaded {
            self.sync_active_chat_detail(cx);
        }
    }

    pub(crate) fn begin_chat_transition(
        &mut self,
        session_id: &str,
        kind: chat_lifecycle::TransitionKind,
        baseline_seq: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.next_chat_transition_generation += 1;
        let generation = self.next_chat_transition_generation;
        let baseline_seq = baseline_seq.unwrap_or_else(|| {
            self.attached
                .get(session_id)
                .map(|chat| chat.terminal.output_activity().last_seq)
                .unwrap_or(0)
        });
        self.chat_transitions.insert(
            session_id.to_owned(),
            ChatTransition {
                kind,
                started_at: Instant::now(),
                baseline_seq,
                generation,
            },
        );
        if self.route == AppRoute::Chat
            && self.active_focused_session_id().as_deref() == Some(session_id)
        {
            self.chat_focus.focus(window);
        }
        let tracked_id = session_id.to_owned();
        cx.spawn_in(window, async move |weak, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let done = weak
                .update_in(cx, |this, window, cx| {
                    let Some(transition) = this.chat_transitions.get(&tracked_id).copied() else {
                        return true;
                    };
                    if transition.generation != generation {
                        return true;
                    }
                    let now = Instant::now();
                    let activity = this
                        .attached
                        .get(&tracked_id)
                        .map(|chat| chat.terminal.output_activity());
                    let output_seen = activity
                        .is_some_and(|activity| activity.last_seq > transition.baseline_seq);
                    let first_paint_seen = activity
                        .is_some_and(|activity| activity.first_paint_seq > transition.baseline_seq);
                    let tui_ready_seen = activity
                        .is_some_and(|activity| activity.tui_ready_seq > transition.baseline_seq);
                    let output_idle_for = activity
                        .and_then(|activity| activity.last_output_at)
                        .map(|last| now.saturating_duration_since(last));
                    let settled = chat_lifecycle::transition_should_settle(
                        transition.kind,
                        now.saturating_duration_since(transition.started_at),
                        first_paint_seen,
                        tui_ready_seen,
                        output_seen,
                        output_idle_for,
                    );
                    if settled {
                        this.chat_transitions.remove(&tracked_id);
                        if this.route == AppRoute::Chat
                            && this.active_focused_session_id().as_deref()
                                == Some(tracked_id.as_str())
                        {
                            this.focus_active_terminal(window, cx);
                        }
                        cx.notify();
                    }
                    settled
                })
                .unwrap_or(true);
            if done {
                break;
            }
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn stop_chat(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.chat_secondary_state(session_id, cx).secondary {
            return;
        }
        if !self.stopping_sessions.insert(session_id.to_owned()) {
            return;
        }
        self.chat_error = None;
        self.app_store.update(cx, |store, store_cx| {
            store.remove_session_activity(session_id, store_cx)
        });
        if self.route == AppRoute::Chat
            && self.active_focused_session_id().as_deref() == Some(session_id)
        {
            self.chat_focus.focus(window);
        }
        cx.notify();
        let core = self.core(cx).clone();
        let target = session_id.to_owned();
        let stop_target = target.clone();
        let stop = cx.background_spawn(async move {
            runner_backend::ops::session::session_kill(&core, &stop_target)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = stop.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                this.stopping_sessions.remove(&target);
                this.chat_transitions.remove(&target);
                this.refresh_sessions(cx);
                this.sync_active_chat_detail(cx);
                match result {
                    Ok(()) => {}
                    Err(error) => this.chat_error = Some(error),
                }
                if this.route == AppRoute::Chat
                    && this.active_focused_session_id().as_deref() == Some(target.as_str())
                {
                    this.chat_focus.focus(window);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn stop_focused_session(
        &mut self,
        _: &StopFocusedSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route != AppRoute::Chat {
            return;
        }
        let Some(session_id) = self.active_focused_session_id() else {
            return;
        };
        if self
            .session_entry(&session_id, cx)
            .is_some_and(|entry| entry.status == SessionStatus::Running)
        {
            self.stop_chat(&session_id, window, cx);
        }
    }

    pub(crate) fn stop_chats(
        &mut self,
        session_ids: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for session_id in session_ids {
            if self
                .session_entry(&session_id, cx)
                .is_some_and(|entry| entry.status == SessionStatus::Running)
            {
                self.stop_chat(&session_id, window, cx);
            }
        }
    }

    pub(crate) fn resume_chats(
        &mut self,
        session_ids: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane_ids = self
            .tabs
            .active()
            .map(|layout| {
                layout
                    .root
                    .leaves()
                    .into_iter()
                    .filter_map(|leaf| {
                        leaf.session_id
                            .as_ref()
                            .map(|session_id| (session_id.clone(), leaf.id.clone()))
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        for session_id in session_ids {
            let resumable = self
                .session_entry(&session_id, cx)
                .is_some_and(|entry| entry.status != SessionStatus::Running)
                && !self.chat_transitions.contains_key(&session_id);
            if resumable {
                if let Some(pane_id) = pane_ids.get(&session_id) {
                    self.resume_chat(pane_id, &session_id, window, cx);
                }
            }
        }
    }

    pub(crate) fn toggle_chat_pin(
        &mut self,
        session_id: String,
        pinned: bool,
        cx: &mut Context<Self>,
    ) {
        let next = !pinned;
        self.app_store.update(cx, |store, store_cx| {
            store.set_session_pinned(&session_id, next, store_cx)
        });
        if let Some(detail) = self
            .active_chat_detail
            .as_mut()
            .filter(|entry| entry.session_id == session_id)
        {
            detail.pinned = next;
        }
        self.chat_error = None;
        cx.notify();
        let core = self.core(cx).clone();
        let target = session_id.clone();
        let pin = cx.background_spawn(async move {
            runner_backend::ops::session::session_pin(&core, &target, next)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = pin.await;
            let _ = weak.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.chat_error = Some(error);
                    this.refresh_sessions(cx);
                }
                this.sync_active_chat_detail(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn handle_chat_menu_action(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .active_focused_session_id()
            .is_some_and(|session_id| self.chat_secondary_state(&session_id, cx).secondary)
        {
            return;
        }
        let Some(action) = self.chat_menu_actions.get(index).cloned() else {
            return;
        };
        match action {
            ChatMenuAction::TogglePin { session_id, pinned } => {
                self.toggle_chat_pin(session_id, pinned, cx)
            }
            ChatMenuAction::RenameSession {
                session_id,
                current,
            } => self.open_chat_rename(
                ChatRenameTarget::Session {
                    session_id,
                    original: current,
                },
                window,
                cx,
            ),
            ChatMenuAction::RenameTab { tab_id, current } => self.open_chat_rename(
                ChatRenameTarget::Tab {
                    tab_id,
                    original: current,
                },
                window,
                cx,
            ),
            ChatMenuAction::Archive(session_ids) => {
                self.archive_chat_sessions(session_ids, window, cx)
            }
            ChatMenuAction::ArchiveAll(session_ids) => {
                self.request_archive_all(None, session_ids, ArchiveAllSource::Chat, window, cx)
            }
        }
    }

    fn open_chat_rename(
        &mut self,
        target: ChatRenameTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let original = match &target {
            ChatRenameTarget::Session { original, .. } | ChatRenameTarget::Tab { original, .. } => {
                original.clone()
            }
        };
        let placeholder = match &target {
            ChatRenameTarget::Session { session_id, .. }
                if self
                    .session_entry(session_id, cx)
                    .is_some_and(|entry| entry.agent_runtime == "shell") =>
            {
                "Terminal name"
            }
            ChatRenameTarget::Session { .. } => "Chat name",
            ChatRenameTarget::Tab { .. } => "Tab name",
        };
        let input = cx.new(|input_cx| {
            runner_app::ui::TextField::new(input_cx.focus_handle(), original, placeholder, false)
                .text_size(13.)
        });
        input.update(cx, |input, input_cx| input.select_all(input_cx));
        let input_focus = input.read(cx).focus_handle();
        self.chat_rename_modal = Some(ChatRenameModal {
            target,
            input,
            close_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            submitting: false,
            error: None,
        });
        input_focus.focus(window);
        cx.notify();
    }

    pub(crate) fn close_chat_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .chat_rename_modal
            .as_ref()
            .is_some_and(|modal| modal.submitting)
        {
            return;
        }
        self.chat_rename_modal = None;
        self.focus_active_terminal(window, cx);
        cx.notify();
    }

    pub(crate) fn submit_chat_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.chat_rename_modal.as_mut() else {
            return;
        };
        if modal.submitting || modal.input.read(cx).is_composing() {
            return;
        }
        let next = modal.input.read(cx).text().trim().to_owned();
        let target = modal.target.clone();
        let unchanged = match &target {
            ChatRenameTarget::Session { original, .. } | ChatRenameTarget::Tab { original, .. } => {
                next == original.trim()
            }
        };
        if unchanged {
            self.close_chat_rename(window, cx);
            return;
        }
        modal.submitting = true;
        modal.error = None;
        cx.notify();

        let core = self.core(cx).clone();
        let rename = cx.background_spawn(async move {
            match target {
                ChatRenameTarget::Session { session_id, .. } => {
                    runner_backend::ops::session::session_rename(&core, &session_id, Some(next))
                        .map(drop)
                }
                ChatRenameTarget::Tab { tab_id, .. } => {
                    runner_backend::ops::node::node_rename(&core, tab_id, next).map(drop)
                }
            }
            .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = rename.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.chat_rename_modal = None;
                        this.refresh_sessions(cx);
                        if let Err(error) = this.reload_tabs(cx) {
                            this.chat_error = Some(error.to_string());
                        }
                        this.sync_active_chat_detail(cx);
                        this.focus_active_terminal(window, cx);
                    }
                    Err(error) => {
                        if let Some(modal) = this.chat_rename_modal.as_mut() {
                            modal.submitting = false;
                            modal.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn begin_pane_rename(
        &mut self,
        session_id: String,
        value: String,
        placeholder: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|input_cx| {
            runner_app::ui::TextField::new(
                input_cx.focus_handle(),
                value.clone(),
                placeholder,
                false,
            )
            .text_size(12.)
        });
        input.update(cx, |field, input_cx| {
            field.set_bare(true, input_cx);
            field.set_right_padding(0., input_cx);
            field.select_all(input_cx);
        });
        let focus = input.read(cx).focus_handle();
        let root = cx.entity();
        self._pane_rename_focus_subscription =
            Some(cx.on_focus_out(&focus, window, move |_, _, window, cx| {
                root.update(cx, |this, cx| this.submit_pane_rename(window, cx));
            }));
        self.pane_rename = Some(PaneRename {
            session_id,
            original: value,
            input,
        });
        focus.focus(window);
        cx.notify();
    }

    pub(crate) fn submit_pane_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.pane_rename.as_ref() else {
            return;
        };
        if rename.input.read(cx).is_composing() {
            return;
        }
        let session_id = rename.session_id.clone();
        let change = pane_rename_change(&rename.original, rename.input.read(cx).text());
        let result = match change {
            PaneRenameChange::Unchanged => Ok(()),
            PaneRenameChange::Persist(title) => {
                runner_backend::ops::session::session_rename(self.core(cx), &session_id, title)
                    .map(drop)
            }
        };
        match result {
            Ok(()) => {
                self.pane_rename = None;
                self._pane_rename_focus_subscription = None;
                self.refresh_sessions(cx);
                if let Err(error) = self.reload_tabs(cx) {
                    self.chat_error = Some(error.to_string());
                }
                self.sync_active_chat_detail(cx);
                self.focus_active_terminal(window, cx);
            }
            Err(error) => {
                self.chat_error = Some(error.to_string());
                if let Some(rename) = self.pane_rename.as_ref() {
                    rename.input.read(cx).focus_handle().focus(window);
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn cancel_pane_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pane_rename = None;
        self._pane_rename_focus_subscription = None;
        self.focus_active_terminal(window, cx);
        cx.notify();
    }

    pub(crate) fn on_chat_rename_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "enter"
            && self
                .chat_rename_modal
                .as_ref()
                .is_some_and(|modal| !modal.input.read(cx).is_composing())
        {
            cx.stop_propagation();
            self.submit_chat_rename(window, cx);
        }
    }

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
        if self.closing {
            return Ok(());
        }
        self.refresh_chat_secondaries(cx);
        let Some(layout) = self.tabs.active().cloned() else {
            self.attached.clear();
            return Ok(());
        };
        let active_ids = layout.session_ids();
        let owned_ids = active_ids
            .iter()
            .filter(|session_id| !self.chat_secondaries.contains_key(*session_id))
            .cloned()
            .collect::<HashSet<_>>();
        self.attached
            .retain(|session_id, _| owned_ids.contains(session_id));
        let mut errors = Vec::new();
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

    pub(crate) fn ensure_attached(
        &mut self,
        _layout: &PaneLayout,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if self.chat_secondary_state(session_id, cx).secondary {
            return Ok(());
        }
        let status = self
            .session_entry(session_id, cx)
            .map(|entry| entry.status)
            .with_context(|| format!("direct chat not found: {session_id}"))?;
        if status != SessionStatus::Running {
            self.attached.remove(session_id);
            return Ok(());
        }
        if self.attached.contains_key(session_id) {
            return Ok(());
        }

        let Some(terminal) = self.app_store.read(cx).bridge.session(session_id) else {
            return Ok(());
        };
        terminal.set_palette(self.settings(cx).terminal_theme.palette());
        terminal.configure(
            app_settings::TERMINAL_SCROLLBACK_LINES,
            match self.settings(cx).terminal_cursor_style {
                app_settings::TerminalCursorStyle::Block => {
                    alacritty_terminal::vte::ansi::CursorShape::Block
                }
                app_settings::TerminalCursorStyle::Underline => {
                    alacritty_terminal::vte::ansi::CursorShape::Underline
                }
                app_settings::TerminalCursorStyle::Bar => {
                    alacritty_terminal::vte::ansi::CursorShape::Beam
                }
            },
        );
        let terminal_scrollbar = cx.new(|_| Scrollbar::terminal(Arc::clone(&terminal)));
        let terminal_interaction = cx.new(|_| TerminalInteraction::new(Arc::clone(&terminal)));
        let terminal_focus = cx.focus_handle();
        let terminal_input = cx.new(|_| TerminalInput::new(Arc::clone(&terminal)));
        let terminal_input_subscription = cx.observe(&terminal_input, |this, input, cx| {
            if let Some(result) = input.update(cx, |input, _| input.take_write_result()) {
                match result {
                    Ok(()) => this.chat_error = None,
                    Err(error) => this.chat_error = Some(error),
                }
            }
            cx.notify();
        });
        let terminal_input_on_focus_out = terminal_input.clone();
        let terminal_focus_subscription =
            cx.on_focus_out(&terminal_focus, window, move |_, _, window, cx| {
                terminal_input_on_focus_out.update(cx, |input, input_cx| {
                    if input.cancel_composition() {
                        window.invalidate_character_coordinates();
                        input_cx.notify();
                    }
                });
            });
        self.attached.insert(
            session_id.to_owned(),
            AttachedChat {
                _terminal_view: terminal.view(),
                terminal,
                terminal_interaction,
                terminal_scrollbar,
                terminal_input,
                _terminal_input_subscription: terminal_input_subscription,
                _terminal_focus_subscription: terminal_focus_subscription,
                terminal_focus,
                scroll_accumulator: 0.,
            },
        );
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
        let sidebar_width = if self.sidebar_collapsed {
            0.
        } else {
            self.settings(cx).sidebar_width * self.settings(cx).app_zoom
        };
        let chat_panel_width = if self.settings(cx).chat_panel_open {
            self.settings(cx).chat_panel_width * self.settings(cx).app_zoom
        } else {
            0.
        };
        let pane_width =
            (f32::from(bounds.width) - sidebar_width - chat_panel_width).max(200.) * width_fraction;
        let grouped = layout.root.leaves().len() > 1;
        let pane_height = (f32::from(bounds.height)
            - WORKSPACE_HEADER_HEIGHT * self.settings(cx).app_zoom)
            .max(160.)
            * height_fraction
            - if grouped {
                PANE_HEADER_HEIGHT * self.settings(cx).app_zoom
            } else {
                0.
            };
        let font_size = self.settings(cx).terminal_font_size as f32 * self.settings(cx).app_zoom;
        let cell_width = font_size * 0.6;
        let line_height = (font_size * crate::terminal::element::LINE_HEIGHT_FACTOR).round();
        (
            (pane_width / cell_width).floor().max(2.) as u16,
            (pane_height / line_height).floor().max(2.) as u16,
        )
    }

    pub(crate) fn active_focused_session_id(&self) -> Option<String> {
        self.tabs
            .active()
            .and_then(PaneLayout::focused_session_id)
            .map(str::to_owned)
    }

    pub(crate) fn session_lifecycle_disabled(&self, session_id: &str, cx: &App) -> bool {
        self.stopping_sessions.contains(session_id)
            || self.sidebar_archiving_session(session_id, cx)
            || self.chat_transitions.contains_key(session_id)
    }

    pub(crate) fn session_is_interactive(&self, session_id: &str, cx: &App) -> bool {
        self.route == AppRoute::Chat
            && !self.chat_secondary_state(session_id, cx).secondary
            && self
                .session_entry(session_id, cx)
                .is_some_and(|entry| entry.status == SessionStatus::Running)
            && !self.session_lifecycle_disabled(session_id, cx)
    }

    pub(crate) fn cached_session_is_interactive(&self, session_id: &str, cx: &App) -> bool {
        self.route == AppRoute::Chat
            && !self.cached_chat_secondary_state(session_id).secondary
            && self
                .session_entry(session_id, cx)
                .is_some_and(|entry| entry.status == SessionStatus::Running)
            && !self.session_lifecycle_disabled(session_id, cx)
    }

    pub(crate) fn focus_active_terminal(&self, window: &mut Window, cx: &App) {
        let Some(session_id) = self.active_focused_session_id() else {
            self.chat_focus.focus(window);
            return;
        };
        if self.session_is_interactive(&session_id, cx) {
            if let Some(chat) = self.attached.get(&session_id) {
                chat.terminal_focus.focus(window);
                return;
            }
        }
        self.chat_focus.focus(window);
    }

    pub(crate) fn activate_tab(
        &mut self,
        tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.tabs.activate(tab_id) {
            return;
        }
        self.sync_active_project_from_active_tab(cx);
        self.layout_picker_open = false;
        match self.ensure_active_tab_attached(window, cx) {
            Ok(()) => {
                self.chat_error = None;
                self.remember_active_runner(cx);
                self.sync_active_chat_detail(cx);
                self.mark_active_tab_viewed(window, cx);
                self.focus_active_terminal(window, cx);
            }
            Err(error) => self.chat_error = Some(error.to_string()),
        }
        cx.notify();
    }

    pub(crate) fn focus_pane(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        let runner_id = self.tabs.active().and_then(|layout| {
            layout
                .root
                .leaves()
                .into_iter()
                .find(|leaf| leaf.id == pane_id)
                .and_then(|leaf| leaf.session_id.as_deref())
                .map(|session_id| {
                    self.session_entry(session_id, cx)
                        .and_then(|entry| entry.runner_id.clone())
                })
        });
        if self
            .tabs
            .active_mut()
            .is_some_and(|layout| layout.focus_pane(pane_id))
        {
            if let Some(runner_id) = runner_id {
                self.last_focused_runner_id = runner_id;
            }
            self.sync_active_chat_detail(cx);
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

    fn cycle_chat_pane(&mut self, direction: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.route != AppRoute::Chat {
            return;
        }
        let Some(layout) = self.tabs.active() else {
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
        self.focus_pane(pane_id, cx);
        self.last_focused_runner_id = self
            .session_entry(session_id, cx)
            .and_then(|entry| entry.runner_id.clone());
        if self.session_is_interactive(session_id, cx) {
            if let Some(chat) = self.attached.get(session_id) {
                chat.terminal_focus.focus(window);
            } else {
                self.chat_focus.focus(window);
            }
        } else {
            self.chat_focus.focus(window);
        }
        self.mark_active_tab_viewed(window, cx);
        self.record_current_runtime_location();
    }

    pub(crate) fn on_key_down(
        &mut self,
        session_id: &str,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.session_is_interactive(session_id, cx) {
            return;
        }
        let Some(chat) = self.attached.get(session_id) else {
            return;
        };
        let keystroke = &event.keystroke;
        if runner_app::terminal_ime::swallows_option_copy(
            keystroke.modifiers.platform,
            keystroke.modifiers.control,
            keystroke.modifiers.alt,
            &keystroke.key,
            keystroke.key_char.as_deref(),
        ) {
            cx.stop_propagation();
            return;
        }
        if runner_app::terminal_ime::terminal_key_route(
            chat.terminal_input.read(cx).is_composing(),
            keystroke.modifiers.platform,
            keystroke.modifiers.control,
            keystroke.modifiers.alt,
            keystroke.modifiers.function,
            &keystroke.key,
        ) != runner_app::terminal_ime::TerminalKeyRoute::Raw
        {
            return;
        }
        match chat.terminal.send_key(
            &keystroke.key,
            keystroke.modifiers.control,
            keystroke.modifiers.alt,
            keystroke.modifiers.shift,
            keystroke.key_char.as_deref(),
        ) {
            Ok(true) => {
                chat.terminal.scroll_to_bottom();
                self.chat_error = None;
                cx.stop_propagation();
                cx.notify();
            }
            Ok(false) => {}
            Err(error) => {
                self.chat_error = Some(error.to_string());
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
        let Some(text) = self
            .attached
            .get(session_id)
            .and_then(|chat| chat.terminal.selection_text())
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        cx.stop_propagation();
    }

    pub(crate) fn on_scroll(
        &mut self,
        session_id: &str,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.attached.get_mut(session_id) else {
            return;
        };
        let lines = match event.delta {
            ScrollDelta::Lines(point) => point.y,
            ScrollDelta::Pixels(point) => f32::from(point.y) / f32::from(window.line_height()),
        };
        chat.scroll_accumulator += lines;
        let whole = chat.scroll_accumulator.trunc() as i32;
        if whole != 0 {
            chat.scroll_accumulator -= whole as f32;
            chat.terminal.scroll(whole, event.modifiers.shift);
            cx.notify();
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
        let Some(paste) = runner_app::terminal_paste::resolve_terminal_paste(
            item.as_ref(),
            runner_backend::ops::session::session_clipboard_file_paths,
        ) else {
            return;
        };
        match paste {
            runner_app::terminal_paste::TerminalPaste::Image(image) => {
                let Some(terminal) = self
                    .attached
                    .get(session_id)
                    .map(|chat| Arc::clone(&chat.terminal))
                else {
                    return;
                };
                let paste = cx.background_spawn(async move {
                    runner_backend::ops::session::session_paste_image(
                        image.bytes,
                        image.format.mime_type(),
                    )?;
                    terminal.write_user_bytes(b"\x16")
                });
                cx.spawn(async move |weak, cx| {
                    let result = paste.await;
                    let _ = weak.update(cx, |this, cx| {
                        match result {
                            Ok(()) => this.chat_error = None,
                            Err(error) => this.chat_error = Some(error.to_string()),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            runner_app::terminal_paste::TerminalPaste::Text(text) => {
                let Some(chat) = self.attached.get(session_id) else {
                    return;
                };
                if let Err(error) = chat.terminal.paste(&text) {
                    self.chat_error = Some(error.to_string());
                }
            }
        }
    }

    pub(crate) fn resume_chat(
        &mut self,
        pane_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.chat_secondary_state(session_id, cx).secondary {
            return;
        }
        if self.chat_transitions.contains_key(session_id) {
            return;
        }
        let Some(layout) = self.tabs.active().cloned() else {
            self.chat_error = Some("No active tab".into());
            cx.notify();
            return;
        };
        let size = self
            .attached
            .get(session_id)
            .map(|chat| chat.terminal.size())
            .unwrap_or_else(|| self.estimated_terminal_size(&layout, pane_id, window, cx));
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
        let core = self.core(cx).clone();
        let target = session_id.to_owned();
        let resume_target = target.clone();
        let resume = cx.background_spawn(async move {
            runner_backend::ops::session::session_resume(
                &core,
                &resume_target,
                Some(size.0),
                Some(size.1),
            )
            .map(drop)
            .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = resume.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.refresh_sessions(cx);
                        if let Err(error) = this.ensure_attached(&layout, &target, window, cx) {
                            this.chat_transitions.remove(&target);
                            this.chat_error = Some(error.to_string());
                        }
                        this.sync_active_chat_detail(cx);
                    }
                    Err(error) => {
                        this.chat_transitions.remove(&target);
                        this.chat_error = Some(error);
                        this.refresh_sessions(cx);
                        this.sync_active_chat_detail(cx);
                        if this.route == AppRoute::Chat
                            && this.active_focused_session_id().as_deref() == Some(target.as_str())
                        {
                            this.chat_focus.focus(window);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn pick_preset(
        &mut self,
        preset: PresetKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = (|| -> Result<()> {
            let Some(layout) = self.tabs.active_mut() else {
                return Ok(());
            };
            layout.apply_preset(preset);
            self.persist_active_tab(cx)?;
            self.reload_tabs(cx)?;
            self.ensure_active_tab_attached(window, cx)?;
            Ok(())
        })();
        self.layout_picker_open = false;
        match result {
            Ok(()) => {
                self.chat_error = None;
                self.remember_active_runner(cx);
                self.mark_active_tab_viewed(window, cx);
                self.focus_active_terminal(window, cx);
            }
            Err(error) => self.chat_error = Some(error.to_string()),
        }
        cx.notify();
    }

    pub(crate) fn split_pane_right(
        &mut self,
        _: &SplitPaneRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_pane(SplitOrientation::Row, window, cx);
    }

    pub(crate) fn split_pane_down(
        &mut self,
        _: &SplitPaneDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_pane(SplitOrientation::Column, window, cx);
    }

    fn split_pane(
        &mut self,
        orientation: SplitOrientation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route != AppRoute::Chat {
            return;
        }
        match self
            .tabs
            .active()
            .context("active tab is missing")
            .and_then(|layout| layout.next_split_preset(orientation))
        {
            Ok(preset) => self.pick_preset(preset, window, cx),
            Err(error) => {
                self.chat_error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    pub(crate) fn persist_active_tab(&self, cx: &App) -> Result<()> {
        let input = self
            .tabs
            .active()
            .context("active tab is missing")?
            .upsert_input()?;
        runner_backend::ops::node::node_tab_upsert(self.core(cx), input)?;
        Ok(())
    }

    pub(crate) fn close_pane(
        &mut self,
        pane_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let closing_session_id = self.tabs.active().and_then(|layout| {
            layout
                .root
                .leaves()
                .into_iter()
                .find(|leaf| leaf.id == pane_id)
                .and_then(|leaf| leaf.session_id.clone())
        });
        let result = (|| -> Result<bool> {
            let Some(layout) = self.tabs.active_mut() else {
                return Ok(false);
            };
            if !layout.close_pane(pane_id) {
                return Ok(false);
            }
            self.persist_active_tab(cx)?;
            self.reload_tabs(cx)?;
            self.ensure_active_tab_attached(window, cx)?;
            Ok(true)
        })();
        match result {
            Ok(true) => {
                if self.pane_rename.as_ref().is_some_and(|rename| {
                    closing_session_id.as_deref() == Some(rename.session_id.as_str())
                }) {
                    self.pane_rename = None;
                    self._pane_rename_focus_subscription = None;
                }
                self.chat_error = None;
                self.remember_active_runner(cx);
                self.mark_active_tab_viewed(window, cx);
                self.focus_active_terminal(window, cx);
                self.record_current_runtime_location();
            }
            Ok(false) => {}
            Err(error) => self.chat_error = Some(error.to_string()),
        }
        cx.notify();
    }

    pub(crate) fn close_terminal_pane(
        &mut self,
        pane_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.terminal_close_confirm = None;
        if !self.stopping_sessions.insert(session_id.to_owned()) {
            return;
        }
        self.chat_error = None;
        let core = self.core(cx).clone();
        let target = session_id.to_owned();
        let close_target = target.clone();
        let pane_id = pane_id.to_owned();
        let close = cx.background_spawn(async move {
            runner_backend::ops::session::session_close(&core, &close_target)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = close.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                this.stopping_sessions.remove(&target);
                match result {
                    Ok(()) => {
                        this.attached.remove(&target);
                        this.session_exit_codes.remove(&target);
                        this.refresh_sessions(cx);
                        this.close_pane(&pane_id, window, cx);
                    }
                    Err(error) => this.chat_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn request_close_terminal_pane(
        &mut self,
        pane_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match runner_backend::ops::session::session_shell_has_foreground_process(
            self.core(cx),
            session_id,
        ) {
            Ok(true) => {
                self.terminal_close_confirm = Some(TerminalCloseConfirm {
                    target: TerminalCloseTarget::Pane {
                        pane_id: pane_id.to_owned(),
                        session_id: session_id.to_owned(),
                    },
                });
                cx.notify();
            }
            Ok(false) => self.close_terminal_pane(pane_id, session_id, window, cx),
            Err(error) => {
                self.chat_error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    pub(crate) fn request_close_terminal_tab(
        &mut self,
        tab_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match runner_backend::ops::session::session_shell_has_foreground_process(
            self.core(cx),
            session_id,
        ) {
            Ok(true) => {
                self.tabs.activate(tab_id);
                self.set_route(AppRoute::Chat, cx);
                if let Err(error) = self.ensure_active_tab_attached(window, cx) {
                    self.chat_error = Some(error.to_string());
                }
                self.terminal_close_confirm = Some(TerminalCloseConfirm {
                    target: TerminalCloseTarget::Tab {
                        session_id: session_id.to_owned(),
                    },
                });
                cx.notify();
            }
            Ok(false) => self.close_terminal_tab(session_id, window, cx),
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn close_terminal_tab(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.terminal_close_confirm = None;
        if !self.stopping_sessions.insert(session_id.to_owned()) {
            return;
        }
        let core = self.core(cx).clone();
        let target = session_id.to_owned();
        let close_target = target.clone();
        let close = cx.background_spawn(async move {
            runner_backend::ops::session::session_close(&core, &close_target)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = close.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                this.stopping_sessions.remove(&target);
                match result {
                    Ok(()) => {
                        this.attached.remove(&target);
                        this.session_exit_codes.remove(&target);
                        this.refresh_sessions(cx);
                        let refresh = this.reload_tabs(cx).and_then(|_| {
                            if this.route == AppRoute::Chat {
                                this.ensure_active_tab_attached(window, cx)?;
                                this.mark_active_tab_viewed(window, cx);
                                this.focus_active_terminal(window, cx);
                                this.record_current_runtime_location();
                            }
                            Ok(())
                        });
                        if let Err(error) = refresh {
                            this.error = Some(error.to_string());
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn request_archive_all(
        &mut self,
        tab_id: Option<&str>,
        session_ids: Vec<String>,
        source: ArchiveAllSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(confirmation_body) = super::sidebar::archive_all_confirmation_body(
            &session_ids,
            &self.app_store.read(cx).sessions,
        ) else {
            self.archive_all_sessions(session_ids, source, window, cx);
            return;
        };
        if let Some(tab_id) = tab_id {
            self.tabs.activate(tab_id);
            self.set_route(AppRoute::Chat, cx);
            if let Err(error) = self.ensure_active_tab_attached(window, cx) {
                self.chat_error = Some(error.to_string());
            }
        }
        self.terminal_close_confirm = Some(TerminalCloseConfirm {
            target: TerminalCloseTarget::ArchiveAll {
                session_ids,
                source,
                confirmation_body,
            },
        });
        cx.notify();
    }

    pub(crate) fn confirm_terminal_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(confirm) = self.terminal_close_confirm.take() else {
            return;
        };
        match confirm.target {
            TerminalCloseTarget::Pane {
                pane_id,
                session_id,
            } => self.close_terminal_pane(&pane_id, &session_id, window, cx),
            TerminalCloseTarget::Tab { session_id } => {
                self.close_terminal_tab(&session_id, window, cx)
            }
            TerminalCloseTarget::ArchiveAll {
                session_ids,
                source,
                ..
            } => self.archive_all_sessions(session_ids, source, window, cx),
        }
    }

    pub(crate) fn cancel_terminal_close(&mut self, cx: &mut Context<Self>) {
        self.terminal_close_confirm = None;
        cx.notify();
    }

    pub(crate) fn resize_split(
        &mut self,
        split_id: &str,
        orientation: SplitOrientation,
        event: &DragMoveEvent<SplitResizeDrag>,
        cx: &mut Context<Self>,
    ) {
        let position = event.event.position;
        let bounds = event.bounds;
        let ratio = match orientation {
            SplitOrientation::Row => {
                f32::from(position.x - bounds.left()) / f32::from(bounds.size.width)
            }
            SplitOrientation::Column => {
                f32::from(position.y - bounds.top()) / f32::from(bounds.size.height)
            }
        }
        .clamp(0.15, 0.85)
            * 100.;
        if self
            .tabs
            .active_mut()
            .is_some_and(|layout| layout.set_split_sizes(split_id, [ratio, 100. - ratio]))
        {
            self.split_sizes_dirty = true;
            cx.notify();
        }
    }

    pub(crate) fn finish_split_resize(&mut self, cx: &mut Context<Self>) {
        if !self.split_sizes_dirty {
            return;
        }
        self.split_sizes_dirty = false;
        if let Err(error) = self.persist_active_tab(cx) {
            self.chat_error = Some(error.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{pane_rename_change, PaneRenameChange};

    #[test]
    fn pane_rename_enter_persists_titles_and_empty_restores_the_default() {
        assert_eq!(
            pane_rename_change("@coder", "  feature 64  "),
            PaneRenameChange::Persist(Some("feature 64".into()))
        );
        assert_eq!(
            pane_rename_change("feature 64", "   "),
            PaneRenameChange::Persist(None)
        );
        assert_eq!(
            pane_rename_change("feature 64", " feature 64 "),
            PaneRenameChange::Unchanged
        );
    }
}
