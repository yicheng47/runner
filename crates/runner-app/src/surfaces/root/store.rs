use super::*;

impl NativeRoot {
    pub(crate) fn handle_app_store_update(&mut self, cx: &mut Context<Self>) {
        let core = self.core(cx).clone();
        if core
            .usage_set_enabled(self.settings(cx).model_runtimes())
            .unwrap_or(false)
        {
            cx.notify();
        }
        let revisions = self.app_store.read(cx).revisions;
        let previous = self.store_revisions;
        self.store_revisions = revisions;
        let reactions = revisions.reactions_since(previous);
        if revisions.remote_enrollment != previous.remote_enrollment {
            self.remote_finish_sign_in(cx);
        }
        if revisions.remote_notice != previous.remote_notice {
            let status = self.app_store.read(cx).remote_warning.clone();
            if let Some(status) = status {
                let pin_locked = matches!(status, app_store::RemoteWarning::PinLocked);
                self.show_toast(
                    match status {
                        app_store::RemoteWarning::Stopped(status) => {
                            surfaces::settings::remote::stopped_text(&status).into()
                        }
                        app_store::RemoteWarning::PinLocked => {
                            i18n::message("remote.pin_locked_toast")
                        }
                    },
                    toast::ToastTone::Warning,
                    cx,
                );
                if pin_locked {
                    self.toasts.pin_lock_warning();
                }
            }
        }

        if revisions.settings != previous.settings {
            self.sync_role_page_size(cx);
            self.sync_crew_page_size(cx);
            let language = self.settings(cx).language.key();
            self.settings_page
                .language
                .update(cx, |select, cx| select.set_value(language, cx));
            let value = self.settings(cx).quit_behavior.key();
            self.settings_page
                .quit_behavior
                .update(cx, |select, cx| select.set_value(value, cx));
            self.sync_terminal_scrollback_select(cx);
            self.sync_side_panel_selects(cx);
            #[cfg(target_os = "macos")]
            {
                let value = self.settings(cx).option_as_meta.key();
                self.settings_page
                    .option_as_meta
                    .update(cx, |select, cx| select.set_value(value, cx));
            }
        }
        if revisions.locale != previous.locale {
            self.toasts.relocalize();
            self.command_palette
                .update(cx, |palette, cx| palette.locale_changed(cx));
        }
        let mut recovery_toast = None;
        if reactions.sync_error {
            let store = self.app_store.read(cx);
            self.error = (!store.daemon_disconnected)
                .then(|| store.error.clone())
                .flatten();
            if !store.daemon_disconnected
                && store.daemon_notice == Some(runner_app::lifecycle::DaemonNotice::Stopped)
            {
                recovery_toast = store.error.clone();
                self.error = None;
            }
            if store.daemon_disconnected {
                self.maximized_panes.clear();
                self.toasts.dismiss();
            } else if store.daemon_notice.is_some() || store.error.is_none() {
                self.chat_error = None;
                self.mission_workspace
                    .update(cx, |workspace, cx| workspace.clear_connection_error(cx));
            }
        }
        if let Some(message) = recovery_toast {
            self.show_toast(message, toast::ToastTone::Error, cx);
            self.toasts.make_single_line();
        }
        if revisions.restart != previous.restart {
            if let Some(message) = self
                .app_store
                .read(cx)
                .restarted_sessions
                .and_then(|count| {
                    runner_app::lifecycle::restart_message(
                        count,
                        &runner_app::version::display_version_label(),
                    )
                })
            {
                self.show_toast(message, toast::ToastTone::Restart, cx);
            }
        }
        if reactions.reload_tabs {
            self.apply_tab_rows(cx);
        }
        if reactions.prune_sidebar {
            self.prune_sidebar_collapse_state(cx);
        }
        if reactions.prune_window_state {
            self.prune_store_dependent_window_state(cx);
        }
        if revisions.roles != previous.roles
            && self
                .active_chat_detail
                .as_ref()
                .is_some_and(|detail| detail.role_id.is_some())
        {
            self.sync_active_chat_detail(cx);
        }
        if reactions.reload_role_surfaces {
            match self.route.clone() {
                AppRoute::Roles => self.load_local_role_page(cx),
                AppRoute::RoleDetail(handle) => self.load_role_detail(handle, cx),
                _ => {}
            }
        }
        if reactions.reload_crew_surfaces {
            match self.route.clone() {
                AppRoute::Crews => self.load_local_crew_page(cx),
                AppRoute::CrewEditor(crew_id) => self.load_crew_editor(crew_id, cx),
                _ => {}
            }
        }
        if reactions.apply_terminal_settings {
            self.apply_terminal_settings(cx);
        }

        if reactions.notify_shell() || (reactions.terminal_wake && self.route.terminal_visible()) {
            cx.notify();
        }
    }
}
