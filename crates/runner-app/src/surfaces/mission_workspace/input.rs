use std::sync::Arc;

use gpui::prelude::*;
use gpui::{ClipboardItem, KeyDownEvent, Window};
use runner_terminal::input_state::ECHO_WINDOW;

use super::*;
use crate::*;

impl MissionWorkspace {
    pub(crate) fn mark_active_session_viewed(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.is_active(cx)
            || !window.is_window_active()
            || self.archived()
            || self.secondary
            || self.loading
        {
            return;
        }
        let MissionTab::Session(session_id) = &self.active_tab else {
            return;
        };
        if let Err(error) = self
            .core(cx)
            .mark_direct_sessions_viewed(std::slice::from_ref(session_id))
        {
            self.error = Some(error.to_string().into());
        }
    }

    pub(crate) fn focus_status_session(
        &mut self,
        mission_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mission_id.as_deref() == Some(mission_id) && !self.loading {
            self.select_mission_session(session_id, window, cx);
        }
    }

    pub(crate) fn notification_tab(&self, mission_id: &str) -> Option<&MissionTab> {
        (self.mission_id.as_deref() == Some(mission_id) && !self.loading && self.mission.is_some())
            .then_some(&self.active_tab)
    }

    // Called only from a notification click; see `mod notifications`.
    #[cfg_attr(all(test, windows), allow(dead_code))]
    pub(crate) fn focus_feed(
        &mut self,
        mission_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mission_id.as_deref() == Some(mission_id) && !self.loading {
            self.select_mission_feed(window, cx);
        }
    }

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
        self.schedule_mission_grid_hint(window, cx);
        self.clear_feed_selection();
        if !self.secondary {
            if let Some(mission_id) = self.mission_id.clone() {
                self.update_app_settings(cx, true, move |settings| {
                    settings
                        .last_mission_terminal_ids
                        .remove(&mission_id)
                        .is_some()
                });
            }
        }
        self.active_tab = MissionTab::Feed;
        crate::notifications::view_changed(cx);
        if self.feed_was_near_bottom {
            self.scroll_feed_to_bottom();
            self.feed_has_new_messages = false;
        }
        window.focus(&self.root_focus, cx);
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
        self.schedule_mission_grid_hint(window, cx);
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
        self.feed_list.scrollbar_drag_ended();
        self.active_tab = MissionTab::Session(session_id.to_owned());
        crate::notifications::view_changed(cx);
        self.mark_active_session_viewed(window, cx);
        if let Err(error) = self.ensure_mission_terminals_attached(window, cx) {
            self.error = Some(error.to_string().into());
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
        self.schedule_mission_grid_hint(window, cx);
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
            crate::notifications::view_changed(cx);
            window.focus(&self.root_focus, cx);
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
                self.error = Some(error.to_string().into());
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
            #[cfg(any(windows, target_os = "linux"))]
            cx.propagate();
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        cx.stop_propagation();
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
        let Some(paste) = runner_app::terminal_paste::resolve_terminal_paste(item.as_ref(), || {
            self.core(cx)
                .session_clipboard_file_paths()
                .unwrap_or_default()
        }) else {
            return;
        };
        match paste {
            runner_app::terminal_paste::TerminalPaste::Image => {
                let runtime = self
                    .sessions
                    .iter()
                    .find(|row| row.session.id == session_id)
                    .and_then(|row| runner_core::Runtime::parse(&row.runtime));
                let Some(chat) = self.attached.get(session_id) else {
                    return;
                };
                let input = runner_app::terminal_paste::image_paste_shortcut(runtime);
                if let Err(error) = chat.terminal.write_user_bytes(input) {
                    self.error = Some(error.to_string().into());
                }
            }
            runner_app::terminal_paste::TerminalPaste::Text(text) => {
                let Some(chat) = self.attached.get(session_id) else {
                    return;
                };
                if let Err(error) = chat.terminal.paste(&text) {
                    self.error = Some(error.to_string().into());
                }
            }
        }
    }

    pub(super) fn on_mission_file_drop(
        &mut self,
        session_id: &str,
        paths: &gpui::ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.mission_terminal_interactive(session_id, cx) {
            return;
        }
        let Some(text) = runner_app::terminal_paste::dropped_paths_text(paths) else {
            return;
        };
        let Some(chat) = self.attached.get(session_id) else {
            return;
        };
        chat.terminal_focus.focus(window, cx);
        if let Err(error) = chat.terminal.paste(&text) {
            self.error = Some(error.to_string().into());
        }
        cx.notify();
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
            self.error = Some(crate::i18n::message("ui.terminal_is_not_attached"));
            cx.notify();
            return;
        };
        let terminal = Arc::clone(&chat.terminal);
        if let Err(error) = terminal.send_key("enter", false, false, false, None) {
            self.error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        let reset_guard = terminal.input_reset_guard();
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(ECHO_WINDOW).await;
            terminal.reset_input_state(reset_guard);
        })
        .detach();
        chat.terminal_focus.focus(window, cx);
        cx.notify();
    }
}
