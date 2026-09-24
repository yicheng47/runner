use gpui::prelude::*;
use gpui::{App, DragMoveEvent, Window};
use runner_backend::model::SessionStatus;
use runner_backend::ops::project::ProjectScope;

use super::*;
use crate::*;

impl MissionWorkspace {
    pub(crate) fn toggle_terminal_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !mission_drawer_available(self.archived(), self.secondary_state(cx).secondary)
            || self.mission.is_none()
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
        match self.persist_mission_layout(cx) {
            Ok(()) => {
                self.error = None;
                let layout = self.layout.clone();
                if let Err(error) = self.resume_visible_drawer_shell_on_launch(&layout, window, cx)
                {
                    self.error = Some(error.to_string());
                }
                if self.layout.drawer.open() {
                    if let Some(session_id) = self.layout.drawer.active_shell() {
                        self.focus_mission_drawer_terminal(session_id, window, cx);
                    }
                } else {
                    self.focus_active_mission_terminal(window, cx);
                }
            }
            Err(error) => {
                self.layout = original;
                self.error = Some(error.to_string());
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
        match self.persist_mission_layout(cx) {
            Ok(()) => {
                self.error = None;
                let layout = self.layout.clone();
                if let Err(error) = self.resume_visible_drawer_shell_on_launch(&layout, window, cx)
                {
                    self.error = Some(error.to_string());
                }
                self.focus_mission_drawer_terminal(session_id, window, cx);
            }
            Err(error) => {
                self.layout = original;
                self.error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    pub(crate) fn add_terminal_drawer_shell(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !mission_drawer_available(self.archived(), self.secondary_state(cx).secondary) {
            return;
        }
        let Some(mission) = self.mission.clone() else {
            return;
        };
        let original = self.layout.clone();
        let project_cwd = mission.project_id.as_deref().and_then(|project_id| {
            self.app_store
                .read(cx)
                .projects
                .iter()
                .find(|project| project.id == project_id)
                .map(|project| project.cwd.as_str())
        });
        let cwd = super::start_chat::terminal_working_dir(
            mission.cwd.as_deref(),
            project_cwd,
            &self.settings(cx).default_working_dir,
            runner_backend::app_paths::home_dir()
                .as_deref()
                .and_then(|home| home.to_str()),
        );
        let size = self.estimated_mission_drawer_terminal_size(window, cx);
        let mut spawned_id = None;
        let result = (|| -> Result<String> {
            let spawned = runner_backend::ops::session::session_start_shell_in(
                self.core(cx),
                ProjectScope::or_root(mission.project_id),
                cwd,
                Some(size.0),
                Some(size.1),
            )?;
            spawned_id = Some(spawned.id.clone());
            self.app_store
                .update(cx, |store, store_cx| store.refresh_sessions(store_cx));
            self.layout.drawer.add(spawned.id.clone());
            self.persist_mission_layout(cx)?;
            self.app_store
                .update(cx, |store, store_cx| store.refresh_nodes(store_cx))?;
            self.ensure_mission_terminal_attached(&spawned.id, SessionStatus::Running, window, cx)?;
            Ok(spawned.id)
        })();

        match result {
            Ok(session_id) => {
                self.error = None;
                self.drawer_exit_codes.remove(&session_id);
                self.begin_mission_transition(
                    &session_id,
                    MissionTransitionKind::Starting,
                    Some(0),
                    window,
                    cx,
                );
                self.focus_mission_drawer_terminal(&session_id, window, cx);
            }
            Err(error) => {
                if let Some(session_id) = spawned_id {
                    let _ = runner_backend::ops::session::session_close(self.core(cx), &session_id);
                }
                self.layout = original;
                let _ = self.persist_mission_layout(cx);
                self.refresh_store(StoreRefreshKind::All, cx);
                self.error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    pub(super) fn resume_terminal_drawer_shell(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resume_terminal_drawer_shell_impl(session_id, false, window, cx);
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
        if self.transitions.contains_key(session_id) {
            return;
        }
        let size = self
            .attached
            .get(session_id)
            .map(|chat| chat.terminal.size())
            .unwrap_or_else(|| self.estimated_mission_drawer_terminal_size(window, cx));
        self.begin_mission_transition(
            session_id,
            MissionTransitionKind::Resuming,
            None,
            window,
            cx,
        );
        self.error = None;
        self.drawer_exit_codes.remove(session_id);
        let core = self.core(cx).clone();
        let target = session_id.to_owned();
        let resume_target = target.clone();
        let resume = cx.background_spawn(async move {
            if launch_claim {
                runner_backend::ops::session::session_resume_on_launch(
                    &core,
                    &resume_target,
                    Some(size.0),
                    Some(size.1),
                )
            } else {
                runner_backend::ops::session::session_resume(
                    &core,
                    &resume_target,
                    Some(size.0),
                    Some(size.1),
                )
            }
            .map(drop)
            .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = resume.await;
            let _ = weak.update_in(cx, |this, window, cx| {
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
                            this.error = Some(error.to_string());
                        }
                        this.focus_mission_drawer_terminal(&target, window, cx);
                    }
                    Err(error) => {
                        this.transitions.remove(&target);
                        this.error = Some(error);
                        this.refresh_store(StoreRefreshKind::All, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn close_terminal_drawer_shell(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.closing_drawer_shells.insert(session_id.to_owned()) {
            return;
        }
        self.error = None;
        let core = self.core(cx).clone();
        let target = session_id.to_owned();
        let close_target = target.clone();
        let close = cx.background_spawn(async move {
            runner_backend::ops::session::session_close(&core, &close_target)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = close.await;
            let _ = weak.update_in(cx, |this, window, cx| {
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
                            this.error = Some(error.to_string());
                        }
                        if let Some(active) = this.layout.drawer.active_shell().map(str::to_owned) {
                            this.focus_mission_drawer_terminal(&active, window, cx);
                        } else {
                            this.focus_active_mission_terminal(window, cx);
                        }
                    }
                    Err(error) => this.error = Some(error),
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
        match runner_backend::ops::session::session_shell_has_foreground_process(
            self.core(cx),
            session_id,
        ) {
            Ok(true) => {
                if let Some(shell) = self.shell.upgrade() {
                    let session_id = session_id.to_owned();
                    shell.update(cx, |shell, shell_cx| {
                        shell.terminal_close_confirm = Some(TerminalCloseConfirm {
                            target: TerminalCloseTarget::MissionDrawer { session_id },
                        });
                        shell_cx.notify();
                    });
                }
            }
            Ok(false) => self.close_terminal_drawer_shell(session_id, window, cx),
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
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
            self.error = Some(error.to_string());
        }
        cx.notify();
    }

    pub(super) fn mission_terminal_interactive(&self, session_id: &str, cx: &App) -> bool {
        self.is_active(cx)
            && !self.secondary_state(cx).secondary
            && !self.archiving
            && self.terminal_status(session_id, cx) == Some(SessionStatus::Running)
            && self.transition_kind(session_id).is_none()
    }

    pub(super) fn cached_mission_terminal_interactive(&self, session_id: &str, cx: &App) -> bool {
        self.is_active(cx)
            && !self.secondary
            && !self.archiving
            && self.terminal_status(session_id, cx) == Some(SessionStatus::Running)
            && self.transition_kind(session_id).is_none()
    }

    fn terminal_status(&self, session_id: &str, cx: &App) -> Option<SessionStatus> {
        self.sessions
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
