use std::collections::HashSet;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, KeyDownEvent, Window};
use runner_app::ui::{MenuItem as UiMenuItem, SessionControlKind, TextField};
use runner_backend::model::SessionStatus;

use super::*;
use crate::*;

impl MissionWorkspace {
    pub(super) fn configure_mission_action_menu(&mut self, cx: &mut Context<Self>) {
        let Some(mission) = self.mission.as_ref() else {
            return;
        };
        let busy = self.lifecycle_busy() || self.secondary;
        let pinned = mission.pinned_at.is_some();
        self.menu_actions = vec![
            MissionMenuAction::Pin,
            MissionMenuAction::Rename,
            MissionMenuAction::Archive,
        ];
        self.action_menu.update(cx, |menu, menu_cx| {
            menu.set_items(
                vec![
                    UiMenuItem::new(if pinned { "Unpin" } else { "Pin" })
                        .icon(if pinned { "pin-off.svg" } else { "pin.svg" })
                        .disabled(busy),
                    UiMenuItem::new("Rename")
                        .icon("square-pen.svg")
                        .disabled(busy),
                    UiMenuItem::new("Archive")
                        .icon("archive.svg")
                        .destructive(true)
                        .disabled(busy),
                ],
                menu_cx,
            )
        });
    }

    pub(super) fn handle_mission_menu_action(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.secondary_state(cx).secondary {
            return;
        }
        let Some(action) = self.menu_actions.get(index).cloned() else {
            return;
        };
        match action {
            MissionMenuAction::Pin => self.toggle_mission_pin(window, cx),
            MissionMenuAction::Rename => self.open_mission_rename(window, cx),
            MissionMenuAction::Archive => self.archive_open_mission(window, cx),
        }
    }

