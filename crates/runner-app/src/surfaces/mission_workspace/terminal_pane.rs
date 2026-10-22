use std::sync::Arc;

use gpui::prelude::*;
use gpui::{div, px, rems, svg, AnyElement, FontWeight, MouseButton, SharedString, Window};
use runner_app::ui::{SessionControlKind, SessionOverlay, SessionOverlayKind};
use runner_core::protocol::model::SessionStatus;
use runner_core::protocol::session::SessionRow;

use super::*;
use crate::*;

impl MissionWorkspace {
    pub(super) fn render_mission_terminal_pane(
        &self,
        session: SessionRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session_id = session.session.id.clone();
        let status = self.slot_agent_status(&session_id, cx);
        let overlay = resolve_slot_overlay(
            self.archiving,
            if self.resuming {
                Some(MissionTransitionKind::Resuming)
            } else {
                self.transition_kind(&session_id)
            },
            session.session.status,
        );
        let terminal_background =
            crate::terminal::element::to_hsla(self.terminal_style(cx).palette.background, 1.);
        let Some(chat) = self.attached.get(&session_id) else {
            let pane = div()
                .absolute()
                .inset_0()
                .overflow_hidden()
                .bg(terminal_background);
            return match overlay {
                SlotOverlayState::Resuming => pane
                    .child(SessionOverlay::transition(
                        SharedString::from(format!("mission-resuming-{session_id}")),
                        SessionOverlayKind::Resuming,
                    ))
                    .into_any_element(),
                SlotOverlayState::Starting => pane
                    .child(SessionOverlay::transition(
                        SharedString::from(format!("mission-starting-{session_id}")),
                        SessionOverlayKind::Starting,
                    ))
                    .into_any_element(),
                SlotOverlayState::Stopped => pane
                    .child(self.render_stopped_slot_overlay(&session, cx))
                    .into_any_element(),
                SlotOverlayState::Archiving => pane.into_any_element(),
                SlotOverlayState::None => pane
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::i18n::text("ui.attaching_terminal"))
                    .into_any_element(),
            };
        };
        let terminal = Arc::clone(&chat.terminal);
        let terminal_interaction = chat.terminal_interaction.clone();
        let terminal_input = chat.terminal_input.clone();
        let terminal_focus = chat.terminal_focus.clone();
        let terminal_scrollbar = chat.terminal_scrollbar.clone();
        let interactive = self.cached_mission_terminal_interactive(&session_id, cx);
        let key_id = session_id.clone();
        let copy_id = session_id.clone();
        let paste_id = session_id.clone();
        let root = cx.entity();
        let copy_root = root.clone();
        let mut terminal_surface = div()
            .id(SharedString::from(format!("mission-terminal-{session_id}")))
            .absolute()
            .inset_0()
            .key_context("Terminal")
            .track_focus(&terminal_focus)
            .flex()
            .py_3()
            .pl_3()
            .pr_1()
            .bg(terminal_background)
            .opacity(match overlay {
                SlotOverlayState::Starting | SlotOverlayState::Resuming => 0.,
                SlotOverlayState::Stopped => 0.45,
                SlotOverlayState::Archiving | SlotOverlayState::None => 1.,
            })
            .on_action(move |action: &Copy, window, cx| {
                copy_root.update(cx, |this, cx| {
                    this.on_mission_terminal_copy(&copy_id, action, window, cx)
                });
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .pr(runner_app::ui::terminal_scrollbar_gutter())
                    .child(
                        TerminalElement::new(
                            terminal,
                            terminal_interaction,
                            terminal_input,
                            terminal_focus,
                            interactive,
                            true,
                            self.terminal_style(cx),
                        )
                        .scrollable(interactive)
                        .cached(),
                    )
                    .child(terminal_scrollbar),
            );
        if interactive || !chat.terminal.control().desktop() {
            let key_root = root.clone();
            let paste_root = root.clone();
            let drop_root = root.clone();
            let drop_id = session_id.clone();
            terminal_surface = terminal_surface
                .on_key_down(move |event, window, cx| {
                    key_root.update(cx, |this, cx| {
                        this.on_mission_key_down(&key_id, event, window, cx)
                    });
                })
                .on_action(move |action: &Paste, window, cx| {
                    paste_root.update(cx, |this, cx| {
                        this.on_mission_paste(&paste_id, action, window, cx)
                    });
                })
                .child(runner_app::ui::file_drop_target(
                    &session_id,
                    move |paths, window, cx| {
                        drop_root.update(cx, |this, cx| {
                            this.on_mission_file_drop(&drop_id, paths, window, cx)
                        });
                    },
                ));
        }
        let mut pane = div()
            .absolute()
            .inset_0()
            .overflow_hidden()
            .bg(terminal_background)
            .child(terminal_surface);
        if let Some(blocked) = (session.session.status == SessionStatus::Running)
            .then(|| self.delivery_blocked.get(&session_id).cloned())
            .flatten()
        {
            let idle =
                self.session_statuses().get(&session.handle) == Some(&SessionActivityState::Idle);
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
            let pane_width = f32::from(window.viewport_size().width)
                - sidebar_width
                - rail_width
                - 16. * self.settings(cx).app_zoom;
            pane = pane.child(self.render_inbox_blocked_pill(
                session_id.clone(),
                blocked,
                idle && !status.observation.needs_you(),
                status.observation.needs_you(),
                pane_width < 600. * self.settings(cx).app_zoom,
                cx,
            ));
        }
        pane = match overlay {
            SlotOverlayState::Resuming => pane.child(SessionOverlay::transition(
                SharedString::from(format!("mission-resuming-{session_id}")),
                SessionOverlayKind::Resuming,
            )),
            SlotOverlayState::Starting => pane.child(SessionOverlay::transition(
                SharedString::from(format!("mission-starting-{session_id}")),
                SessionOverlayKind::Starting,
            )),
            SlotOverlayState::Stopped => pane.child(self.render_stopped_slot_overlay(&session, cx)),
            SlotOverlayState::Archiving | SlotOverlayState::None => pane,
        };
        if session.session.status == SessionStatus::Running
            && !crate::terminal::control::mission_covered(
                self.control_session_ids(cx)
                    .iter()
                    .map(|id| self.attached.get(id)),
            )
        {
            pane = crate::terminal::control::cover(
                pane,
                format!("take-back-slot-{session_id}"),
                &[chat],
                crate::terminal::control::GroupKind::Tab,
            );
        }
        pane.into_any_element()
    }

