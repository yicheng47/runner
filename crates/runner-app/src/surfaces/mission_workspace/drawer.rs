use gpui::prelude::*;
use gpui::{App, DragMoveEvent, Window};
use runner_core::protocol::model::SessionStatus;

use super::*;
use crate::*;

#[derive(Clone)]
struct RemoteDrawerCloseFocus {
    session_id: String,
    generation: u64,
    target: crate::surfaces::front_page::FrontPageTarget,
    origin_focus: Option<gpui::FocusHandle>,
}

impl RemoteDrawerCloseFocus {
    fn defer_restore(
        self,
        workspace: &mut MissionWorkspace,
        window: &mut Window,
        cx: &mut Context<MissionWorkspace>,
    ) {
        workspace.drawer_close_focus_token = workspace.drawer_close_focus_token.wrapping_add(1);
        let token = workspace.drawer_close_focus_token;
        workspace.drawer_close_focus_subscription = None;
        cx.defer_in(window, move |this, window, cx| {
            if this.drawer_close_focus_token != token {
                return;
            }
            if self.restore_if_closed(this, window, cx) {
                return;
            }
            // A remote close acknowledgement can arrive before the host pushes the updated overview.
            let clients = this.app_store.read(cx).remote_clients.clone();
            this.drawer_close_focus_subscription =
                Some(cx.observe_in(&clients, window, move |_, _, window, cx| {
                    let focus = self.clone();
                    cx.defer_in(window, move |this, window, cx| {
                        if this.drawer_close_focus_token != token {
                            return;
                        }
                        if focus.restore_if_closed(this, window, cx) {
                            this.drawer_close_focus_token =
                                this.drawer_close_focus_token.wrapping_add(1);
                            this.drawer_close_focus_subscription = None;
                        }
                    });
                }));
        });
    }

    fn restore_if_closed(
        &self,
        this: &mut MissionWorkspace,
        window: &mut Window,
        cx: &mut Context<MissionWorkspace>,
    ) -> bool {
        if !this.active
            || this.generation != self.generation
            || this.client.target != self.target
            || !this.remote_dialogs_clear()
        {
            return true;
        }
        let shell_allows = this.shell.upgrade().is_none_or(|shell| {
            let root = shell.read(cx);
            root.allows_remote_terminal_auto_focus_window(cx)
                && root.remote_client.allows_auto_terminal_focus(None)
        });
        if !shell_allows {
            return true;
        }
        let focused = window.focused(cx);
        let dropped_focus = focused.as_ref().is_some_and(|handle| {
            this.shell.upgrade().is_some_and(|shell| {
                shell
                    .read(cx)
                    .remote_client
                    .allows_auto_terminal_focus(Some(handle))
            })
        });
        let focus_eligible = focused == self.origin_focus
            || focused.is_none()
            || focused.as_ref() == Some(&this.drawer_focus)
            || dropped_focus;
        if !focus_eligible {
            return true;
        }
        if this.layout.drawer.shells().contains(&self.session_id) {
            return false;
        }
        if this.layout.drawer.open() {
            if let Some(active) = this.layout.drawer.active_shell().map(str::to_owned) {
                this.drawer_was_focused = true;
                this.focus_mission_drawer_terminal(&active, window, cx);
                return true;
            }
        }
        this.drawer_was_focused = false;
        this.focus_active_mission_terminal(window, cx);
        true
    }
}