    fn toggle_mission_pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mission) = self.mission.clone() else {
            return;
        };
        let mission_id = mission.id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_pin_impl(
                &core,
                mission.id,
                mission.pinned_at.is_none(),
            )
            .await
            .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, _, cx| {
                if this.mission_id.as_deref() != Some(mission_id.as_str()) {
                    return;
                }
                match result {
                    Ok(mission) => {
                        this.mission = Some(mission);
                        this.refresh_store(StoreRefreshKind::All, cx);
                        this.core(cx).events.emit(
                            "mission/changed",
                            &serde_json::json!({ "mission_id": mission_id }),
                        );
                    }
                    Err(error) => {
                        this.error = Some(action_failure("update the mission pin", error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn request_slot_action(
        &mut self,
        session_id: &str,
        action: SessionControlKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action == SessionControlKind::Restart {
            self.open_restart_confirm(session_id, window, cx);
        } else {
            self.act_on_slot(session_id, action, window, cx);
        }
    }

    fn open_restart_confirm(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.stopping
            || self.resuming
            || self.archiving
            || !mission_slot_actions_available(
                self.mission.as_ref().map(|mission| mission.status),
                self.archived(),
                self.secondary_state(cx).secondary,
            )
            || self.slot_actions.contains(session_id)
            || self.transitions.contains_key(session_id)
        {
            return;
        }
        self.restart_confirm = Some(session_id.to_owned());
        self.root_focus.focus(window);
        cx.notify();
    }

    pub(super) fn render_restart_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let session_id = self.restart_confirm.clone().unwrap_or_default();
        let target = self
            .sessions
            .iter()
            .find(|session| session.session.id == session_id);
        let handle = target
            .map(|session| session.handle.clone())
            .unwrap_or_default();
        let lead_handle = self
            .sessions
            .iter()
            .find(|session| session.lead)
            .map(|session| session.handle.clone())
            .unwrap_or_default();
        let confirm = cx.entity();
        let cancel = confirm.clone();
        let restart_id = session_id.clone();
        runner_app::ui::ConfirmDialog::new(
            format!("Restart @{handle}?"),
            restart_confirm_body(
                &handle,
                &lead_handle,
                target.is_some_and(|session| session.lead),
            ),
            "Restart",
            "Restarting…",
            false,
            Rc::new(move |window, cx| {
                confirm.update(cx, |this, cx| {
                    this.restart_confirm = None;
                    this.act_on_slot(&restart_id, SessionControlKind::Restart, window, cx);
                })
            }),
            Rc::new(move |window, cx| {
                cancel.update(cx, |this, cx| {
                    this.restart_confirm = None;
                    this.focus_active_mission_terminal(window, cx);
                    cx.notify();
                })
            }),
        )
        .icon("rotate-ccw.svg")
        .into_any_element()
    }

    pub(super) fn act_on_slot(
        &mut self,
        session_id: &str,
        action: SessionControlKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.stopping
            || self.resuming
            || self.archiving
            || !mission_slot_actions_available(
                self.mission.as_ref().map(|mission| mission.status),
                self.archived(),
                self.secondary_state(cx).secondary,
            )
            || self.slot_actions.contains(session_id)
            || self.transitions.contains_key(session_id)
        {
            return;
        }
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        self.slot_actions.insert(session_id.to_owned());
        let generation = self.generation;
        if action != SessionControlKind::Stop {
            self.begin_mission_transition(
                session_id,
                if action == SessionControlKind::Restart {
                    MissionTransitionKind::Restarting
                } else {
                    MissionTransitionKind::Resuming
                },
                None,
                window,
                cx,
            );
        }
        let core = self.core(cx).clone();
        let target = session_id.to_owned();
        let size = self.current_mission_terminal_size(window, cx);
        cx.notify();
        let task_target = target.clone();
        let task = cx.background_spawn(async move {
            let result = match action {
                SessionControlKind::Stop => {
                    runner_backend::ops::session::session_kill(&core, &task_target)
                }
                SessionControlKind::Resume => runner_backend::ops::session::session_resume(
                    &core,
                    &task_target,
                    Some(size.0),
                    Some(size.1),
                )
                .map(|_| ()),
                SessionControlKind::Restart => runner_backend::ops::session::session_restart(
                    &core,
                    &task_target,
                    Some(size.0),
                    Some(size.1),
                )
                .map(|_| ()),
                _ => unreachable!(),
            };
            result.map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if !this.is_current(&mission_id, generation) {
                    return;
                }
                this.slot_actions.remove(&target);
                if let Err(error) = result {
                    this.transitions.remove(&target);
                    if !is_concurrent_resume_error(&error) {
                        this.error = Some(action_failure(
                            match action {
                                SessionControlKind::Stop => "stop the slot",
                                SessionControlKind::Restart => "restart the slot",
                                _ => "resume the slot",
                            },
                            error,
                        ));
                    }
                }
                this.refresh_open_mission(window, cx);
                this.refresh_store(StoreRefreshKind::All, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open_stop_all_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.lifecycle_busy() || self.secondary_state(cx).secondary {
            return;
        }
        self.stop_all_confirm = true;
        self.root_focus.focus(window);
        cx.notify();
    }

    pub(super) fn render_stop_all_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let confirm = cx.entity();
        let cancel = confirm.clone();
        runner_app::ui::ConfirmDialog::new(
            stop_all_title(
                self.sessions
                    .iter()
                    .filter(|session| session.session.status == SessionStatus::Running)
                    .count(),
            ),
            STOP_ALL_BODY,
            "Stop all",
            "Stopping…",
            false,
            Rc::new(move |window, cx| {
                confirm.update(cx, |this, cx| {
                    this.stop_all_confirm = false;
                    this.stop_open_mission(window, cx);
                })
            }),
            Rc::new(move |window, cx| {
                cancel.update(cx, |this, cx| {
                    this.stop_all_confirm = false;
                    this.focus_active_mission_terminal(window, cx);
                    cx.notify();
                })
            }),
        )
        .icon("square.svg")
        .into_any_element()
    }

    fn stop_open_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        if self.lifecycle_busy() || self.secondary_state(cx).secondary {
            return;
        }
        self.stopping = true;
        self.root_focus.focus(window);
        cx.notify();
        let core = self.core(cx).clone();
        let stop_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            let mission = runner_backend::ops::mission::mission_stop_impl(&core, stop_id.clone())
                .await
                .map_err(|error| error.to_string())?;
            let sessions = runner_backend::ops::session::session_list(&core, &stop_id)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((mission, sessions))
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.mission_id.as_deref() != Some(mission_id.as_str()) {
                    return;
                }
                this.stopping = false;
                match result {
                    Ok((mission, sessions)) => {
                        this.mission = Some(mission);
                        this.sessions = sessions;
                        let drawer_ids = this
                            .layout
                            .drawer
                            .shells()
                            .iter()
                            .cloned()
                            .collect::<HashSet<_>>();
                        this.transitions
                            .retain(|session_id, _| drawer_ids.contains(session_id));
                        this.sync_mission_copy_entities(cx);
                        this.refresh_store(StoreRefreshKind::All, cx);
                    }
                    Err(error) => {
                        this.error = Some(action_failure("stop the mission", error));
                    }
                }
                this.focus_active_mission_terminal(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn resume_open_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        if self.lifecycle_busy() || self.secondary_state(cx).secondary {
            return;
        }
        let stopped = self
            .sessions
            .iter()
            .filter(|session| session.session.status != SessionStatus::Running)
            .map(|session| session.session.id.clone())
            .collect::<Vec<_>>();
        if stopped.is_empty() {
            return;
        }
        let size = self.current_mission_terminal_size(window, cx);
        self.resuming = true;
        for session_id in &stopped {
            self.begin_mission_transition(
                session_id,
                MissionTransitionKind::Resuming,
                None,
                window,
                cx,
            );
        }
        self.root_focus.focus(window);
        cx.notify();
        let core = self.core(cx).clone();
        let resume_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            let mut sessions = runner_backend::ops::session::session_list(&core, &resume_id)
                .map_err(|error| error.to_string())?;
            let mut first_error = None;
            for candidate in sessions.clone() {
                let already_running = sessions
                    .iter()
                    .find(|session| session.session.id == candidate.session.id)
                    .is_some_and(|session| session.session.status == SessionStatus::Running);
                if already_running {
                    continue;
                }
                if let Err(error) = runner_backend::ops::session::session_resume(
                    &core,
                    &candidate.session.id,
                    Some(size.0),
                    Some(size.1),
                ) {
                    let message = error.to_string();
                    if is_concurrent_resume_error(&message) {
                        match runner_backend::ops::session::session_list(&core, &resume_id) {
                            Ok(refreshed) => sessions = refreshed,
                            Err(error) => {
                                first_error.get_or_insert_with(|| error.to_string());
                            }
                        }
                    } else {
                        first_error.get_or_insert(message);
                    }
                }
            }
            match runner_backend::ops::session::session_list(&core, &resume_id) {
                Ok(refreshed) => sessions = refreshed,
                Err(error) => {
                    first_error.get_or_insert_with(|| error.to_string());
                }
            }
            Ok::<_, String>((sessions, first_error))
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.mission_id.as_deref() != Some(mission_id.as_str()) {
                    return;
                }
                this.resuming = false;
                match result {
                    Ok((sessions, error)) => {
                        this.sessions = sessions;
                        for session in &this.sessions {
                            if !this.open_tabs.contains(&session.session.id) {
                                this.open_tabs.push(session.session.id.clone());
                            }
                        }
                        if let Some(error) = error {
                            this.error = Some(action_failure("resume the mission", error));
                        } else {
                            this.error = None;
                        }
                        this.sync_mission_copy_entities(cx);
                        if let Err(error) = this.ensure_mission_terminals_attached(window, cx) {
                            this.error = Some(error.to_string());
                        }
                        this.refresh_store(StoreRefreshKind::All, cx);
                    }
                    Err(error) => {
                        this.error = Some(action_failure("resume the mission", error));
                    }
                }
                this.focus_active_mission_terminal(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn archive_open_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        if self.lifecycle_busy() || self.secondary_state(cx).secondary {
            return;
        }
        self.archiving = true;
        self.set_sidebar_archiving(&mission_id, true, cx);
        self.root_focus.focus(window);
        cx.notify();
        let core = self.core(cx).clone();
        let archive_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_archive_impl(&core, archive_id)
                .await
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                this.set_sidebar_archiving(&mission_id, false, cx);
                if this.mission_id.as_deref() != Some(mission_id.as_str()) {
                    return;
                }
                match result {
                    Ok(_) => {
                        let removed_mission_id = mission_id.clone();
                        this.update_app_settings(cx, true, move |settings| {
                            settings
                                .last_mission_terminal_ids
                                .remove(&removed_mission_id);
                            true
                        });
                        this.leave_archived_mission(&mission_id, Some(false), true, window, cx);
                        this.refresh_store(StoreRefreshKind::All, cx);
                        this.core(cx).events.emit(
                            "mission/changed",
                            &serde_json::json!({ "mission_id": mission_id }),
                        );
                    }
                    Err(error) => {
                        this.archiving = false;
                        this.error = Some(action_failure("archive the mission", error));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn open_mission_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mission) = self.mission.as_ref() else {
            return;
        };
        let original = mission.title.clone();
        let input = cx.new(|input_cx| {
            TextField::new(
                input_cx.focus_handle(),
                original.clone(),
                "Mission name",
                false,
            )
            .text_size(theme::text_body())
        });
        input.update(cx, |input, input_cx| input.select_all(input_cx));
        let input_focus = input.read(cx).focus_handle();
        self.rename_modal = Some(MissionRenameModal {
            mission_id: mission.id.clone(),
            original: mission.title.clone(),
            input,
            close_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            submitting: false,
            error: None,
        });
        input_focus.focus(window);
        cx.notify();
    }

    pub(super) fn close_mission_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .rename_modal
            .as_ref()
            .is_some_and(|modal| modal.submitting)
        {
            return;
        }
        self.rename_modal = None;
        self.focus_active_mission_terminal(window, cx);
        cx.notify();
    }

    pub(super) fn submit_mission_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.rename_modal.as_mut() else {
            return;
        };
        if modal.submitting || modal.input.read(cx).is_composing() {
            return;
        }
        let title = modal.input.read(cx).text().trim().to_owned();
        if title.is_empty() || title == modal.original.trim() {
            self.close_mission_rename(window, cx);
            return;
        }
        modal.submitting = true;
        modal.error = None;
        let mission_id = modal.mission_id.clone();
        cx.notify();
        let core = self.core(cx).clone();
        let rename_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_rename_impl(&core, rename_id, title)
                .await
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.mission_id.as_deref() != Some(mission_id.as_str()) {
                    return;
                }
                match result {
                    Ok(mission) => {
                        this.mission = Some(mission);
                        this.rename_modal = None;
                        this.refresh_store(StoreRefreshKind::All, cx);
                        this.core(cx).events.emit(
                            "mission/changed",
                            &serde_json::json!({ "mission_id": mission_id }),
                        );
                        this.focus_active_mission_terminal(window, cx);
                    }
                    Err(error) => {
                        if let Some(modal) = this.rename_modal.as_mut() {
                            modal.submitting = false;
                            modal.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn on_mission_rename_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "enter"
            && self
                .rename_modal
                .as_ref()
                .is_some_and(|modal| !modal.input.read(cx).is_composing())
        {
            cx.stop_propagation();
            self.submit_mission_rename(window, cx);
        }
    }
}
