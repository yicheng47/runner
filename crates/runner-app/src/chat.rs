//! Chat lifecycle and input: attaching sessions to terminals, tab
//! activation/focus, key/scroll/paste routing, chat start/resume, and
//! split resizing.
use super::*;

impl NativeRoot {
    pub(crate) fn ensure_active_tab_attached(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let Some(layout) = self.tabs.active().cloned() else {
            return Ok(());
        };
        let mut errors = Vec::new();
        for session_id in layout.session_ids() {
            if let Err(error) = self.ensure_attached(&layout, &session_id, window, cx) {
                errors.push(error.to_string());
            }
        }
        self.refresh_sessions();
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(errors.join("\n"))
        }
    }

    pub(crate) fn ensure_attached(
        &mut self,
        layout: &PaneLayout,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let _entry = runner_backend::ops::session::session_get(&self.core, session_id)?
            .with_context(|| format!("direct chat not found: {session_id}"))?;
        let pane_id = layout
            .root
            .leaves()
            .into_iter()
            .find(|leaf| leaf.session_id.as_deref() == Some(session_id))
            .map(|leaf| leaf.id.as_str());
        let estimated = pane_id
            .map(|pane_id| self.estimated_terminal_size(layout, pane_id, window))
            .unwrap_or((INITIAL_COLS, INITIAL_ROWS));
        let size = self
            .attached
            .get(session_id)
            .map(|chat| chat.terminal.size())
            .unwrap_or(estimated);

        if self.attached.contains_key(session_id) {
            return Ok(());
        }

        let terminal = TerminalSession::attach(
            self.core.clone(),
            session_id.to_owned(),
            size.0,
            size.1,
            Arc::clone(&self.waker),
        )?;
        terminal.set_palette(self.settings.terminal_theme.palette());
        self.bridge.attach(Arc::clone(&terminal))?;
        let terminal_focus = cx.focus_handle();
        let terminal_input = cx.new(|_| TerminalInput::new(Arc::clone(&terminal)));
        let terminal_input_subscription = cx.observe(&terminal_input, |this, input, cx| {
            if let Some(result) = input.update(cx, |input, _| input.take_write_result()) {
                match result {
                    Ok(()) => this.error = None,
                    Err(error) => this.error = Some(error),
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
                terminal,
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
    ) -> (u16, u16) {
        let (width_fraction, height_fraction) =
            pane_fractions(&layout.root, pane_id).unwrap_or((1., 1.));
        let bounds = window.bounds().size;
        let sidebar_width = if self.settings.sidebar_collapsed {
            0.
        } else {
            self.settings.sidebar_width * self.settings.app_zoom
        };
        let pane_width = (f32::from(bounds.width) - sidebar_width).max(200.) * width_fraction;
        let grouped = layout.root.leaves().len() > 1;
        let pane_height =
            (f32::from(bounds.height) - WORKSPACE_HEADER_HEIGHT * self.settings.app_zoom).max(160.)
                * height_fraction
                - if grouped {
                    PANE_HEADER_HEIGHT * self.settings.app_zoom
                } else {
                    0.
                };
        let font_size = self.settings.terminal_font_size as f32 * self.settings.app_zoom;
        let cell_width = font_size * 0.6;
        let line_height = (font_size * terminal_element::LINE_HEIGHT_FACTOR).round();
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

    pub(crate) fn focus_active_terminal(&self, window: &mut Window) {
        let Some(session_id) = self.active_focused_session_id() else {
            self.root_focus.focus(window);
            return;
        };
        if let Some(chat) = self.attached.get(&session_id) {
            chat.terminal_focus.focus(window);
        } else {
            self.root_focus.focus(window);
        }
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
        self.layout_picker_open = false;
        match self.ensure_active_tab_attached(window, cx) {
            Ok(()) => {
                self.error = None;
                self.remember_active_runner();
                self.focus_active_terminal(window);
            }
            Err(error) => self.error = Some(error.to_string()),
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
                    self.session_entry(session_id)
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
            cx.notify();
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
            .session_entry(session_id)
            .and_then(|entry| entry.runner_id.clone());
        if let Some(chat) = self.attached.get(session_id) {
            chat.terminal_focus.focus(window);
        }
    }

    pub(crate) fn on_key_down(
        &mut self,
        session_id: &str,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.attached.get(session_id) else {
            return;
        };
        let keystroke = &event.keystroke;
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
            keystroke.key_char.as_deref(),
        ) {
            Ok(true) => {
                chat.terminal.scroll_to_bottom();
                self.error = None;
                cx.stop_propagation();
                cx.notify();
            }
            Ok(false) => {}
            Err(error) => {
                self.error = Some(error.to_string());
                cx.stop_propagation();
                cx.notify();
            }
        }
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
            chat.terminal.scroll(whole);
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
        let Some(chat) = self.attached.get(session_id) else {
            return;
        };
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if let Err(error) = chat.terminal.paste(&text) {
            self.error = Some(error.to_string());
        }
    }

    pub(crate) fn resume_chat(
        &mut self,
        pane_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = (|| -> Result<()> {
            let layout = self.tabs.active().cloned().context("no active tab")?;
            let size = self
                .attached
                .get(session_id)
                .map(|chat| chat.terminal.size())
                .unwrap_or_else(|| self.estimated_terminal_size(&layout, pane_id, window));
            runner_backend::ops::session::session_resume(
                &self.core,
                session_id,
                Some(size.0),
                Some(size.1),
            )?;
            self.ensure_attached(&layout, session_id, window, cx)?;
            self.refresh_sessions();
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.error = None;
                self.focus_terminal(pane_id, session_id, window, cx);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    pub(crate) fn pick_preset(
        &mut self,
        preset: PresetKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = (|| -> Result<Option<String>> {
            let Some(layout) = self.tabs.active_mut() else {
                return Ok(None);
            };
            layout.apply_preset(preset);
            let empty_pane_id = layout
                .root
                .leaves()
                .into_iter()
                .find(|leaf| leaf.session_id.is_none())
                .map(|leaf| leaf.id.clone());
            self.persist_active_tab()?;
            self.reload_tabs()?;
            self.ensure_active_tab_attached(window, cx)?;
            Ok(empty_pane_id)
        })();
        self.layout_picker_open = false;
        match result {
            Ok(empty_pane_id) => {
                self.error = None;
                if let Some(pane_id) = empty_pane_id {
                    self.open_pane_chat_modal(&pane_id, window, cx);
                } else {
                    self.remember_active_runner();
                    self.focus_active_terminal(window);
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    pub(crate) fn persist_active_tab(&self) -> Result<()> {
        let input = self
            .tabs
            .active()
            .context("active tab is missing")?
            .upsert_input()?;
        runner_backend::ops::node::node_tab_upsert(&self.core, input)?;
        Ok(())
    }

    pub(crate) fn close_pane(
        &mut self,
        pane_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = (|| -> Result<bool> {
            let Some(layout) = self.tabs.active_mut() else {
                return Ok(false);
            };
            if !layout.close_pane(pane_id) {
                return Ok(false);
            }
            self.persist_active_tab()?;
            self.reload_tabs()?;
            self.ensure_active_tab_attached(window, cx)?;
            Ok(true)
        })();
        match result {
            Ok(true) => {
                self.error = None;
                self.remember_active_runner();
                self.focus_active_terminal(window);
            }
            Ok(false) => {}
            Err(error) => self.error = Some(error.to_string()),
        }
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

    pub(crate) fn finish_split_resize(&mut self) {
        if !self.split_sizes_dirty {
            return;
        }
        self.split_sizes_dirty = false;
        if let Err(error) = self.persist_active_tab() {
            self.error = Some(error.to_string());
        }
    }
}
