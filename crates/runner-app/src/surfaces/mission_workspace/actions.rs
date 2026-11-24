use std::collections::HashSet;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, KeyDownEvent, Window};
use runner_app::ui::{MenuItem as UiMenuItem, SessionControlKind, TextField};
use runner_core::protocol::model::SessionStatus;
use runner_core::protocol::NodeType;

use super::*;
use crate::surfaces::front_page::model::WorkspaceMission;
use crate::*;

impl MissionWorkspace {
    pub(super) fn configure_mission_action_menu(&mut self, cx: &mut Context<Self>) {
        let Some(mission) = self.mission_view() else {
            return;
        };
        let busy = self.lifecycle_busy() || self.secondary || self.archived();
        let pinned = self.mission_pinned(&mission, cx);
        self.menu_pinned = pinned;
        let mut actions = Vec::new();
        let mut items = Vec::new();
        let running = mission.status == MissionStatus::Running;
        if running && !self.resuming {
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
            if self.any_session_live() && self.remote_supports(&["stopMission"]) {
                actions.push(MissionMenuAction::StopAll);
                items.push(
                    UiMenuItem::new(crate::i18n::message("ui.stop_all_slots"))
                        .icon("circle-stop.svg")
                        .disabled(busy),
                );
            }
            if self.remote_supports(&["archive"]) {
                actions.push(MissionMenuAction::Archive);
                items.push(
                    UiMenuItem::new(crate::i18n::message("ui.archive"))
                        .icon("archive.svg")
                        .destructive(true)
                        .disabled(busy),
                );
            }
        }
        self.menu_actions = actions;
        let workspace = cx.weak_entity();
        let header_renderer: runner_app::ui::menu::HeaderRenderer =
            Rc::new(move |_: &mut Window, cx: &mut App| {
                workspace
                    .update(cx, |this, cx| this.render_mission_info_popover_card(cx))
                    .unwrap_or_else(|_| div().into_any_element())
            });
        let title = mission.title;
        self.action_menu.update(cx, |menu, cx| {
            menu.set_header_renderer(Some(header_renderer), cx);
            menu.set_trigger_label(
                runner_app::ui::MenuTriggerLabel {
                    text: title.into(),
                    color: theme::text(),
                },
                cx,
            );
            menu.set_items(items, cx);
        });
    }