impl MissionWorkspace {
    pub(crate) fn toggle_terminal_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !mission_drawer_available(self.archived(), self.secondary_state(cx).secondary)
            || self.mission_view().is_none()
        {
            return;
        }
        if self.layout.drawer.open() {
            self.hide_terminal_drawer(window, cx);
        } else if self.layout.drawer.shells().is_empty() {
            self.add_terminal_drawer_shell(window, cx);
        } else {
            let original = self.layout.clone();
            self.layout.drawer.set_open(true);
            self.finish_terminal_drawer_update(original, window, cx);
        }
    }

    pub(crate) fn hide_terminal_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drawer_was_focused = false;
        let original = self.layout.clone();
        self.layout.drawer.set_open(false);
        self.finish_terminal_drawer_update(original, window, cx);
    }

    fn finish_terminal_drawer_update(
        &mut self,
        original: MissionLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.presentation.is_remote() {
            self.remote_navigation_changed(cx);
        }
        match self.persist_mission_layout(cx) {
            Ok(()) => {
                self.error = None;
                let layout = self.layout.clone();
                if let Err(error) = self.resume_visible_drawer_shell_on_launch(&layout, window, cx)
                {
                    self.error = Some(error.to_string().into());
                }
                if self.layout.drawer.open() {
                    if let Some(session_id) = self.layout.drawer.active_shell() {
                        self.drawer_was_focused = true;
                        self.focus_mission_drawer_terminal(session_id, window, cx);
                    }
                } else {
                    self.focus_active_mission_terminal(window, cx);
                }
            }
            Err(error) => {
                self.layout = original;
                self.error = Some(error.to_string().into());
            }
        }
        cx.notify();
    }

    pub(super) fn activate_terminal_drawer_shell(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let original = self.layout.clone();
        if !self.layout.drawer.activate(session_id) {
            return;
        }
        if self.presentation.is_remote() {
            self.remote_navigation_changed(cx);
        }
        match self.persist_mission_layout(cx) {
            Ok(()) => {
                self.error = None;
                let layout = self.layout.clone();
                if let Err(error) = self.resume_visible_drawer_shell_on_launch(&layout, window, cx)
                {
                    self.error = Some(error.to_string().into());
                }
                self.drawer_was_focused = true;
                self.focus_mission_drawer_terminal(session_id, window, cx);
            }
            Err(error) => {
                self.layout = original;
                self.error = Some(error.to_string().into());
            }
        }
        cx.notify();
    }

    pub(crate) fn add_terminal_drawer_shell(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote_target = self.client.is_remote();
        if !mission_drawer_available(self.archived(), self.secondary_state(cx).secondary)
            || !self.remote_supports(&["startShell", "layout"])
        {
            return;
        }
        let Some(mission) = self.mission_view() else {
            return;
        };
        let original = self.layout.clone();
        let project_cwd = mission.project_id.as_deref().and_then(|project| {
            self.app_store
                .read(cx)
                .projects
                .iter()
                .find(|item| item.id == project)
                .map(|item| item.cwd.as_str())
        });
        let cwd = if remote_target {
            mission.cwd.clone()
        } else {
            super::start_chat::terminal_working_dir(
                mission.cwd.as_deref(),
                project_cwd,
                &self.settings(cx).default_working_dir,
                runner_core::app_paths::home_dir()
                    .as_deref()
                    .and_then(|home| home.to_str()),
            )
        };
        let size = self.estimated_mission_drawer_terminal_size(window, cx);
        let before = self
            .remote_summary()
            .map(|summary| summary.shells.clone())
            .unwrap_or_default();
        let stage = if remote_target {
            let Some(stage) = self.presentation.begin_shell() else {
                return;
            };
            Some(stage)
        } else {
            None
        };
        let client = self.client.clone();
        let guard = client.clone();
        let generation = self.generation;
        let mut shells = before.clone();
        let operation = move || {
            let existing = stage
                .as_ref()
                .and_then(|stage| stage.lock().unwrap().clone());
            let id = if let Some(id) = existing {
                id
            } else {
                let id = client.start_shell(mission.project_id, cwd, None, size)?;
                if let Some(stage) = &stage {
                    *stage.lock().unwrap() = Some(id.clone());
                }
                id
            };
            if remote_target {
                if !shells.contains(&id) {
                    shells.push(id.clone());
                }
                client.mission_layout(mission.id, shells.clone())?;
            }
            Ok::<_, runner_core::protocol::ClientError>((id, shells))
        };
        if remote_target {
            let task = cx.background_spawn(async move { operation() });
            cx.spawn(async move |weak, cx| {
                let result = task.await;
                let _ = weak.update(cx, |this: &mut Self, cx| {
                    if this.generation != generation || !guard.valid() {
                        return;
                    }
                    if this.client.target != guard.target {
                        return;
                    }
                    let Some(result) = this.presentation.complete_shell(
                        this.mission_id.as_deref(),
                        before,
                        &mut this.layout,
                        result,
                    ) else {
                        return;
                    };
                    match result {
                        Ok(()) => {
                            this.error = None;
                            this.remote_navigation_changed(cx);
                        }
                        Err(error) => {
                            this.error = Some(action_failure(
                                crate::i18n::message("mission.shell_action"),
                                error.to_string(),
                            ))
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
            cx.notify();
        } else {
            let result = operation().map(|(id, _)| id).map_err(anyhow::Error::from);
            let spawned_id = result.as_ref().ok().cloned();
            let this = self;
            let result = result.and_then(|spawned| {
                this.app_store
                    .update(cx, |store, store_cx| store.refresh_sessions(store_cx));
                this.layout.drawer.add(spawned.clone());
                this.persist_mission_layout(cx)?;
                this.app_store
                    .update(cx, |store, store_cx| store.refresh_nodes(store_cx))?;
                this.ensure_mission_terminal_attached(
                    &spawned,
                    SessionStatus::Running,
                    window,
                    cx,
                )?;
                Ok(spawned)
            });

            match result {
                Ok(session_id) => {
                    this.error = None;
                    this.drawer_exit_codes.remove(&session_id);
                    this.begin_mission_transition(
                        &session_id,
                        MissionTransitionKind::Starting,
                        Some(0),
                        window,
                        cx,
                    );
                    this.focus_mission_drawer_terminal(&session_id, window, cx);
                }
                Err(error) => {
                    if let Some(session_id) = spawned_id {
                        let _ = this.client.close(&session_id);
                    }
                    this.layout = original;
                    let _ = this.persist_mission_layout(cx);
                    this.refresh_store(StoreRefreshKind::All, cx);
                    this.error = Some(error.to_string().into());
                }
            }
            cx.notify();
        }
    }

    pub(super) fn resume_terminal_drawer_shell_on_launch(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resume_terminal_drawer_shell_impl(session_id, true, window, cx);
    }

    fn resume_terminal_drawer_shell_impl(
        &mut self,
        session_id: &str,
        launch_claim: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote = self.client.is_remote();
        if remote
            && !self
                .layout
                .drawer
                .shells()
                .iter()
                .any(|id| id == session_id)
        {
            return;
        }
        let size = if remote {
            self.estimated_mission_drawer_terminal_size(window, cx)
        } else {
            self.attached
                .get(session_id)
                .map(|chat| chat.terminal.size())
                .unwrap_or_else(|| self.estimated_mission_drawer_terminal_size(window, cx))
        };
        if !remote {
            if self.transitions.contains_key(session_id) {
                return;
            }
            self.begin_mission_transition(
                session_id,
                MissionTransitionKind::Resuming,
                None,
                window,
                cx,
            );
            self.error = None;
            self.drawer_exit_codes.remove(session_id);
        }
        let Some(operation) =
            self.prepare_mission_action("resume", format!("shell-resume:{session_id}"))
        else {
            return;
        };
        let core = self.client.clone();
        let target = session_id.to_owned();
        let resume_target = target.clone();
        let resume = cx.background_spawn(async move {
            core.resume_session(&resume_target, size, launch_claim)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = resume.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let Some(result) =
                    this.complete_mission_action(operation, result, |id| vec![id], cx)
                else {
                    return;
                };
                let result = result.map(drop);
                match result {
                    Ok(()) => {
                        this.refresh_store(StoreRefreshKind::All, cx);
                        if let Err(error) = this.ensure_mission_terminal_attached(
                            &target,
                            SessionStatus::Running,
                            window,
                            cx,
                        ) {
                            this.transitions.remove(&target);
                            this.error = Some(error.to_string().into());
                        }
                        this.focus_mission_drawer_terminal(&target, window, cx);
                    }
                    Err(error) => {
                        this.transitions.remove(&target);
                        this.error = Some(error.into());
                        this.refresh_store(StoreRefreshKind::All, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn close_focused_terminal_drawer_shell(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session_id) = self.layout.drawer.active_shell().map(str::to_owned) {
            self.request_close_terminal_drawer_shell(&session_id, window, cx);
        } else {
            self.hide_terminal_drawer(window, cx);
        }
    }

    pub(crate) fn close_terminal_drawer_shell(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote = self.client.is_remote();
        if remote
            && (!self.presentation.drawer_actions_available()
                || !self
                    .layout
                    .drawer
                    .shells()
                    .iter()
                    .any(|id| id == session_id))
        {
            return;
        }
        if !remote && self.closing_drawer_shells.contains(session_id) {
            return;
        }
        let Some(operation) =
            self.prepare_mission_action("close", format!("shell-close:{session_id}"))
        else {
            return;
        };
        if !remote {
            self.closing_drawer_shells.insert(session_id.to_owned());
            self.error = None;
        }
        let origin_focus = window.focused(cx);
        let origin_in_drawer = self.drawer_focused(window, cx);
        let mut remote_focus = (remote && origin_in_drawer).then(|| RemoteDrawerCloseFocus {
            session_id: session_id.to_owned(),
            generation: self.generation,
            target: self.client.target.clone(),
            origin_focus: origin_focus.clone(),
        });
        let core = self.client.clone();
        let target = session_id.to_owned();
        let close_target = target.clone();
        let close = cx.background_spawn(async move {
            core.close(&close_target).map_err(|error| error.to_string())
        });
        #[cfg(test)]
        let test_gate = self.drawer_close_gates.pop_front();
        cx.spawn_in(window, async move |weak, cx| {
            let result = close.await;
            #[cfg(test)]
            let applied = match test_gate {
                Some(gate) => Some(gate.wait().await),
                None => None,
            };
            let _ = weak.update_in(cx, |this, window, cx| {
                #[cfg(test)]
                let _applied = applied;
                let Some(result) = this.complete_mission_action_observed(
                    operation,
                    result,
                    |_| Vec::new(),
                    |this, succeeded, cx| {
                        if remote && succeeded {
                            if let Some(focus) = remote_focus.take() {
                                focus.defer_restore(this, window, cx);
                            }
                        }
                    },
                    cx,
                ) else {
                    return;
                };
                this.closing_drawer_shells.remove(&target);
                match result {
                    Ok(()) => {
                        this.attached.remove(&target);
                        this.drawer_exit_codes.remove(&target);
                        this.transitions.remove(&target);
                        this.layout.drawer.remove(&target);
                        this.refresh_store(StoreRefreshKind::All, cx);
                        let layout = this.layout.clone();
                        if let Err(error) =
                            this.resume_visible_drawer_shell_on_launch(&layout, window, cx)
                        {
                            this.error = Some(error.to_string().into());
                        }
                        if !this.layout.drawer.open() || this.layout.drawer.active_shell().is_none()
                        {
                            this.drawer_was_focused = false;
                        }
                        let in_settings = this
                            .shell
                            .upgrade()
                            .is_some_and(|shell| shell.read(cx).route == AppRoute::Settings);
                        let focused = window.focused(cx);
                        let restore_focus = origin_in_drawer
                            && this.is_active(cx)
                            && !in_settings
                            && (focused == origin_focus
                                || focused.is_none()
                                || focused.as_ref() == Some(&this.drawer_focus));
                        if restore_focus {
                            if this.layout.drawer.open() {
                                if let Some(active) =
                                    this.layout.drawer.active_shell().map(str::to_owned)
                                {
                                    this.drawer_was_focused = true;
                                    this.focus_mission_drawer_terminal(&active, window, cx);
                                } else {
                                    this.drawer_was_focused = false;
                                    this.focus_active_mission_terminal(window, cx);
                                }
                            } else {
                                this.drawer_was_focused = false;
                                this.focus_active_mission_terminal(window, cx);
                            }
                        }
                    }
                    Err(error) => this.error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn request_close_terminal_drawer_shell(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_terminal_drawer_shell(session_id, window, cx);
    }

    pub(super) fn resize_terminal_drawer(
        &mut self,
        event: &DragMoveEvent<DrawerResizeDrag>,
        cx: &mut Context<Self>,
    ) {
        let height =
            f32::from(event.bounds.bottom() - event.event.position.y) / self.settings(cx).app_zoom;
        self.layout.drawer.set_height(height);
        self.drawer_resizing = true;
        cx.notify();
    }

    pub(super) fn finish_terminal_drawer_resize(&mut self, cx: &mut Context<Self>) {
        if !self.drawer_resizing {
            return;
        }
        self.drawer_resizing = false;
        if let Err(error) = self.persist_mission_layout(cx) {
            self.error = Some(error.to_string().into());
        }
        cx.notify();
    }

    pub(super) fn mission_terminal_interactive(&self, session_id: &str, cx: &App) -> bool {
        self.is_active(cx)
            && self
                .attached
                .get(session_id)
                .is_none_or(|chat| chat.terminal.control().desktop())
            && !self.secondary_state(cx).secondary
            && !self.archiving
            && self.terminal_status(session_id, cx) == Some(SessionStatus::Running)
            && self.transition_kind(session_id).is_none()
    }

    pub(super) fn cached_mission_terminal_interactive(&self, session_id: &str, cx: &App) -> bool {
        self.is_active(cx)
            && self
                .attached
                .get(session_id)
                .is_none_or(|chat| chat.terminal.control().desktop())
            && !self.secondary
            && !self.archiving
            && self.terminal_status(session_id, cx) == Some(SessionStatus::Running)
            && self.transition_kind(session_id).is_none()
    }

    pub(super) fn terminal_status(&self, session_id: &str, cx: &App) -> Option<SessionStatus> {
        self.members()
            .iter()
            .find(|session| session.session.id == session_id)
            .map(|session| session.session.status)
            .or_else(|| {
                self.layout
                    .drawer
                    .shells()
                    .iter()
                    .any(|shell_id| shell_id == session_id)
                    .then(|| {
                        self.drawer_session_entry(session_id, cx)
                            .map(|entry| entry.status)
                    })
                    .flatten()
            })
    }
}
