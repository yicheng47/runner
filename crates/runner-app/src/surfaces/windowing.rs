use super::*;
use crate::*;
use runner_backend::windows::Subject;

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
        if let Err(error) = runner_backend::ops::window::report_subjects(
            self.core(cx),
            &self.window_label,
            self.current_subjects(),
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
            if let Err(error) =
                runner_backend::ops::window::mark_focused(self.core(cx), &self.window_label)
            {
                self.error = Some(error.to_string());
            }
            if self.route == AppRoute::Chat {
                self.mark_active_tab_viewed(window, cx);
            }
        } else {
            runner_backend::ops::window::mark_blurred(self.core(cx), &self.window_label);
        }
        self.sync_subject_ownership(window, cx);
        cx.notify();
    }

    pub(crate) fn start_focus_map_listener(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (focus_tx, mut focus_rx) = futures::channel::mpsc::unbounded::<()>();
        let mut events = self.core(cx).events.subscribe();
        cx.background_spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) if event.name == "window_focus_map" => {
                        if focus_tx.unbounded_send(()).is_err() {
                            break;
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
        .detach();
        cx.spawn_in(window, async move |weak, cx| {
            while focus_rx.next().await.is_some() {
                while focus_rx.try_recv().is_ok() {}
                if weak
                    .update_in(cx, |this, window, cx| {
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
        let entries = self.core(cx).windows.snapshot();
        let next = active_ids
            .iter()
            .filter_map(|session_id| {
                let state = runner_backend::ops::window::is_secondary_for(
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
    ) -> runner_backend::ops::window::SecondaryState {
        runner_backend::ops::window::is_secondary_for(
            &self.core(cx).windows.snapshot(),
            &self.window_label,
            &Subject::DirectChat(session_id.to_owned()),
        )
    }

    pub(crate) fn cached_chat_secondary_state(
        &self,
        session_id: &str,
    ) -> runner_backend::ops::window::SecondaryState {
        let primary_label = self.chat_secondaries.get(session_id).cloned();
        runner_backend::ops::window::SecondaryState {
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
            AppRoute::Runners => Some("/runners".into()),
            AppRoute::RunnerDetail(handle) => Some(format!("/runners/{handle}")),
            AppRoute::Crews => Some("/crews".into()),
            AppRoute::CrewEditor(crew_id) => Some(format!("/crews/{crew_id}")),
            AppRoute::Mission(mission_id) => Some(format!("/missions/{mission_id}")),
            AppRoute::Settings => Some("/settings".into()),
            AppRoute::ArchivedChat => None,
        }
    }

    pub(crate) fn prepare_window_close(&mut self, window: &Window, cx: &mut Context<Self>) {
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
        runner_backend::ops::window::unregister(self.core(cx), &self.window_label);
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
        let mut layout = PaneLayout::fresh(PresetKind::Cols2, Some("chat-a"), &["chat-a".into()]);
        let empty = layout
            .root
            .leaves()
            .into_iter()
            .find(|leaf| leaf.session_id.is_none())
            .unwrap()
            .id
            .clone();
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
