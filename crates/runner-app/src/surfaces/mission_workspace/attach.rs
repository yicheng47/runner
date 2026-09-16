use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{App, Window};
use runner_app::terminal_ime::TerminalInput;
use runner_app::ui::CopyValueButton;
use runner_backend::model::SessionStatus;
use runner_backend::windows::Subject;

use super::*;
use crate::surfaces::*;
use crate::*;

impl MissionWorkspace {
    pub(crate) fn open_mission(
        &mut self,
        mission_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active = true;
        self.core(cx).windows.set_subjects(
            &self.window_label,
            vec![Subject::Mission(mission_id.clone())],
        );
        if window.is_window_active() {
            self.core(cx).windows.mark_focused(&self.window_label);
        } else {
            self.core(cx).windows.mark_blurred(&self.window_label);
        }
        self.core(cx).broadcast_focus_map();
        let generation = self.prepare_mission(mission_id.clone(), MissionRailView::Roles);
        self.composer_input.update(cx, |input, input_cx| {
            input.reset("", input_cx);
            input.set_disabled(false, input_cx);
        });
        self.mission_id_copy.update(cx, |copy, copy_cx| {
            copy.set_value(Some(mission_id.clone()), copy_cx)
        });
        window.focus(&self.root_focus);
        cx.notify();

        let loading_id = mission_id.clone();
        cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.is_current(&loading_id, generation) && this.loading {
                    this.loading_overlay_visible = true;
                    cx.notify();
                }
            });
        })
        .detach();

        let ticker_id = mission_id.clone();
        cx.spawn(async move |weak, cx| loop {
            cx.background_executor()
                .timer(Duration::from_secs(60))
                .await;
            let keep_running = weak
                .update(cx, |this, cx| {
                    let current = this.is_current(&ticker_id, generation);
                    if current {
                        cx.notify();
                    }
                    current
                })
                .unwrap_or(false);
            if !keep_running {
                break;
            }
        })
        .detach();

        let core = self.core(cx).clone();
        let load_id = mission_id.clone();
        let load = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_attach(&core, &load_id)
                .await
                .map_err(|error| error.to_string())?;
            let mission = runner_backend::ops::mission::mission_get(&core, &load_id)
                .map_err(|error| error.to_string())?;
            let sessions = runner_backend::ops::session::session_list(&core, &load_id)
                .map_err(|error| error.to_string())?;
            let events = runner_backend::ops::mission::mission_events_replay(&core, &load_id)
                .map_err(|error| error.to_string())?;
            let crew = runner_backend::ops::crew::crew_get(&core, &mission.crew_id).ok();
            let roster =
                runner_backend::ops::slot::slot_list(&core, &mission.crew_id).unwrap_or_default();
            Ok::<_, String>(MissionLoadResult {
                mission,
                crew,
                roster,
                sessions,
                events,
            })
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = load.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if !this.is_current(&mission_id, generation) || !this.is_active(cx) {
                    return;
                }
                match result {
                    Ok(mut loaded) => {
                        let mut seen = loaded
                            .events
                            .iter()
                            .map(|event| event.id.clone())
                            .collect::<HashSet<_>>();
                        for event in std::mem::take(&mut this.events) {
                            if seen.insert(event.id.clone()) {
                                loaded.events.push(event);
                            }
                        }
                        loaded.events.sort_by(|a, b| a.id.cmp(&b.id));
                        let archived = loaded.mission.archived_at.is_some();
                        let valid_ids = loaded
                            .sessions
                            .iter()
                            .map(|session| session.session.id.clone())
                            .collect::<HashSet<_>>();
                        let remembered = this
                            .settings(cx)
                            .last_mission_terminal_ids
                            .get(&mission_id)
                            .filter(|session_id| valid_ids.contains(*session_id))
                            .cloned();
                        this.mission = Some(loaded.mission);
                        this.crew = loaded.crew;
                        this.roster = loaded.roster;
                        this.sessions = loaded.sessions;
                        this.events = loaded.events;
                        this.rebuild_event_projection();
                        this.feed_scroll.scroll_to_bottom();
                        this.loading = false;
                        this.error = None;
                        if archived {
                            this.mission_node_id = None;
                            this.layout = MissionLayout::default();
                            this.active_tab = MissionTab::Feed;
                            this.open_tabs.clear();
                            let removed_mission_id = mission_id.clone();
                            this.update_app_settings(cx, true, move |settings| {
                                settings
                                    .last_mission_terminal_ids
                                    .remove(&removed_mission_id);
                                true
                            });
                        } else {
                            match this
                                .app_store
                                .update(cx, |store, store_cx| store.refresh_nodes(store_cx))
                            {
                                Ok(()) => {
                                    if let Err(error) = this.load_mission_layout(cx) {
                                        this.error = Some(error.to_string());
                                    }
                                }
                                Err(error) => this.error = Some(error.to_string()),
                            }
                            this.open_tabs = this
                                .sessions
                                .iter()
                                .map(|session| session.session.id.clone())
                                .collect();
                            this.active_tab = remembered
                                .map(MissionTab::Session)
                                .unwrap_or(MissionTab::Feed);
                        }
                        this.sync_mission_copy_entities(cx);
                        let active = this.is_active(cx);
                        this.sync_mission_subject_ownership(active, window, cx);
                        this.mark_active_session_viewed(window, cx);
                        if !this.secondary {
                            if let Err(error) = this.ensure_mission_terminals_attached(window, cx) {
                                this.error = Some(error.to_string());
                            }
                        }
                        this.focus_active_mission_terminal(window, cx);
                    }
                    Err(error) => {
                        this.loading = false;
                        this.error = Some(action_failure("load the mission", error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn sync_mission_copy_entities(&mut self, cx: &mut Context<Self>) {
        let sessions = self.sessions.clone();
        let valid = sessions
            .iter()
            .map(|session| session.session.id.clone())
            .collect::<HashSet<_>>();
        self.session_key_copies
            .retain(|session_id, _| valid.contains(session_id));
        for session in sessions {
            let label = format!("Copy @{} session_key", session.handle);
            let value = session.agent_session_key.clone();
            let copy = self
                .session_key_copies
                .entry(session.session.id.clone())
                .or_insert_with(|| {
                    cx.new(|copy_cx| {
                        CopyValueButton::new(copy_cx.focus_handle(), value.clone(), label)
                    })
                })
                .clone();
            copy.update(cx, |copy, copy_cx| copy.set_value(value, copy_cx));
        }
    }

    fn layout_estimated_mission_terminal_size(&self, window: &Window, cx: &App) -> (u16, u16) {
        let bounds = window.bounds().size;
        let sidebar_width = if self.sidebar_collapsed {
            0.
        } else {
            self.settings(cx).sidebar_width * self.settings(cx).app_zoom
        };
        let rail_width = if self.settings(cx).mission_rail_open {
            self.settings(cx).mission_rail_width * self.settings(cx).app_zoom
        } else {
            0.
        };
        let width = (f32::from(bounds.width) - sidebar_width - rail_width - 16.).max(200.);
        let height = (f32::from(bounds.height)
            - (WORKSPACE_HEADER_HEIGHT + WORKSPACE_TABS_HEIGHT + 24.) * self.settings(cx).app_zoom)
            .max(160.);
        let font_size = self.settings(cx).terminal_font_size as f32 * self.settings(cx).app_zoom;
        let cell_width = font_size * 0.6;
        let line_height = (font_size * crate::terminal::element::LINE_HEIGHT_FACTOR).round();
        (
            (width / cell_width).floor().max(2.) as u16,
            (height / line_height).floor().max(2.) as u16,
        )
    }

    pub(crate) fn estimated_mission_terminal_size(&self, window: &Window, cx: &App) -> (u16, u16) {
        preferred_terminal_size(
            None,
            self.last_measured_terminal_size,
            self.layout_estimated_mission_terminal_size(window, cx),
        )
    }

    pub(crate) fn estimated_mission_drawer_terminal_size(
        &self,
        window: &Window,
        cx: &App,
    ) -> (u16, u16) {
        self.estimated_mission_drawer_terminal_size_for_height(
            self.layout.drawer.height(),
            window,
            cx,
        )
    }

    pub(crate) fn estimated_mission_drawer_terminal_size_for_height(
        &self,
        drawer_height: f32,
        window: &Window,
        cx: &App,
    ) -> (u16, u16) {
        let bounds = window.bounds().size;
        let sidebar_width = if self.sidebar_collapsed {
            0.
        } else {
            self.settings(cx).sidebar_width * self.settings(cx).app_zoom
        };
        let rail_width = if self.settings(cx).mission_rail_open {
            self.settings(cx).mission_rail_width * self.settings(cx).app_zoom
        } else {
            0.
        };
        let width = (f32::from(bounds.width) - sidebar_width - rail_width - 16.).max(200.);
        let height = (drawer_height - (32. + 5. + 24.)).max(80.) * self.settings(cx).app_zoom;
        let font_size = self.settings(cx).terminal_font_size as f32 * self.settings(cx).app_zoom;
        let cell_width = font_size * 0.6;
        let line_height = (font_size * crate::terminal::element::LINE_HEIGHT_FACTOR).round();
        (
            (width / cell_width).floor().max(2.) as u16,
            (height / line_height).floor().max(2.) as u16,
        )
    }

    pub(crate) fn drawer_focused(&self, window: &Window, cx: &App) -> bool {
        self.layout.drawer.open() && self.drawer_focus.contains_focused(window, cx)
    }

    pub(crate) fn drawer_available(&self, cx: &App) -> bool {
        self.mission.is_some()
            && mission_drawer_available(self.archived(), self.secondary_state(cx).secondary)
    }

    pub(crate) fn current_mission_terminal_size(&self, window: &Window, cx: &App) -> (u16, u16) {
        let measured = match &self.active_tab {
            MissionTab::Session(session_id) => self
                .attached
                .get(session_id)
                .map(|chat| chat.terminal.size()),
            MissionTab::Feed => None,
        };
        preferred_terminal_size(
            measured,
            self.last_measured_terminal_size,
            self.layout_estimated_mission_terminal_size(window, cx),
        )
    }

    pub(super) fn cache_active_terminal_size(&mut self, window: &Window, cx: &App) {
        let MissionTab::Session(session_id) = &self.active_tab else {
            return;
        };
        if let Some(measured) = self
            .attached
            .get(session_id)
            .map(|chat| chat.terminal.size())
        {
            self.last_measured_terminal_size = Some(CachedTerminalSize {
                measured,
                layout_estimate: self.layout_estimated_mission_terminal_size(window, cx),
            });
        }
    }

    pub(crate) fn sync_mission_grid_hint(&self, window: &Window, cx: &App) {
        if !self.is_active(cx) || self.secondary {
            return;
        }
        let (cols, rows) = self.current_mission_terminal_size(window, cx);
        let _ = runner_backend::ops::mission::mission_grid_hint_set(self.core(cx), cols, rows);
    }

    pub(super) fn ensure_mission_terminals_attached(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if self.archived() || self.secondary_state(cx).secondary {
            return Ok(());
        }
        let mut errors = Vec::new();
        let layout = self.layout.clone();
        if let Err(error) = self.resume_visible_drawer_shell_on_launch(&layout, window, cx) {
            errors.push(error.to_string());
        }
        for session in self.sessions.clone() {
            if !self.open_tabs.contains(&session.session.id) {
                continue;
            }
            if let Err(error) = self.ensure_mission_terminal_attached(
                &session.session.id,
                session.session.status,
                window,
                cx,
            ) {
                errors.push(error.to_string());
            }
        }
        for session_id in self.layout.drawer.shells().to_vec() {
            let Some(status) = self
                .drawer_session_entry(&session_id, cx)
                .map(|entry| entry.status)
            else {
                continue;
            };
            if let Err(error) =
                self.ensure_mission_terminal_attached(&session_id, status, window, cx)
            {
                errors.push(error.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(errors.join("\n"))
        }
    }

    pub(super) fn resume_visible_drawer_shell_on_launch(
        &mut self,
        layout: &MissionLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let Some(session_id) = layout.drawer.active_shell() else {
            return Ok(());
        };
        let Some(status) = self
            .drawer_session_entry(session_id, cx)
            .map(|entry| entry.status)
        else {
            return Ok(());
        };
        let visible = self.is_active(cx)
            && layout.drawer.open()
            && !self.secondary_state(cx).secondary
            && !self.transitions.contains_key(session_id);
        let core = self.core(cx);
        if chat_lifecycle::take_visible_drawer_launch_claim(visible, status, || {
            runner_backend::ops::session::session_take_resume_on_launch(core, session_id)
        })? {
            self.resume_terminal_drawer_shell_on_launch(session_id, window, cx);
        }
        Ok(())
    }

    pub(super) fn ensure_mission_terminal_attached(
        &mut self,
        session_id: &str,
        status: SessionStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if self.secondary_state(cx).secondary {
            return Ok(());
        }
        if status != SessionStatus::Running {
            return Ok(());
        }
        if self.attached.contains_key(session_id) {
            return Ok(());
        }
        let Some(terminal) = self.app_store.read(cx).bridge.session(session_id) else {
            return Ok(());
        };
        terminal.set_palette(app_settings::terminal_palette(
            self.settings(cx),
            theme::active_variant(),
        ));
        terminal.configure(
            app_settings::TERMINAL_SCROLLBACK_LINES,
            match self.settings(cx).terminal_cursor_style {
                app_settings::TerminalCursorStyle::Block => {
                    alacritty_terminal::vte::ansi::CursorShape::Block
                }
                app_settings::TerminalCursorStyle::Underline => {
                    alacritty_terminal::vte::ansi::CursorShape::Underline
                }
                app_settings::TerminalCursorStyle::Bar => {
                    alacritty_terminal::vte::ansi::CursorShape::Beam
                }
            },
        );
        let terminal_scrollbar = cx.new(|_| Scrollbar::terminal(Arc::clone(&terminal)));
        let terminal_interaction = cx.new(|_| TerminalInteraction::new(Arc::clone(&terminal)));
        let terminal_focus = cx.focus_handle();
        let terminal_input = cx.new(|_| TerminalInput::new(Arc::clone(&terminal)));
        let input_session_id = session_id.to_owned();
        let terminal_input_subscription = cx.observe(&terminal_input, move |this, input, cx| {
            if let Some(Err(error)) = input.update(cx, |input, _| input.take_write_result()) {
                let visible = this.is_active(cx)
                    && (this
                        .sessions
                        .iter()
                        .any(|session| session.session.id == input_session_id)
                        || this.layout.drawer.shells().contains(&input_session_id));
                if visible {
                    this.error = Some(error);
                }
            }
            cx.notify();
        });
        let terminal_input_on_focus_out = terminal_input.clone();
        let terminal_focus_subscription =
            cx.on_focus_out(&terminal_focus, window, move |_, _, window, cx| {
                terminal_input_on_focus_out.update(cx, |input, input_cx| {
                    if input.cancel_composition() {
                        window.invalidate_character_coordinates();
                        input_cx.notify();
                    }
                });
            });
        self.attached.insert(
            session_id.to_owned(),
            AttachedChat {
                _terminal_view: terminal.view(),
                terminal,
                terminal_interaction,
                terminal_scrollbar,
                terminal_input,
                _terminal_input_subscription: terminal_input_subscription,
                _terminal_focus_subscription: terminal_focus_subscription,
                terminal_focus,
                scroll_accumulator: 0.,
            },
        );
        Ok(())
    }

    pub(super) fn begin_mission_transition(
        &mut self,
        session_id: &str,
        kind: MissionTransitionKind,
        baseline_seq: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.next_transition_generation += 1;
        let generation = self.next_transition_generation;
        let baseline_seq = baseline_seq.unwrap_or_else(|| {
            self.attached
                .get(session_id)
                .map(|chat| chat.terminal.output_activity().last_seq)
                .unwrap_or(0)
        });
        self.transitions.insert(
            session_id.to_owned(),
            MissionTransition {
                kind,
                started_at: Instant::now(),
                baseline_seq,
                generation,
            },
        );
        let tracked_id = session_id.to_owned();
        cx.spawn_in(window, async move |weak, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let done = weak
                .update_in(cx, |this, window, cx| {
                    let Some(transition) = this.transitions.get(&tracked_id).copied() else {
                        return true;
                    };
                    if transition.generation != generation {
                        return true;
                    }
                    if this.slot_actions.contains(&tracked_id) {
                        return false;
                    }
                    let now = Instant::now();
                    let activity = this
                        .attached
                        .get(&tracked_id)
                        .map(|chat| chat.terminal.output_activity());
                    let settled = chat_lifecycle::transition_should_settle(
                        match transition.kind {
                            MissionTransitionKind::Starting => {
                                chat_lifecycle::TransitionKind::Starting
                            }
                            MissionTransitionKind::Resuming | MissionTransitionKind::Restarting => {
                                chat_lifecycle::TransitionKind::Resuming
                            }
                        },
                        now.saturating_duration_since(transition.started_at),
                        activity.is_some_and(|activity| {
                            activity.first_paint_seq > transition.baseline_seq
                        }),
                        activity.is_some_and(|activity| {
                            activity.tui_ready_seq > transition.baseline_seq
                        }),
                        activity
                            .is_some_and(|activity| activity.last_seq > transition.baseline_seq),
                        activity
                            .and_then(|activity| activity.last_output_at)
                            .map(|last| now.saturating_duration_since(last)),
                    );
                    if settled {
                        this.transitions.remove(&tracked_id);
                        if drawer_session_should_take_focus(&this.layout, &tracked_id) {
                            this.focus_mission_drawer_terminal(&tracked_id, window, cx);
                        }
                        cx.notify();
                    }
                    settled
                })
                .unwrap_or(true);
            if done {
                break;
            }
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn sync_mission_subject_ownership(
        &mut self,
        active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !active {
            return;
        }
        let Some(mission_id) = self.mission_id.as_ref() else {
            return;
        };
        let state = runner_backend::ops::window::is_secondary_for(
            &self.core(cx).windows.snapshot(),
            &self.window_label,
            &Subject::Mission(mission_id.clone()),
        );
        let secondary = state.secondary;
        let primary = state.primary_label;
        if secondary != self.secondary {
            self.secondary = secondary;
            self.duplicate_dismissed = false;
            self.primary_label = primary;
            if secondary {
                self.attached.clear();
                window.focus(&self.root_focus);
            } else if let Err(error) = self.ensure_mission_terminals_attached(window, cx) {
                self.error = Some(error.to_string());
            }
            cx.notify();
        } else {
            self.primary_label = primary;
        }
    }

    pub(super) fn focus_active_mission_terminal(&self, window: &mut Window, cx: &App) {
        if self.rename_modal.is_some() || self.stop_all_confirm || self.restart_confirm.is_some() {
            return;
        }
        let MissionTab::Session(session_id) = &self.active_tab else {
            window.focus(&self.root_focus);
            return;
        };
        if self.mission_terminal_interactive(session_id, cx) {
            if let Some(chat) = self.attached.get(session_id) {
                chat.terminal_focus.focus(window);
                return;
            }
        }
        window.focus(&self.root_focus);
    }

    pub(super) fn focus_mission_drawer_terminal(
        &self,
        session_id: &str,
        window: &mut Window,
        cx: &App,
    ) {
        if self.mission_terminal_interactive(session_id, cx) {
            if let Some(chat) = self.attached.get(session_id) {
                chat.terminal_focus.focus(window);
                return;
            }
        }
        self.drawer_focus.focus(window);
    }
}
