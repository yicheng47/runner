use super::*;
use crate::*;
use runner_core::protocol::window::Subject;

impl NativeRoot {
    pub(crate) fn current_subjects(&self) -> Vec<Subject> {
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
        if let Err(error) = self.core(cx).report_subjects(
            &self.window_label,
            self.current_subjects(),
            self.tabs
                .active()
                .filter(|_| self.route == AppRoute::Chat)
                .and_then(PaneLayout::focused_session_id),
        ) {
            self.error = Some(error.to_string());
        }
        checkpoint_window_layout_deferred(cx);
    }

    pub(crate) fn sync_window_activation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        self.report_current_subjects(cx);
        if window.is_window_active() {
            if let Err(error) = self.core(cx).mark_focused(&self.window_label) {
                self.error = Some(error.to_string());
            }
            if self.route == AppRoute::Chat {
                self.mark_active_tab_viewed(window, cx);
            }
        } else {
            let _ = self.core(cx).mark_blurred(&self.window_label);
        }
        self.sync_subject_ownership(window, cx);
        if matches!(self.route, AppRoute::Mission(_)) {
            self.mission_workspace.update(cx, |workspace, cx| {
                workspace.mark_active_session_viewed(window, cx);
            });
        }
        cx.notify();
    }

    pub(crate) fn start_focus_map_listener(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (focus_tx, mut focus_rx) = futures::channel::mpsc::unbounded::<bool>();
        let mut events = self.core(cx).subscribe();
        cx.background_spawn(async move {
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
        })
        .detach();
        cx.spawn_in(window, async move |weak, cx| {
            while let Some(mut reconnected) = focus_rx.next().await {
                while let Ok(next) = focus_rx.try_recv() {
                    reconnected |= next;
                }
                if weak
                    .update_in(cx, |this, window, cx| {
                        if reconnected && !this.closing {
                            if let Err(error) = this.core(cx).window_register(&this.window_label) {
                                this.error = Some(error.to_string());
                            }
                            this.sync_window_activation(window, cx);
                        }
                        this.app_store.update(cx, |store, cx| {
                            store.window_entries =
                                store.client.window_snapshot().unwrap_or_default();
                            cx.notify();
                        });
                        this.sync_subject_ownership(window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn sync_subject_ownership(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        self.sync_chat_subject_ownership(window, cx);
        self.sync_mission_subject_ownership(window, cx);
    }

    fn sync_chat_subject_ownership(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.route == AppRoute::Chat {
            if let Err(error) = self.ensure_owned_active_tab_attached(window, cx) {
                self.chat_error = Some(error.to_string());
            }
        } else {
            self.refresh_chat_secondaries(cx);
            self.attached.clear();
        }
        cx.notify();
    }

    pub(crate) fn refresh_chat_secondaries(&mut self, cx: &App) {
        let active_ids = self
            .tabs
            .active()
            .filter(|_| self.route == AppRoute::Chat)
            .map(PaneLayout::session_ids)
            .unwrap_or_default();
        let entries = self.core(cx).window_snapshot().unwrap_or_default();
        let next = active_ids
            .iter()
            .filter_map(|session_id| {
                let state = runner_core::protocol::window::is_secondary_for(
                    &entries,
                    &self.window_label,
                    &Subject::DirectChat(session_id.clone()),
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
    }

    pub(crate) fn chat_secondary_state(
        &self,
        session_id: &str,
        cx: &App,
    ) -> runner_core::protocol::window::SecondaryState {
        runner_core::protocol::window::is_secondary_for(
            &self.app_store.read(cx).window_entries.clone(),
            &self.window_label,
            &Subject::DirectChat(session_id.to_owned()),
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
            AppRoute::ArchivedChat => None,
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
        self.save_main_window_state(window, cx);
        self.save_settings(cx);
        self.attached.clear();
        self.mission_workspace
            .update(cx, |workspace, workspace_cx| {
                workspace.release_window(workspace_cx)
            });
        let _ = self.core(cx).unregister(&self.window_label);
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
