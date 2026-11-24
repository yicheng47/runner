mod checkpoint;
mod windows;

pub(crate) use checkpoint::*;
pub(crate) use windows::*;

use super::*;
use crate::*;
use runner_core::protocol::window::Subject;

impl NativeRoot {
    pub(crate) fn current_subjects(&self) -> Vec<Subject> {
        if self.remote_client.is_active() {
            return self.remote_client.current_subjects();
        }
        match &self.route {
            AppRoute::Mission(mission_id) => vec![Subject::Mission(mission_id.clone())],
            AppRoute::Chat => self
                .tabs
                .active()
                .into_iter()
                .flat_map(|layout| subjects_for_pane_tree(&layout.root))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn report_current_subjects(&mut self, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        self.report_current_subjects_and_focus(None, None, cx);
    }

    pub(crate) fn report_current_subjects_and_focus(
        &mut self,
        focus: Option<bool>,
        viewed_tab: Option<(String, Vec<String>, Option<String>)>,
        cx: &mut Context<Self>,
    ) {
        if self.closing {
            return;
        }
        let subjects = self.current_subjects();
        self.remote_client
            .record_reported_subjects(subjects.clone());
        let focused_session_id = self
            .tabs
            .active()
            .filter(|_| self.route == AppRoute::Chat && self.remote_client.is_local())
            .and_then(PaneLayout::focused_session_id)
            .map(str::to_owned);
        let label = self.window_label.clone();
        self.app_store.update(cx, |store, store_cx| {
            store.enqueue_window_state(
                &label,
                subjects,
                focused_session_id,
                focus,
                viewed_tab,
                store_cx,
            );
        });
        checkpoint_window_layout_deferred(cx);
        crate::notifications::view_changed(cx);
    }

    pub(crate) fn enqueue_tab_viewed(
        &mut self,
        tab_id: String,
        member_ids: Vec<String>,
        viewed_session_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.closing {
            return;
        }
        let label = self.window_label.clone();
        self.app_store.update(cx, |store, store_cx| {
            store.enqueue_window_tab_viewed(
                &label,
                tab_id,
                member_ids,
                viewed_session_id,
                store_cx,
            );
        });
        checkpoint_window_layout_deferred(cx);
        crate::notifications::view_changed(cx);
    }

    pub(crate) fn sync_window_activation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _turn = crate::stall_watchdog::enter_turn("window.sync_activation");
        if self.closing {
            return;
        }
        let active = window.is_window_active();
        self.window_active = active;
        let viewed_tab = if active && self.remote_client.is_local() && self.route == AppRoute::Chat
        {
            self.tabs.active().map(|layout| {
                (
                    layout.id.clone(),
                    layout.session_ids(),
                    layout.focused_session_id().map(str::to_owned),
                )
            })
        } else {
            None
        };
        self.report_current_subjects_and_focus(Some(active), viewed_tab, cx);
        if active {
            crate::notifications::refresh_permission(cx);
        }
        crate::notifications::view_changed(cx);
        self.sync_subject_ownership(window, cx);
        if active && self.remote_client.is_active() {
            self.mark_remote_sessions_viewed_if_unobstructed(cx);
        }
        if self.remote_client.is_local() && matches!(self.route, AppRoute::Mission(_)) {
            self.mission_workspace.update(cx, |workspace, cx| {
                workspace.mark_active_session_viewed(window, cx);
            });
        }
        cx.notify();
    }

    pub(crate) fn start_focus_map_listener(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (focus_tx, mut focus_rx) = futures::channel::mpsc::unbounded::<bool>();
        let mut events = self.core(cx).subscribe();
        let bg_events = cx.background_spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event)
                        if matches!(
                            event.name.as_str(),
                            "window_focus_map" | "daemon/reconnected"
                        ) =>
                    {
                        if focus_tx
                            .unbounded_send(event.name == "daemon/reconnected")
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(_) | Err(runner_core::protocol::EventError::Lagged(_)) => {}
                    Err(runner_core::protocol::EventError::Closed) => break,
                }
            }
        });
        let fg_events = cx.spawn_in(window, async move |weak, cx| {
            while let Some(mut reconnected) = focus_rx.next().await {
                while let Ok(next) = focus_rx.try_recv() {
                    reconnected |= next;
                }
                if weak
                    .update_in(cx, |this, window, cx| {
                        if this.closing {
                            return;
                        }
                        let label = this.window_label.clone();
                        this.app_store.update(cx, |store, store_cx| {
                            store.enqueue_window_focus_event(&label, reconnected, store_cx);
                        });
                        if reconnected {
                            this.sync_window_activation(window, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        self._focus_map_tasks = Some([bg_events, fg_events]);
    }

    pub(crate) fn sync_subject_ownership(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        self.sync_chat_subject_ownership(window, cx);
        if self.remote_client.is_local() {
            self.sync_mission_subject_ownership(window, cx);
        }
    }

    fn sync_chat_subject_ownership(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote_client.is_active() {
            self.refresh_chat_secondaries(cx);
            self.release_demoted_remote_terminal_focus(window, cx);
            self.remote_client.attach_remote_panes(cx);
            if let Some(focused_id) = self
                .remote_client
                .active_tab_layout()
                .and_then(PaneLayout::focused_session_id)
                .map(str::to_owned)
            {
                self.restore_remote_terminal_focus_if_eligible(&focused_id, window, cx);
            }
            self.remote_client.reconcile_auto_take(cx);
            if self.window_active && self.remote_unobstructed() {
                self.remote_client.mark_remote_sessions_viewed_on_sync(cx);
            }
            cx.notify();
            return;
        }
        if self.route == AppRoute::Chat {
            if let Err(error) = self.ensure_owned_active_tab_attached(window, cx) {
                self.chat_error = Some(error.to_string().into());
            }
        } else {
            self.refresh_chat_secondaries(cx);
            self.attached.clear();
        }
        cx.notify();
    }

    fn release_demoted_remote_terminal_focus(&mut self, window: &mut Window, cx: &mut App) {
        let Some(focused) = window.focused(cx) else {
            return;
        };
        if self
            .remote_client
            .release_demoted_terminal_focus(&self.chat_secondaries, &focused)
        {
            self.chat_focus.focus(window, cx);
        }
    }

    pub(crate) fn allows_remote_terminal_auto_focus_window(&self, cx: &App) -> bool {
        !(self.closing
            || !self.remote_client.is_active()
            || self.route == AppRoute::Settings
            || self.command_palette.read(cx).is_open()
            || self.start_chat_modal.is_some()
            || self.project_modal.is_some()
            || self.project_delete_confirm.is_some()
            || self.pane_rename.is_some()
            || self.terminal_close_confirm.is_some()
            || self.fork_confirm.is_some()
            || self.quit_dialog.is_some()
            || self.agent_update.is_some()
            || self.usage.open)
    }

    pub(crate) fn can_auto_focus_remote_terminal(&self, window: &Window, cx: &App) -> bool {
        if !self.allows_remote_terminal_auto_focus_window(cx) {
            return false;
        }
        if let Some(workspace) = self.remote_client.remote_mission_workspace() {
            if !workspace
                .read(cx)
                .allows_remote_center_auto_focus(window, cx)
            {
                return false;
            }
        }
        let focused = window.focused(cx);
        let neutral = focused.as_ref().is_some_and(|current| {
            current == &self.root_focus
                || current == &self.chat_focus
                || self
                    .remote_client
                    .remote_mission_workspace()
                    .is_some_and(|ws| ws.read(cx).has_neutral_remote_focus(Some(current)))
        });
        if neutral {
            return self.remote_client.allows_auto_terminal_focus(None);
        }
        self.remote_client
            .allows_auto_terminal_focus(focused.as_ref())
    }

    pub(crate) fn restore_remote_terminal_focus_if_eligible(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(workspace) = self.remote_client.remote_mission_workspace() {
            let Some(drawer) = workspace
                .read(cx)
                .remote_auto_focus_target(session_id, window, cx)
            else {
                return;
            };
            let eligible = if drawer {
                self.allows_remote_terminal_auto_focus_window(cx)
                    && self.remote_client.allows_auto_terminal_focus(None)
            } else {
                self.can_auto_focus_remote_terminal(window, cx)
            };
            if eligible {
                self.remote_client
                    .focus_terminal_and_clear_dropped(session_id, window, cx);
            }
            return;
        }
        if !self.can_auto_focus_remote_terminal(window, cx) {
            return;
        }
        let Some(focused_id) = self
            .remote_client
            .active_tab_layout()
            .and_then(PaneLayout::focused_session_id)
        else {
            return;
        };
        if focused_id != session_id || self.cached_chat_secondary_state(session_id).secondary {
            return;
        }
        self.remote_client
            .focus_terminal_and_clear_dropped(session_id, window, cx);
    }

    pub(crate) fn refresh_chat_secondaries(&mut self, cx: &mut Context<Self>) {
        let active_ids = if self.remote_client.is_active() {
            self.remote_client
                .active_tab_layout()
                .map(PaneLayout::session_ids)
                .unwrap_or_default()
        } else {
            self.tabs
                .active()
                .filter(|_| self.route == AppRoute::Chat)
                .map(PaneLayout::session_ids)
                .unwrap_or_default()
        };
        let remote_computer = self.remote_client.computer_id().map(str::to_owned);
        let entries = &self.app_store.read(cx).window_entries;
        let next = active_ids
            .iter()
            .filter_map(|session_id| {
                let subject = match &remote_computer {
                    Some(computer) => Subject::RemoteChat {
                        computer: computer.clone(),
                        session: session_id.clone(),
                    },
                    None => Subject::DirectChat(session_id.clone()),
                };
                let state = runner_core::protocol::window::is_secondary_for(
                    entries,
                    &self.window_label,
                    &subject,
                );
                state
                    .primary_label
                    .map(|primary| (session_id.clone(), primary))
            })
            .collect::<HashMap<_, _>>();
        if next != self.chat_secondaries {
            self.dismissed_duplicate_chats
                .retain(|session_id| next.contains_key(session_id));
            for session_id in next.keys() {
                if !self.chat_secondaries.contains_key(session_id) {
                    self.dismissed_duplicate_chats.remove(session_id);
                }
            }
            self.chat_secondaries = next;
        }
        self.remote_client
            .set_secondaries(self.chat_secondaries.keys().cloned().collect());
    }

    pub(crate) fn chat_secondary_state(
        &self,
        session_id: &str,
        cx: &App,
    ) -> runner_core::protocol::window::SecondaryState {
        let subject = match self.remote_client.computer_id() {
            Some(computer) => Subject::RemoteChat {
                computer: computer.to_owned(),
                session: session_id.to_owned(),
            },
            None => Subject::DirectChat(session_id.to_owned()),
        };

        runner_core::protocol::window::is_secondary_for(
            &self.app_store.read(cx).window_entries,
            &self.window_label,
            &subject,
        )
    }

    pub(crate) fn cached_chat_secondary_state(
        &self,
        session_id: &str,
    ) -> runner_core::protocol::window::SecondaryState {
        let primary_label = self.chat_secondaries.get(session_id).cloned();
        runner_core::protocol::window::SecondaryState {
            secondary: primary_label.is_some(),
            primary_label,
        }
    }

    pub(crate) fn dismiss_duplicate_chat(&mut self, session_id: &str, cx: &mut Context<Self>) {
        self.dismissed_duplicate_chats.insert(session_id.to_owned());
        cx.notify();
    }

    pub(crate) fn persisted_window_route(&self) -> Option<String> {
        match &self.route {
            AppRoute::Chat => Some(
                self.active_focused_session_id()
                    .map(|session_id| format!("/chats/{session_id}"))
                    .unwrap_or_else(|| "/chats".into()),
            ),
            AppRoute::Roles | AppRoute::NewRole => Some("/roles".into()),
            AppRoute::RoleDetail(handle) => Some(format!("/roles/{handle}")),
            AppRoute::Crews | AppRoute::NewCrew => Some("/crews".into()),
            AppRoute::CrewEditor(crew_id) => Some(format!("/crews/{crew_id}")),
            AppRoute::Mission(mission_id) => Some(format!("/missions/{mission_id}")),
            AppRoute::Settings => Some("/settings".into()),
            AppRoute::Launcher | AppRoute::ArchivedChat => None,
        }
    }

    pub(crate) fn prepare_window_close(&mut self, window: &Window, cx: &mut Context<Self>) {
        if cx
            .try_global::<runner_app::lifecycle::QuitState>()
            .is_some()
        {
            cx.global_mut::<runner_app::lifecycle::QuitState>()
                .cancel(window.window_handle().window_id());
        }
        self.quit_dialog = None;
        if self.closing {
            return;
        }
        self.closing = true;
        self._focus_map_tasks = None;
        self.save_main_window_state(window, cx);
        self.save_settings(cx);
        self.attached.clear();
        self.remote_client.disconnect();
        self.side_panel.files.release(cx);
        self.mission_workspace
            .update(cx, |workspace, workspace_cx| {
                workspace.release_window(workspace_cx)
            });
        let label = self.window_label.clone();
        self.app_store.update(cx, |store, store_cx| {
            store.close_window_queue(&label, store_cx);
        });
        checkpoint_window_layout_deferred(cx);
    }
}

fn subjects_for_pane_tree(root: &PaneNode) -> Vec<Subject> {
    root.leaves()
        .into_iter()
        .filter_map(|leaf| leaf.session_id.clone())
        .map(Subject::DirectChat)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_subjects_come_from_every_leaf_in_the_active_pane_tree() {
        let mut layout = PaneLayout::single(Some("chat-a"), &["chat-a".into()]);
        let empty = layout.split("p1", SplitOrientation::Row).unwrap();
        layout.assign_session(&empty, "chat-b").unwrap();
        assert_eq!(
            subjects_for_pane_tree(&layout.root),
            vec![
                Subject::DirectChat("chat-a".into()),
                Subject::DirectChat("chat-b".into()),
            ]
        );
    }
}
