use super::*;

impl NativeRoot {
    fn clear_remote_transients(&mut self) {
        self.usage_open = false;
        self.remote_client.usage_popover = None;
        self.start_chat_modal = None;
        self.pane_rename = None;
        self.maximized_panes.clear();
        self.fork_confirm = None;
        self.terminal_close_confirm = None;
    }

    pub(super) fn choose_computer(
        &mut self,
        computer: Computer,
        new_window: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remote_client.ui.picker_open = false;
        if computer.is_self {
            if new_window {
                let _ = open_new_runner_window(None, cx);
            } else {
                self.return_to_this_computer(window, cx);
            }
        } else if !computer.paired {
            self.remote_client.open_computer_pair(
                computer,
                pairing::Purpose::Open { new_window },
                window,
                cx,
            );
        } else {
            self.remote_client.open_computer(computer, new_window, cx);
        }
        cx.notify();
    }

    pub(crate) fn visit_computer(
        &mut self,
        computer: Computer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .remote_client
            .visit_computer(computer, &self.window_label, window, cx)
        {
            self.clear_remote_transients();
            self.command_palette
                .update(cx, |palette, cx| palette.close_for_target_change(cx));
            self.mission_workspace
                .update(cx, |workspace, cx| workspace.release_window(cx));
            self.report_current_subjects(cx);
            self.sync_remote_state(cx);
        }
    }

    pub(crate) fn return_to_this_computer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_remote_transients();
        self.command_palette
            .update(cx, |palette, cx| palette.close_for_target_change(cx));
        self.remote_client.return_to_this_computer(window, cx);
        self.sync_window_activation(window, cx);
    }

    pub(crate) fn open_this_computer_settings(
        &mut self,
        pane: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remote_client.ui.picker_open = false;
        if self.remote_client.is_active() {
            self.return_to_this_computer(window, cx);
        }
        self.enter_settings_route(pane, window, cx);
        cx.notify();
    }

    pub(crate) fn sync_remote_state(&mut self, cx: &mut Context<Self>) {
        let connection = self.remote_client.connection_marker();
        let was_empty = self.launcher_is_empty_state();
        if self.remote_client.sync_remote_state(cx) {
            self.clear_remote_transients();
        }
        if !was_empty && self.launcher_is_empty_state() {
            self.reset_launcher_page(cx);
        }
        if self.remote_client.connection_marker() != connection {
            self.maximized_panes.clear();
        }
        self.remote_client
            .refresh_remote_palette(&self.command_palette, cx);
        if self.window_active && self.remote_unobstructed() {
            self.remote_client.mark_remote_sessions_viewed_on_sync(cx);
        }
    }

    fn remote_unobstructed(&self) -> bool {
        !self.remote_client.ui.picker_open && self.start_chat_modal.is_none()
    }

    pub(crate) fn mark_remote_sessions_viewed_if_unobstructed(&mut self, cx: &mut Context<Self>) {
        if self.remote_unobstructed() {
            self.remote_client.mark_remote_sessions_viewed(cx);
        }
    }

    pub(crate) fn select_remote_item(&mut self, item: RemoteItem, cx: &mut Context<Self>) {
        if self.remote_client.select_remote_item(item) {
            self.start_chat_modal = None;
        }
        self.sync_remote_state(cx);
        self.remote_client.mark_remote_sessions_viewed(cx);
        cx.notify();
    }

    pub(crate) fn render_remote_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mission = self
            .remote_client
            .workspace
            .as_ref()
            .is_some_and(|remote| matches!(remote.active, Some(RemoteItem::Mission(_))));
        let library = self
            .remote_client
            .workspace
            .as_ref()
            .and_then(|remote| remote.active.clone());
        let body = if matches!(library, Some(RemoteItem::Roles)) {
            self.render_roles_page(window, cx)
        } else if matches!(library, Some(RemoteItem::Crews)) {
            self.render_crews_page(window, cx)
        } else if mission {
            self.remote_client.mission_surface(cx).into_any_element()
        } else {
            self.render_active_tab(window, cx)
        };
        self.remote_client.render_remote_workspace(body, cx)
    }
}