    pub(super) fn control_session_ids(&self, cx: &App) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|session| session.session.status == SessionStatus::Running)
            .map(|session| session.session.id.clone())
            .chain(
                self.layout
                    .drawer
                    .shells()
                    .iter()
                    .filter(|id| {
                        self.drawer_session_entry(id, cx)
                            .is_some_and(|entry| entry.status == SessionStatus::Running)
                    })
                    .cloned(),
            )
            .collect()
    }

    fn render_inbox_blocked_pill(
        &self,
        session_id: String,
        blocked: DeliveryBlocked,
        idle: bool,
        needs_you: bool,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let unread_count = blocked.unread_count;
        let unsubmitted = blocked.reason.as_deref() == Some("unsubmitted");
        let root = cx.entity();
        let clear_root = root;
        div()
            .absolute()
            .right_4()
            .top_3()
            .flex()
            .items_center()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_color(theme::with_alpha(theme::warning(), 0.25))
            .bg(theme::panel())
            .px_3()
            .py_2()
            .text_size(theme::text_ui())
            .shadow_lg()
            .child(
                svg()
                    .flex_none()
                    .path("mail.svg")
                    .size(rems(14. / 16.))
                    .text_color(theme::warning()),
            )
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::warning())
                    .child(if unsubmitted {
                        crate::i18n::text("ui.message_not_submitted_press_return").into()
                    } else if unread_count > 1 {
                        crate::i18n::tf(
                            "format.inbox_waiting",
                            &[("unread_count", (unread_count).to_string())],
                        )
                    } else {
                        crate::i18n::text("ui.inbox_waiting").into()
                    }),
            )
            .children((!narrow && !unsubmitted).then(|| {
                div().text_color(theme::muted()).child(if needs_you {
                    crate::i18n::text("ui.respond_to_the_agent_s_prompt_to_release_delivery")
                } else {
                    crate::i18n::text(
                        "ui.a_draft_in_the_composer_is_holding_delivery_submit_or_clear_it",
                    )
                })
            }))
            .children(idle.then(|| {
                div()
                    .id(SharedString::from(format!(
                        "clear-mission-draft-{session_id}"
                    )))
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_md()
                    .border_1()
                    .border_color(theme::with_alpha(theme::warning(), 0.4))
                    .bg(theme::with_alpha(theme::warning(), 0.1))
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .text_size(theme::text_meta())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::warning())
                    .hover(|button| button.bg(theme::with_alpha(theme::warning(), 0.15)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        clear_root.update(cx, |this, cx| {
                            this.submit_or_clear_mission_input(&session_id, window, cx)
                        });
                    })
                    .child(crate::i18n::text("ui.submit_clear"))
                    .child(
                        div()
                            .text_size(theme::text_caption())
                            .font_weight(FontWeight::NORMAL)
                            .child("↵"),
                    )
            }))
            .into_any_element()
    }

    fn render_stopped_slot_overlay(
        &self,
        session: &SessionRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !mission_slot_actions_available(
            self.mission.as_ref().map(|mission| mission.status),
            self.archived(),
            self.secondary,
        ) {
            return div().into_any_element();
        }
        let others = self
            .sessions
            .iter()
            .filter(|other| {
                other.session.id != session.session.id
                    && other.session.status == SessionStatus::Running
            })
            .count();
        if others == 0 {
            return self.render_mission_paused_overlay(cx);
        }
        let resume = cx.entity();
        let restart = resume.clone();
        let resume_id = session.session.id.clone();
        let restart_id = resume_id.clone();
        SessionOverlay::ended(
            format!("slot-stopped-{resume_id}"),
            stopped_slot_description(&session.handle, others),
            move |window, cx| {
                resume.update(cx, |this, cx| {
                    this.act_on_slot(&resume_id, SessionControlKind::Resume, window, cx)
                })
            },
            move |window, cx| {
                restart.update(cx, |this, cx| {
                    this.request_slot_action(&restart_id, SessionControlKind::Restart, window, cx)
                })
            },
        )
        .slot_stopped()
        .into_any_element()
    }

    pub(super) fn render_mission_paused_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.roster.is_empty() {
            return div().into_any_element();
        }
        let root = cx.entity();
        let resume_root = root.clone();
        let archive_root = root;
        SessionOverlay::ended(
            "mission-paused",
            crate::i18n::text("ui.all_slots_are_paused_resume_to_respawn_every_slot_and_pick_up_the_conversation_the_event_l"),
            move |window, cx| {
                resume_root.update(cx, |this, cx| this.resume_open_mission(window, cx));
            },
            move |window, cx| {
                archive_root.update(cx, |this, cx| this.archive_open_mission(window, cx));
            },
        )
        .title(crate::i18n::text("ui.mission_paused"))
            .into_any_element()
    }

    pub(super) fn render_duplicate_mission_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        let stay_root = cx.entity();
        let primary_label = self.primary_label.clone();
        DuplicateSubjectOverlay::new(
            "duplicate-mission",
            DuplicateSubjectKind::Mission,
            primary_label.is_some(),
            move |_, cx| {
                if let Some(label) = primary_label.as_deref() {
                    focus_other_window(label, cx);
                }
            },
            move |_, cx| {
                stay_root.update(cx, |this, cx| {
                    this.duplicate_dismissed = true;
                    cx.notify();
                });
            },
        )
        .into_any_element()
    }
}
