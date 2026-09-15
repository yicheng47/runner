use std::collections::HashSet;
use std::time::Instant;

use gpui::prelude::*;
use gpui::Window;
use runner_backend::model::Event;

use super::*;
use crate::*;

impl MissionWorkspace {
    pub(crate) fn handle_mission_workspace_event(
        &mut self,
        event: runner_backend::events::AppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_active(cx) {
            return;
        }
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        if (event.name == "session/exit"
            || (event.name == "session/status"
                && event.payload.get("status").is_some_and(|status| {
                    status
                        .get("unread_since")
                        .is_some_and(|value| !value.is_null())
                        || status
                            .get("error_since")
                            .is_some_and(|value| !value.is_null())
                })))
            && event
                .payload
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| self.active_tab == MissionTab::Session(id.to_owned()))
        {
            self.mark_active_session_viewed(window, cx);
        }
        match event.name {
            "event/appended" => {
                if event
                    .payload
                    .get("mission_id")
                    .and_then(serde_json::Value::as_str)
                    != Some(mission_id.as_str())
                {
                    return;
                }
                let Some(value) = event.payload.get("event") else {
                    return;
                };
                let Ok(appended) = serde_json::from_value::<Event>(value.clone()) else {
                    return;
                };
                if !self
                    .events
                    .iter()
                    .any(|existing| existing.id == appended.id)
                {
                    self.handle_feed_append(&appended);
                    self.events.push(appended);
                    self.events.sort_by(|a, b| a.id.cmp(&b.id));
                    self.rebuild_event_projection();
                    cx.notify();
                }
            }
            "router/delivery-blocked" => {
                if event
                    .payload
                    .get("mission_id")
                    .and_then(serde_json::Value::as_str)
                    != Some(mission_id.as_str())
                {
                    return;
                }
                let Some(session_id) = event
                    .payload
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                else {
                    return;
                };
                let blocked = event
                    .payload
                    .get("blocked")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let unread_count = event
                    .payload
                    .get("unread_count")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                if blocked && unread_count > 0 {
                    self.delivery_blocked
                        .insert(session_id, DeliveryBlocked { unread_count });
                } else {
                    self.delivery_blocked.remove(&session_id);
                }
                cx.notify();
            }
            "session/warning" => {
                if event
                    .payload
                    .get("mission_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(mission_id.as_str())
                {
                    self.warning = event
                        .payload
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                    cx.notify();
                }
            }
            "session/input-error" => {
                let relevant = event
                    .payload
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|session_id| {
                        self.sessions
                            .iter()
                            .any(|session| session.session.id == session_id)
                            || self
                                .layout
                                .drawer
                                .shells()
                                .iter()
                                .any(|shell_id| shell_id == session_id)
                    });
                if relevant {
                    self.error = event
                        .payload
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .map(|message| action_failure("write to the terminal", message));
                    cx.notify();
                }
            }
            "session/archived" => {
                if let Some(session_id) = event
                    .payload
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                {
                    let relevant = self
                        .sessions
                        .iter()
                        .any(|session| session.session.id == session_id);
                    self.attached.remove(session_id);
                    self.delivery_blocked.remove(session_id);
                    self.transitions.remove(session_id);
                    if relevant {
                        self.refresh_open_mission(window, cx);
                    }
                }
            }
            "session/exit" | "session/spawned" | "session/updated" => {
                let session_id = event
                    .payload
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                let mission_event = event
                    .payload
                    .get("mission_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(mission_id.as_str());
                let drawer_event = session_id
                    .as_ref()
                    .is_some_and(|session_id| self.layout.drawer.shells().contains(session_id));
                if mission_event || drawer_event {
                    if event.name == "session/exit" {
                        if let Some(session_id) = session_id.as_deref() {
                            self.delivery_blocked.remove(session_id);
                            if self.transition_kind(session_id)
                                != Some(MissionTransitionKind::Restarting)
                            {
                                self.transitions.remove(session_id);
                            }
                            if mission_event {
                                self.slot_exit_codes.insert(
                                    session_id.to_owned(),
                                    event
                                        .payload
                                        .get("exit_code")
                                        .and_then(serde_json::Value::as_i64)
                                        .and_then(|code| i32::try_from(code).ok()),
                                );
                            }
                            self.closing_drawer_shells.remove(session_id);
                            if drawer_event {
                                let exit_code = event
                                    .payload
                                    .get("exit_code")
                                    .and_then(serde_json::Value::as_i64)
                                    .and_then(|code| i32::try_from(code).ok());
                                self.drawer_exit_codes
                                    .insert(session_id.to_owned(), exit_code);
                                if drawer_session_should_take_focus(&self.layout, session_id) {
                                    self.drawer_focus.focus(window);
                                }
                            }
                        }
                    } else if event.name == "session/spawned" {
                        if let Some(session_id) = session_id.as_deref() {
                            self.attached.remove(session_id);
                            self.delivery_blocked.remove(session_id);
                            self.drawer_exit_codes.remove(session_id);
                            self.slot_exit_codes.remove(session_id);
                            let existing = self.transitions.get_mut(session_id).map(|transition| {
                                transition.baseline_seq = 0;
                                transition.started_at = Instant::now();
                                transition.kind
                            });
                            if let Some(kind) = transition_to_begin_on_spawn(existing) {
                                self.begin_mission_transition(
                                    session_id,
                                    kind,
                                    Some(0),
                                    window,
                                    cx,
                                );
                            }
                        }
                    }
                    if mission_event {
                        self.refresh_open_mission(window, cx);
                    } else {
                        self.refresh_store(StoreRefreshKind::All, cx);
                        if event.name != "session/exit" {
                            if let Some(session_id) = session_id.as_deref() {
                                if let Some(status) = self
                                    .drawer_session_entry(session_id, cx)
                                    .map(|entry| entry.status)
                                {
                                    if let Err(error) = self.ensure_mission_terminal_attached(
                                        session_id, status, window, cx,
                                    ) {
                                        self.error = Some(error.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "mission/changed" => {
                let relevant = event
                    .payload
                    .get("mission_id")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|changed| changed == mission_id);
                if relevant {
                    self.refresh_open_mission(window, cx);
                }
            }
            "mission/resync" => self.resync_mission_events(cx),
            _ => {}
        }
    }

    fn resync_mission_events(&mut self, cx: &mut Context<Self>) {
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        let generation = self.generation;
        self.event_resync_generation += 1;
        let resync_generation = self.event_resync_generation;
        let core = self.core(cx).clone();
        let resync_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_events_replay(&core, &resync_id)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                if !this.is_current(&mission_id, generation)
                    || this.event_resync_generation != resync_generation
                {
                    return;
                }
                match result {
                    Ok(replayed) => {
                        let old_tail = this.events.last().map(|event| event.id.clone());
                        let mut seen = this
                            .events
                            .iter()
                            .map(|event| event.id.clone())
                            .collect::<HashSet<_>>();
                        for event in replayed {
                            if seen.insert(event.id.clone()) {
                                this.events.push(event);
                            }
                        }
                        this.events.sort_by(|a, b| a.id.cmp(&b.id));
                        let new_tail = this.events.last().cloned();
                        if new_tail.as_ref().map(|event| &event.id) != old_tail.as_ref() {
                            if let Some(tail) = new_tail.as_ref() {
                                this.handle_feed_append(tail);
                            }
                        }
                        this.rebuild_event_projection();
                    }
                    Err(error) => {
                        this.warning = Some(action_failure("resync the mission feed", error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn refresh_open_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        let generation = self.generation;
        self.refresh_generation += 1;
        let refresh_generation = self.refresh_generation;
        let core = self.core(cx).clone();
        let refresh_id = mission_id.clone();
        let refresh = cx.background_spawn(async move {
            let mission = runner_backend::ops::mission::mission_get(&core, &refresh_id)
                .map_err(|error| error.to_string())?;
            let sessions = runner_backend::ops::session::session_list(&core, &refresh_id)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((mission, sessions))
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = refresh.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if !this.is_current(&mission_id, generation)
                    || this.refresh_generation != refresh_generation
                    || !this.is_active(cx)
                {
                    return;
                }
                match result {
                    Ok((mission, sessions)) => {
                        let was_archived = this
                            .mission
                            .as_ref()
                            .map(|mission| mission.archived_at.is_some());
                        let archived = mission.archived_at.is_some();
                        let valid_ids = sessions
                            .iter()
                            .map(|session| session.session.id.clone())
                            .collect::<HashSet<_>>();
                        this.mission = Some(mission);
                        this.sessions = sessions;
                        this.delivery_blocked
                            .retain(|session_id, _| valid_ids.contains(session_id));
                        let drawer_ids = this
                            .layout
                            .drawer
                            .shells()
                            .iter()
                            .cloned()
                            .collect::<HashSet<_>>();
                        this.transitions.retain(|session_id, _| {
                            valid_ids.contains(session_id) || drawer_ids.contains(session_id)
                        });
                        this.open_tabs
                            .retain(|session_id| valid_ids.contains(session_id));
                        if archived {
                            this.active_tab = MissionTab::Feed;
                            this.open_tabs.clear();
                            let removed_mission_id = mission_id.clone();
                            this.update_app_settings(cx, true, move |settings| {
                                settings
                                    .last_mission_terminal_ids
                                    .remove(&removed_mission_id);
                                true
                            });
                            this.leave_archived_mission(
                                &mission_id,
                                was_archived,
                                archived,
                                window,
                                cx,
                            );
                        } else if matches!(
                            &this.active_tab,
                            MissionTab::Session(session_id) if !valid_ids.contains(session_id)
                        ) {
                            this.active_tab = MissionTab::Feed;
                        }
                        this.sync_mission_copy_entities(cx);
                        if let Err(error) = this.ensure_mission_terminals_attached(window, cx) {
                            this.error = Some(error.to_string());
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
