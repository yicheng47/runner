use std::collections::HashSet;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, KeyDownEvent, Window};
use runner_app::ui::{MenuItem as UiMenuItem, SessionControlKind, TextField};
use runner_core::protocol::model::SessionStatus;

use super::*;
use crate::*;

impl MissionWorkspace {
    pub(super) fn configure_mission_action_menu(&mut self, cx: &mut Context<Self>) {
        let Some(mission) = self.mission_view() else {
            return;
        };
        let busy = self.lifecycle_busy() || self.secondary;
        let pinned = mission.pinned;
        if self.remote.is_some() {
            let pinned = self.mission_view().is_some_and(|mission| mission.pinned);
            let mut actions = Vec::new();
            let mut items = Vec::new();
            for (feature, action, label, icon, destructive) in [
                (
                    "pin",
                    MissionMenuAction::Pin,
                    if pinned { "ui.unpin" } else { "ui.pin" },
                    if pinned { "pin-off.svg" } else { "pin.svg" },
                    false,
                ),
                (
                    "rename",
                    MissionMenuAction::Rename,
                    "ui.rename",
                    "square-pen.svg",
                    false,
                ),
                (
                    "archive",
                    MissionMenuAction::Archive,
                    "ui.archive",
                    "archive.svg",
                    true,
                ),
            ] {
                if self.remote_supports(&[feature]) {
                    actions.push(action);
                    items.push(
                        UiMenuItem::new(crate::i18n::message(label))
                            .icon(icon)
                            .destructive(destructive)
                            .disabled(busy),
                    );
                }
            }
            self.menu_actions = actions;
            self.action_menu
                .update(cx, |menu, cx| menu.set_items(items, cx));
            return;
        }
        self.menu_actions = vec![
            MissionMenuAction::Pin,
            MissionMenuAction::Rename,
            MissionMenuAction::Archive,
        ];
        self.action_menu.update(cx, |menu, menu_cx| {
            menu.set_items(
                vec![
                    UiMenuItem::new(if pinned {
                        crate::i18n::message("ui.unpin")
                    } else {
                        crate::i18n::message("ui.pin")
                    })
                    .icon(if pinned { "pin-off.svg" } else { "pin.svg" })
                    .disabled(busy),
                    UiMenuItem::new(crate::i18n::message("ui.rename"))
                        .icon("square-pen.svg")
                        .disabled(busy),
                    UiMenuItem::new(crate::i18n::message("ui.archive"))
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
        if self.remote.is_some() {
            if let Some(mission) = self.mission_view() {
                self.remote_action(
                    "pin",
                    "pin".into(),
                    move |client| {
                        client
                            .mission_pin(mission.id, !mission.pinned)
                            .map(|_| Vec::new())
                    },
                    cx,
                );
            }
            return;
        }
        let Some(mission) = self.mission.clone() else {
            return;
        };
        let mission_id = mission.id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            core.mission_pin_impl(mission.id, mission.pinned_at.is_none())
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
                    }
                    Err(error) => {
                        this.error = Some(action_failure(
                            crate::i18n::message("action.update_pin"),
                            error,
                        ));
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
        if !self.remote_supports(&[match action {
            SessionControlKind::Stop => "stop",
            SessionControlKind::Restart => "restart",
            _ => "resume",
        }]) {
            return;
        }
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
                self.mission_view().map(|mission| mission.status),
                self.archived(),
                self.secondary_state(cx).secondary,
            )
            || self.slot_actions.contains(session_id)
            || self.transitions.contains_key(session_id)
        {
            return;
        }
        self.restart_confirm = Some(session_id.to_owned());
        self.root_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn render_restart_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let session_id = self.restart_confirm.clone().unwrap_or_default();
        let members = self.members();
        let target = members
            .iter()
            .find(|session| session.session.id == session_id);
        let handle = target
            .map(|session| session.handle.clone())
            .unwrap_or_default();
        let lead_handle = members
            .iter()
            .find(|session| session.lead)
            .map(|session| session.handle.as_str());
        let confirm = cx.entity();
        let cancel = confirm.clone();
        let restart_id = session_id.clone();
        runner_app::ui::ConfirmDialog::new(
            crate::i18n::tf("format.restart", &[("handle", (handle).to_string())]),
            restart_confirm_body(
                &handle,
                lead_handle,
                target.is_some_and(|session| session.lead),
            ),
            crate::i18n::text("ui.restart"),
            crate::i18n::text("ui.restarting"),
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

    pub(super) fn request_remove_role(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote.is_some() {
            return;
        }
        if self
            .sessions
            .iter()
            .any(|session| session.session.id == session_id && !session.lead)
            && !self.slot_actions.contains(session_id)
        {
            self.remove_confirm = Some(session_id.to_owned());
            self.root_focus.focus(window, cx);
            cx.notify();
        }
    }

    pub(super) fn render_remove_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let session_id = self.remove_confirm.clone().unwrap_or_default();
        let handle = self
            .members()
            .iter()
            .find(|session| session.session.id == session_id)
            .map(|session| session.handle.clone())
            .unwrap_or_default();
        let confirm = cx.entity();
        let cancel = confirm.clone();
        runner_app::ui::ConfirmDialog::new(
            crate::i18n::tf("format.remove_from_the_mission", &[("handle", (handle).to_string())]),
            crate::i18n::tf("format.its_session_stops_and_it_leaves_the_team_its_messages_stay_in_the_feed_message", &[("handle", (handle).to_string())]),
            crate::i18n::text("ui.remove"), crate::i18n::text("ui.removing"), false,
            Rc::new(move |window, cx| confirm.update(cx, |this, cx| {
                this.remove_confirm = None;
                this.remove_role(&session_id, &handle, window, cx);
            })),
            Rc::new(move |window, cx| cancel.update(cx, |this, cx| {
                this.remove_confirm = None;
                this.focus_active_mission_terminal(window, cx);
                cx.notify();
            })),
        ).icon("log-out.svg").into_any_element()
    }

    fn remove_role(
        &mut self,
        session_id: &str,
        handle: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote.is_some() {
            return;
        }
        if self.stopping
            || self.resuming
            || self.archiving
            || self.slot_actions.contains(session_id)
            || self.transitions.contains_key(session_id)
            || !mission_slot_actions_available(
                self.mission_view().map(|mission| mission.status),
                self.archived(),
                self.secondary_state(cx).secondary,
            )
        {
            return;
        }
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        let session_id = session_id.to_owned();
        self.slot_actions.insert(session_id.clone());
        let generation = self.generation;
        let core = self.core(cx).clone();
        let remove_id = mission_id.clone();
        let handle = handle.to_owned();
        let remove = cx.background_spawn(async move {
            core.mission_remove_role(&remove_id, &handle)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = remove.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if !this.is_current(&mission_id, generation) {
                    return;
                }
                this.slot_actions.remove(&session_id);
                if let Err(error) = result {
                    this.warning = Some(action_failure(
                        crate::i18n::message("action.remove_role"),
                        error,
                    ));
                }
                this.refresh_open_mission(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn act_on_slot(
        &mut self,
        session_id: &str,
        action: SessionControlKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote.is_some() {
            let size = self.current_mission_terminal_size(window, cx);
            self.remote_slot_action(session_id, action, size, cx);
            return;
        }
        if self.stopping
            || self.resuming
            || self.archiving
            || !mission_slot_actions_available(
                self.mission_view().map(|mission| mission.status),
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
                SessionControlKind::Stop => core.session_kill(&task_target),
                SessionControlKind::Resume => core
                    .session_resume(&task_target, Some(size.0), Some(size.1))
                    .map(|_| ()),
                SessionControlKind::Restart => core
                    .session_restart(&task_target, Some(size.0), Some(size.1))
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
                                SessionControlKind::Stop => {
                                    crate::i18n::message("action.stop_slot")
                                }
                                SessionControlKind::Restart => {
                                    crate::i18n::message("action.restart_slot")
                                }
                                _ => crate::i18n::message("action.resume_slot"),
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
        if !self.remote_supports(&["stopMission"]) {
            return;
        }
        if self.lifecycle_busy() || self.secondary_state(cx).secondary {
            return;
        }
        self.stop_all_confirm = true;
        self.root_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn render_stop_all_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let confirm = cx.entity();
        let cancel = confirm.clone();
        runner_app::ui::ConfirmDialog::new(
            stop_all_title(
                self.members()
                    .iter()
                    .filter(|session| session.session.status == SessionStatus::Running)
                    .count(),
            ),
            crate::i18n::text(STOP_ALL_BODY),
            crate::i18n::text("ui.stop_all"),
            crate::i18n::text("ui.stopping"),
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

    pub(super) fn stop_open_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote.is_some() {
            if !self.lifecycle_busy()
                && self.remote_supports(&["stopMission"])
                && self
                    .mission_view()
                    .is_some_and(|mission| mission.status == MissionStatus::Running)
            {
                if let Some(id) = self.mission_id.clone() {
                    self.stopping = true;
                    self.remote_action(
                        "stopMission",
                        "stopMission".into(),
                        move |client| client.stop_mission(id).map(|_| Vec::new()),
                        cx,
                    );
                }
            }
            return;
        }
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        if self.lifecycle_busy() || self.secondary_state(cx).secondary {
            return;
        }
        self.stopping = true;
        self.root_focus.focus(window, cx);
        cx.notify();
        let core = self.core(cx).clone();
        let stop_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            let mission = core
                .mission_stop_impl(stop_id.clone())
                .map_err(|error| error.to_string())?;
            let sessions = core
                .session_list(&stop_id)
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
                        this.error = Some(action_failure(
                            crate::i18n::message("action.stop_mission"),
                            error,
                        ));
                    }
                }
                this.focus_active_mission_terminal(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn resume_open_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote.is_some() {
            if !self.lifecycle_busy()
                && self.remote_supports(&["resumeMission"])
                && self
                    .mission_view()
                    .is_some_and(|mission| mission.status == MissionStatus::Running)
            {
                if let Some(id) = self.mission_id.clone() {
                    let size = self.current_mission_terminal_size(window, cx);
                    self.resuming = true;
                    self.remote_action(
                        "resumeMission",
                        "resumeMission".into(),
                        move |client| client.resume_mission(id, size),
                        cx,
                    );
                }
            }
            return;
        }
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
        self.root_focus.focus(window, cx);
        cx.notify();
        let core = self.core(cx).clone();
        let resume_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            let mut sessions = core
                .session_list(&resume_id)
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
                if let Err(error) =
                    core.session_resume(&candidate.session.id, Some(size.0), Some(size.1))
                {
                    let message = error.to_string();
                    if is_concurrent_resume_error(&message) {
                        match core.session_list(&resume_id) {
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
            match core.session_list(&resume_id) {
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
                            this.error = Some(action_failure(
                                crate::i18n::message("action.resume_mission"),
                                error,
                            ));
                        } else {
                            this.error = None;
                        }
                        this.sync_mission_copy_entities(cx);
                        if let Err(error) = this.ensure_mission_terminals_attached(window, cx) {
                            this.error = Some(error.to_string().into());
                        }
                        this.refresh_store(StoreRefreshKind::All, cx);
                    }
                    Err(error) => {
                        this.error = Some(action_failure(
                            crate::i18n::message("action.resume_mission"),
                            error,
                        ));
                    }
                }
                this.focus_active_mission_terminal(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn archive_open_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote.is_some() {
            if !self.lifecycle_busy() && self.remote_supports(&["archive"]) {
                if let Some(id) = self.mission_id.clone() {
                    self.archiving = true;
                    self.remote_action(
                        "archive",
                        "archive".into(),
                        move |client| {
                            client
                                .archive(runner_core::companion::ActionTarget {
                                    kind: runner_core::companion::ActionKind::Mission,
                                    id,
                                })
                                .map(|_| Vec::new())
                        },
                        cx,
                    );
                }
            }
            return;
        }
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        if self.lifecycle_busy() || self.secondary_state(cx).secondary {
            return;
        }
        self.archiving = true;
        self.set_sidebar_archiving(&mission_id, true, cx);
        self.root_focus.focus(window, cx);
        cx.notify();
        let core = self.core(cx).clone();
        let archive_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            core.mission_archive_impl(archive_id)
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
                    }
                    Err(error) => {
                        this.archiving = false;
                        this.error = Some(action_failure(
                            crate::i18n::message("action.archive_mission"),
                            error,
                        ));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn open_mission_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.remote_supports(&["rename"]) {
            return;
        }
        let Some(mission) = self.mission_view() else {
            return;
        };
        let original = mission.title.clone();
        let input = cx.new(|input_cx| {
            TextField::new(
                input_cx.focus_handle(),
                original.clone(),
                crate::i18n::message("ui.mission_name"),
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
        input_focus.focus(window, cx);
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
        if self.remote.is_some() {
            if !self.remote_supports(&["rename"]) {
                return;
            }
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
            let id = modal.mission_id.clone();
            self.remote_action(
                "rename",
                "rename".into(),
                move |client| {
                    client
                        .rename(
                            runner_core::companion::ActionTarget {
                                kind: runner_core::companion::ActionKind::Mission,
                                id,
                            },
                            title,
                        )
                        .map(|_| Vec::new())
                },
                cx,
            );
            return;
        }
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
            core.mission_rename_impl(rename_id, title)
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
