use std::sync::Arc;

use gpui::prelude::*;
use gpui::{ClipboardItem, KeyDownEvent, ScrollDelta, ScrollWheelEvent, Window};
use runner_terminal::input_state::ECHO_WINDOW;

use super::*;
use crate::*;

impl MissionWorkspace {
    pub(crate) fn cycle_mission_tab(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut tabs = vec![MissionTab::Feed];
        if !self.archived() && !self.secondary {
            tabs.extend(
                self.open_tabs
                    .iter()
                    .filter(|session_id| {
                        self.sessions
                            .iter()
                            .any(|session| session.session.id == session_id.as_str())
                    })
                    .map(|session_id| MissionTab::Session(session_id.clone())),
            );
        }
        let Some(next) = mission_tab_in_direction(&tabs, &self.active_tab, direction) else {
            return;
        };
        match next {
            MissionTab::Feed => self.select_mission_feed(window, cx),
            MissionTab::Session(session_id) => {
                self.select_mission_session(&session_id, window, cx);
            }
        }
    }

    pub(super) fn select_mission_feed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cache_active_terminal_size(window, cx);
        self.clear_feed_selection();
        self.active_tab = MissionTab::Feed;
        if self.feed_was_near_bottom {
            self.feed_scroll.scroll_to_bottom();
            self.feed_has_new_messages = false;
        }
        window.focus(&self.root_focus);
        cx.notify();
    }

    pub(super) fn select_mission_session(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.archived() || self.secondary {
            return;
        }
        let valid = self
            .sessions
            .iter()
            .any(|session| session.session.id == session_id);
        if !valid {
            self.select_mission_feed(window, cx);
            return;
        }
        self.cache_active_terminal_size(window, cx);
        self.clear_feed_selection();
        if !self.open_tabs.iter().any(|open| open == session_id) {
            self.open_tabs.push(session_id.to_owned());
        }
        if let Some(mission_id) = &self.mission_id {
            let mission_id = mission_id.clone();
            let session_id = session_id.to_owned();
            self.update_app_settings(cx, true, move |settings| {
                settings
                    .last_mission_terminal_ids
                    .insert(mission_id, session_id);
                true
            });
        }
        self.active_tab = MissionTab::Session(session_id.to_owned());
        if let Err(error) = self.ensure_mission_terminals_attached(window, cx) {
            self.error = Some(error.to_string());
        }
        self.focus_active_mission_terminal(window, cx);
        cx.notify();
    }

    pub(super) fn close_mission_session_tab(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cache_active_terminal_size(window, cx);
        self.open_tabs.retain(|open| open != session_id);
        if let Some(mission_id) = &self.mission_id {
            if self
                .settings(cx)
                .last_mission_terminal_ids
                .get(mission_id)
                .is_some_and(|remembered| remembered == session_id)
            {
                let mission_id = mission_id.clone();
                self.update_app_settings(cx, true, move |settings| {
                    settings.last_mission_terminal_ids.remove(&mission_id);
                    true
                });
            }
        }
        if self.active_tab == MissionTab::Session(session_id.to_owned()) {
            self.clear_feed_selection();
            self.active_tab = MissionTab::Feed;
            window.focus(&self.root_focus);
        }
        self.attached.remove(session_id);
        cx.notify();
    }

    pub(super) fn on_mission_key_down(
        &mut self,
        session_id: &str,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.mission_terminal_interactive(session_id, cx) {
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

    pub(super) fn on_mission_terminal_copy(
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
            #[cfg(windows)]
            cx.propagate();
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        cx.stop_propagation();
    }

    pub(super) fn on_mission_scroll(
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

    pub(super) fn on_mission_paste(
        &mut self,
        session_id: &str,
        _: &Paste,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.mission_terminal_interactive(session_id, cx) {
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
                            Ok(()) => this.error = None,
                            Err(error) => this.error = Some(error.to_string()),
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
                    self.error = Some(error.to_string());
                }
            }
        }
    }

    pub(super) fn submit_or_clear_mission_input(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.mission_terminal_interactive(session_id, cx) {
            return;
        }
        let Some(chat) = self.attached.get(session_id) else {
            self.error = Some("Terminal is not attached.".into());
            cx.notify();
            return;
        };
        let terminal = Arc::clone(&chat.terminal);
        if let Err(error) = terminal.send_key("enter", false, false, false, None) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        let reset_guard = terminal.input_reset_guard();
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(ECHO_WINDOW).await;
            terminal.reset_input_state(reset_guard);
        })
        .detach();
        chat.terminal_focus.focus(window);
        cx.notify();
    }
}
