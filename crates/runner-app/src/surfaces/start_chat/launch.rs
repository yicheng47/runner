use super::*;
impl NativeRoot {
    pub(crate) fn finish_local_chat_launch(
        &mut self,
        spawned_id: Option<String>,
        result: runner_core::protocol::ClientResult<
            crate::surfaces::front_page::client::ChatLaunched,
        >,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = result.map_err(anyhow::Error::from).and_then(|output| {
            self.refresh_sessions(cx);
            self.reload_tabs(cx)?;
            self.tabs.activate_session(&output.session);
            self.sync_active_project_from_active_tab(cx);
            self.set_route(AppRoute::Chat, cx);
            self.ensure_active_tab_attached(window, cx)?;
            Ok(output.session)
        });
        match result {
            Ok(session_id) => {
                let open = crate::surfaces::side_panel::OpenSurfaces::new_chat(self.settings(cx));
                self.chat_surfaces.reset(open);
                self.reveal_started_scope(
                    &ProjectScope::or_root(self.active_tab_project_id(cx)),
                    cx,
                );
                self.start_chat_modal = None;
                self.error = None;
                self.remember_active_role(cx);
                self.mark_active_tab_viewed(window, cx);
                self.sync_active_chat_detail(cx);
                self.begin_chat_transition(
                    &session_id,
                    chat_lifecycle::TransitionKind::Starting,
                    Some(0),
                    window,
                    cx,
                );
                self.focus_empty_chat_panel(window, cx);
            }
            Err(start_error) => {
                if let Some(session_id) = spawned_id {
                    self.start_chat_modal = None;
                    let _ = self.reload_tabs(cx);
                    self.tabs.activate_session(&session_id);
                    self.sync_active_project_from_active_tab(cx);
                    self.set_route(AppRoute::Chat, cx);
                    let _ = self.ensure_active_tab_attached(window, cx);
                    self.remember_active_role(cx);
                    self.mark_active_tab_viewed(window, cx);
                    self.sync_active_chat_detail(cx);
                    self.begin_chat_transition(
                        &session_id,
                        chat_lifecycle::TransitionKind::Starting,
                        Some(0),
                        window,
                        cx,
                    );
                    self.error = Some(start_error.to_string().into());
                } else {
                    self.chat_error = Some(start_error.to_string().into());
                }
            }
        }
        cx.notify();
    }
}