    pub(crate) fn toggle_mission_info_popover(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mission_view().is_none() {
            cx.propagate();
            return;
        }
        self.configure_mission_action_menu(cx);
        self.action_menu
            .update(cx, |menu, menu_cx| menu.toggle_open(window, menu_cx));
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
            MissionMenuAction::StopAll => self.open_stop_all_confirm(window, cx),
            MissionMenuAction::Archive => self.archive_open_mission(window, cx),
        }
    }

    /// A linked mission's pin is its group's: the pin of the tab holding its
    /// chat (#748). The daemon pins that tab when the mission is pinned.
    pub(super) fn mission_pinned(&self, mission: &WorkspaceMission, cx: &App) -> bool {
        let Some(chat) = mission.linked_chat.as_ref().filter(|chat| !chat.archived) else {
            return mission.pinned;
        };
        if self.presentation.is_remote() {
            return self
                .presentation
                .chat_tab_pinned(&chat.session_id)
                .unwrap_or(mission.pinned);
        }
        self.app_store
            .read(cx)
            .nodes
            .iter()
            .find(|node| {
                node.node_type == NodeType::Tab
                    && PaneLayout::from_node_row(node)
                        .is_ok_and(|layout| layout.contains_session(&chat.session_id))
            })
            .map_or(mission.pinned, |tab| tab.pinned_position.is_some())
    }

    #[cfg(test)]
    pub(crate) fn toggle_pin_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.configure_mission_action_menu(cx);
        let index = self
            .menu_actions
            .iter()
            .position(|action| matches!(action, MissionMenuAction::Pin))
            .expect("the header offers Pin");
        self.handle_mission_menu_action(index, window, cx);
    }

    #[cfg(test)]
    pub(crate) fn menu_pinned_for_test(&self) -> bool {
        self.menu_pinned
    }

    fn toggle_mission_pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mission) = self.mission_view() else {
            return;
        };
        let pinned = self.mission_pinned(&mission, cx);
        let mission_id = mission.id.clone();
        let Some(operation) = self.prepare_mission_action("pin", "pin".into()) else {
            return;
        };
        let client = self.client.clone();
        let task = cx.background_spawn(async move {
            client
                .pin_mission_state(mission.id, !pinned)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, _, cx| {
                let Some(result) =
                    this.complete_mission_action(operation, result, |_| Vec::new(), cx)
                else {
                    return;
                };
                if this.mission_id.as_deref() != Some(mission_id.as_str()) {
                    return;
                }
                match result {
                    Ok(mission) => {
                        this.mission = mission;
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
        let confirm = cx.entity();
        let cancel = confirm.clone();
        let restart_id = session_id.clone();
        runner_app::ui::ConfirmDialog::new(
            crate::i18n::tf("format.restart", &[("handle", (handle).to_string())]),
            restart_confirm_body(&handle),
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

    pub(crate) fn add_role_context(
        &self,
        window: &Window,
        cx: &App,
    ) -> Option<crate::surfaces::start_chat::AddRoleContext> {
        let mission = self.mission_view()?;
        if !self.remote_supports(&["catalog", "addMissionRole"])
            || !mission_slot_actions_available(
                Some(mission.status),
                self.archived(),
                self.secondary_state(cx).secondary,
            )
        {
            return None;
        }
        let mut current: HashSet<String> = self
            .members()
            .into_iter()
            .map(|member| member.handle)
            .collect();
        if !self.presentation.is_remote() {
            current.extend(self.roster.iter().map(|slot| slot.slot.slot_handle.clone()));
        }
        let mut removed: HashSet<String> = self
            .events
            .iter()
            .filter(|event| {
                event.from == "human"
                    && event.kind == EventKind::Signal
                    && event
                        .signal_type
                        .as_ref()
                        .is_some_and(|kind| kind.as_str() == "role_removed")
            })
            .filter_map(|event| event.payload["handle"].as_str().map(str::to_owned))
            .collect();
        if let Some(summary) = self.remote_summary() {
            removed.extend(summary.removed_handles.iter().cloned());
        }
        let viewport_size = self
            .client
            .is_remote()
            .then(|| self.current_mission_terminal_size(window, cx));
        Some(crate::surfaces::start_chat::AddRoleContext {
            mission_id: mission.id,
            title: mission.title,
            cwd: mission.cwd.unwrap_or_default(),
            current,
            removed,
            client: self.client.clone(),
            viewport_size,
        })
    }

    pub(super) fn request_remove_role(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.remote_supports(&["removeMissionRole"])
            || self.stopping
            || self.resuming
            || self.archiving
            || !mission_slot_actions_available(
                self.mission_view().map(|mission| mission.status),
                self.archived(),
                self.secondary_state(cx).secondary,
            )
            || self.slot_actions.contains(session_id)
            || (!self.client.is_remote() && self.transitions.contains_key(session_id))
        {
            return;
        }
        if self
            .members()
            .iter()
            .any(|session| session.session.id == session_id)
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

    pub(super) fn remove_role(
        &mut self,
        session_id: &str,
        handle: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote = self.client.is_remote();
        if !self.remote_supports(&["removeMissionRole"])
            || (remote
                && !self
                    .members()
                    .iter()
                    .any(|member| member.session.id == session_id))
        {
            return;
        }
        if self.stopping
            || self.resuming
            || self.archiving
            || self.slot_actions.contains(session_id)
            || (!remote && self.transitions.contains_key(session_id))
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
        let Some(operation) =
            self.prepare_mission_action("removeMissionRole", session_id.to_owned())
        else {
            return;
        };
        let session_id = session_id.to_owned();
        self.slot_actions.insert(session_id.clone());
        let generation = self.generation;
        let client = self.client.clone();
        let remove_id = mission_id.clone();
        let handle = handle.to_owned();
        let remove = cx.background_spawn(async move {
            client
                .mission_remove_role(&remove_id, &handle)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = remove.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let Some(result) =
                    this.complete_mission_action(operation, result, |_| Vec::new(), cx)
                else {
                    return;
                };
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

    pub(crate) fn act_on_slot(
        &mut self,
        session_id: &str,
        action: SessionControlKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.act_on_slot_with_restore(session_id, action, false, window, cx);
    }

    pub(super) fn act_on_slot_with_restore(
        &mut self,
        session_id: &str,
        action: SessionControlKind,
        restore_in_main: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote = self.client.is_remote();
        let feature = match action {
            SessionControlKind::Stop => "stop",
            SessionControlKind::Restart => "restart",
            _ => "resume",
        };
        if !self.remote_supports(&[feature])
            || (remote
                && !self
                    .members()
                    .iter()
                    .any(|member| member.session.id == session_id))
        {
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
            || (!remote && self.transitions.contains_key(session_id))
        {
            return;
        }
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        if action == SessionControlKind::Resume {
            self.restore_fallback = None;
        }
        let slot_cwd = self
            .members()
            .iter()
            .find(|member| member.session.id == session_id)
            .and_then(|member| member.session.cwd.clone())
            .or_else(|| self.mission_view().and_then(|mission| mission.cwd.clone()));
        let mission_project_id = self
            .mission_view()
            .and_then(|mission| mission.project_id.clone());
        self.slot_actions.insert(session_id.to_owned());
        let generation = self.generation;
        if !remote && action != SessionControlKind::Stop {
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
        let Some(operation) = self.prepare_mission_action(feature, session_id.to_owned()) else {
            return;
        };
        let client = self.client.clone();
        let target = session_id.to_owned();
        let size = self.current_mission_terminal_size(window, cx);
        cx.notify();
        let task_target = target.clone();
        let task = cx.background_spawn(async move {
            let result = match action {
                SessionControlKind::Stop => client.stop(&task_target).map(|()| Vec::new()),
                SessionControlKind::Resume => client
                    .resume_session_with_options(
                        &task_target,
                        size,
                        false,
                        restore_in_main && !remote,
                    )
                    .map(|id| vec![id]),
                SessionControlKind::Restart => {
                    client.restart(&task_target, size).map(|id| vec![id])
                }
                _ => unreachable!(),
            };
            let gate = result
                .as_ref()
                .err()
                .and_then(|error| error.version_gate.clone());
            let fallback = if !remote && !restore_in_main && action == SessionControlKind::Resume {
                result.as_ref().err().and_then(|error| {
                    crate::surfaces::worktree_restore::classify_restore_failure(
                        client.local_daemon(),
                        slot_cwd.as_deref(),
                        mission_project_id.as_deref(),
                        &error.message,
                    )
                })
            } else {
                None
            };
            (result.map_err(|error| error.to_string()), gate, fallback)
        });
        cx.spawn_in(window, async move |weak, cx| {
            let (result, gate, fallback) = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let Some(result) = this.complete_mission_action_observed(
                    operation,
                    result,
                    |ids| ids,
                    |workspace, succeeded, cx| {
                        let target = target.clone();
                        let host = workspace.client.target.clone();
                        let _ = workspace.shell.update(cx, |root, cx| {
                            root.record_session_gate(
                                &host,
                                &target,
                                gate,
                                if action == SessionControlKind::Restart {
                                    crate::surfaces::agent_recovery::RecoveryAction::Restart
                                } else {
                                    crate::surfaces::agent_recovery::RecoveryAction::Resume
                                },
                                succeeded,
                            );
                            cx.notify();
                        });
                    },
                    cx,
                ) else {
                    return;
                };
                if !this.is_current(&mission_id, generation) {
                    return;
                }
                this.slot_actions.remove(&target);
                if let Err(error) = result {
                    this.transitions.remove(&target);
                    if let Some(prompt) = fallback {
                        this.restore_fallback = Some(MissionRestoreFallback {
                            slot_session_id: Some(target.clone()),
                            prompt,
                        });
                        this.error = None;
                    } else if !is_concurrent_resume_error(&error) {
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
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        if self.lifecycle_busy() || self.secondary_state(cx).secondary {
            return;
        }
        let remote = self.client.is_remote();
        if remote
            && !self
                .mission_view()
                .is_some_and(|mission| mission.status == MissionStatus::Running)
        {
            return;
        }
        let Some(operation) = self.prepare_mission_action("stopMission", "stopMission".into())
        else {
            return;
        };
        self.stopping = true;
        if !remote {
            self.root_focus.focus(window, cx);
        }
        cx.notify();
        let client = self.client.clone();
        let stop_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            client
                .stop_mission_state(stop_id)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let Some(result) =
                    this.complete_mission_action(operation, result, |_| Vec::new(), cx)
                else {
                    return;
                };
                if this.mission_id.as_deref() != Some(mission_id.as_str()) {
                    return;
                }
                this.stopping = false;
                match result {
                    Ok(output) => {
                        this.mission = output.mission;
                        this.sessions = output.sessions;
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

    pub(crate) fn resume_open_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.resume_open_mission_inner(false, window, cx);
    }

    pub(crate) fn confirm_mission_restore_in_main(
        &mut self,
        slot_session_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.restore_fallback = None;
        match slot_session_id {
            Some(session_id) => {
                self.act_on_slot_with_restore(
                    &session_id,
                    SessionControlKind::Resume,
                    true,
                    window,
                    cx,
                );
            }
            None => self.resume_open_mission_inner(true, window, cx),
        }
    }

    fn resume_open_mission_inner(
        &mut self,
        restore_in_main: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        if self.lifecycle_busy() || self.secondary_state(cx).secondary {
            return;
        }
        let remote = self.client.is_remote();
        if remote
            && !self
                .mission_view()
                .is_some_and(|mission| mission.status == MissionStatus::Running)
        {
            return;
        }
        let stopped = self
            .members()
            .iter()
            .filter(|session| session.session.status != SessionStatus::Running)
            .map(|session| session.session.id.clone())
            .collect::<Vec<_>>();
        if !remote && stopped.is_empty() {
            return;
        }
        self.restore_fallback = None;
        let mission_project_id = self
            .mission_view()
            .and_then(|mission| mission.project_id.clone());
        let fallback_cwd = self
            .mission_view()
            .and_then(|mission| mission.cwd.clone())
            .or_else(|| {
                self.members()
                    .iter()
                    .find(|session| stopped.contains(&session.session.id))
                    .and_then(|session| session.session.cwd.clone())
            });
        let size = self.current_mission_terminal_size(window, cx);
        let Some(operation) = self.prepare_mission_action("resumeMission", "resumeMission".into())
        else {
            return;
        };
        self.resuming = true;
        for session_id in stopped.iter().filter(|_| !remote) {
            self.begin_mission_transition(
                session_id,
                MissionTransitionKind::Resuming,
                None,
                window,
                cx,
            );
        }
        if !remote {
            self.root_focus.focus(window, cx);
        }
        cx.notify();
        let client = self.client.clone();
        let resume_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            let result = client.resume_mission_state_with_options(
                resume_id,
                size,
                restore_in_main && !remote,
            );
            let gate = result
                .as_ref()
                .err()
                .and_then(|error| error.version_gate.clone());
            let failure_text = match &result {
                Ok(output) => output.error.as_deref(),
                Err(error) => Some(error.message.as_str()),
            };
            let fallback = if !remote && !restore_in_main {
                failure_text.and_then(|err| {
                    crate::surfaces::worktree_restore::classify_restore_failure(
                        client.local_daemon(),
                        fallback_cwd.as_deref(),
                        mission_project_id.as_deref(),
                        err,
                    )
                })
            } else {
                None
            };
            (result.map_err(|error| error.to_string()), gate, fallback)
        });
        cx.spawn_in(window, async move |weak, cx| {
            let (result, gate, fallback) = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let Some(result) = this.complete_mission_action_observed(
                    operation,
                    result,
                    |output| output.sessions,
                    |workspace, succeeded, cx| {
                        let host = workspace.client.target.clone();
                        let sessions = workspace
                            .members()
                            .iter()
                            .filter(|session| {
                                stopped.contains(&session.session.id)
                                    && gate
                                        .as_ref()
                                        .is_none_or(|gate| session.runtime == gate.runtime.key())
                            })
                            .map(|session| session.session.id.clone())
                            .collect::<Vec<_>>();
                        let _ = workspace.shell.update(cx, |root, cx| {
                            for session in sessions {
                                root.record_session_gate(
                                    &host,
                                    &session,
                                    gate.clone(),
                                    crate::surfaces::agent_recovery::RecoveryAction::Resume,
                                    succeeded,
                                );
                            }
                            cx.notify();
                        });
                    },
                    cx,
                ) else {
                    return;
                };
                if this.mission_id.as_deref() != Some(mission_id.as_str()) {
                    return;
                }
                this.resuming = false;
                match result {
                    Ok(output) => {
                        this.sessions = output.local_sessions;
                        for session in &this.sessions {
                            if !this.open_tabs.contains(&session.session.id) {
                                this.open_tabs.push(session.session.id.clone());
                            }
                        }
                        if let Some(error) = output.error {
                            if let Some(prompt) = fallback {
                                this.restore_fallback = Some(MissionRestoreFallback {
                                    slot_session_id: None,
                                    prompt,
                                });
                                this.error = None;
                            } else {
                                this.error = Some(action_failure(
                                    crate::i18n::message("action.resume_mission"),
                                    error,
                                ));
                            }
                        } else {
                            this.error = None;
                            this.restore_fallback = None;
                        }
                        this.sync_mission_copy_entities(cx);
                        if let Err(error) = this.ensure_mission_terminals_attached(window, cx) {
                            this.error = Some(error.to_string().into());
                        }
                        if restore_in_main {
                            this.refresh_open_mission(window, cx);
                        }
                        this.refresh_store(StoreRefreshKind::All, cx);
                    }
                    Err(error) => {
                        if let Some(prompt) = fallback {
                            this.restore_fallback = Some(MissionRestoreFallback {
                                slot_session_id: None,
                                prompt,
                            });
                            this.error = None;
                        } else {
                            this.error = Some(action_failure(
                                crate::i18n::message("action.resume_mission"),
                                error,
                            ));
                        }
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
        if !self.client.is_remote()
            && let Some(shell) = self.shell.upgrade()
            && !shell
                .read(cx)
                .emptied_managed_worktrees(&[], std::slice::from_ref(&mission_id), cx)
                .is_empty()
        {
            shell.update(cx, |root, cx| {
                root.request_archive_mission(mission_id, window, cx);
            });
            return;
        }
        let Some(operation) = self.prepare_mission_action("archive", "archive".into()) else {
            return;
        };
        self.archiving = true;
        if !self.client.is_remote() {
            self.set_sidebar_archiving(&mission_id, true, cx);
            self.root_focus.focus(window, cx);
        }
        cx.notify();
        let client = self.client.clone();
        let archive_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            client
                .archive_mission_state(archive_id)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let Some(result) =
                    this.complete_mission_action(operation, result, |_| Vec::new(), cx)
                else {
                    return;
                };
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
        let mission_id = modal.mission_id.clone();
        cx.notify();
        let Some(operation) = self.prepare_mission_action("rename", "rename".into()) else {
            return;
        };
        let client = self.client.clone();
        let rename_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            client
                .rename_mission_state(rename_id, title)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let Some(result) =
                    this.complete_mission_action(operation, result, |_| Vec::new(), cx)
                else {
                    return;
                };
                if this.mission_id.as_deref() != Some(mission_id.as_str()) {
                    return;
                }
                match result {
                    Ok(mission) => {
                        this.mission = mission;
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
