use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, ClipboardItem, CursorStyle, FontWeight, KeyDownEvent,
    MouseButton, SharedString, Window,
};
use runner_app::ui::{RoleAvatar, Tooltip};
use runner_backend::model::{Event, EventKind};

use super::*;
use crate::surfaces::mission_feed::{message_target, message_text, FeedBlock};
use crate::surfaces::mission_markdown::{
    FeedPosition, FeedSelection, FeedSelectionHandler, FeedSelectionPhase,
};
use crate::*;

impl MissionWorkspace {
    pub(super) fn session_title_tooltip(&self, session: &SessionRow, cx: &App) -> String {
        let title = self
            .attached
            .get(&session.session.id)
            .map(|chat| chat.terminal.title())
            .or_else(|| {
                self.app_store
                    .read(cx)
                    .bridge
                    .session(&session.session.id)
                    .map(|terminal| terminal.title())
            })
            .and_then(|title| {
                runner_backend::session::title::provider_title(
                    &title,
                    session.session.cwd.as_deref(),
                )
            })
            .or_else(|| {
                session.live_title.as_deref().and_then(|title| {
                    runner_backend::session::title::provider_title(
                        title,
                        session.session.cwd.as_deref(),
                    )
                })
            });
        match title {
            Some(title) => format!("@{} · {title}", session.handle),
            None => format!("@{}", session.handle),
        }
    }

    pub(super) fn render_mission_tabs(
        &self,
        feed_active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let feed_root = root.clone();
        let mut strip = div()
            .h(rems(WORKSPACE_TABS_HEIGHT / 16.))
            .flex_none()
            .px_6()
            .flex()
            .items_end()
            .gap_1()
            .border_b_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .child(
                mission_tab("mission-feed-tab", "feed", feed_active).on_click(
                    move |_, window, cx| {
                        feed_root.update(cx, |this, cx| this.select_mission_feed(window, cx));
                    },
                ),
            );
        if !self.archived() && !self.secondary {
            for session_id in &self.open_tabs {
                let Some(session) = self
                    .sessions
                    .iter()
                    .find(|session| &session.session.id == session_id)
                else {
                    continue;
                };
                let active = self.active_tab == MissionTab::Session(session_id.clone());
                let status = self.slot_agent_status(session_id, cx);
                let rollup = runner_app::ui::agent_status::StatusRollup {
                    entries: vec![(session_id.clone(), status)],
                };
                let select_id = session_id.clone();
                let close_id = session_id.clone();
                let select_root = root.clone();
                let close_root = root.clone();
                strip = strip.child(Tooltip::new(
                    SharedString::from(format!("mission-tab-tooltip-{session_id}")),
                    self.session_title_tooltip(session, cx),
                    div()
                        .id(SharedString::from(format!("mission-tab-{session_id}")))
                        .relative()
                        .h(rems(32. / 16.))
                        .flex_none()
                        .px(rems(14. / 16.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_b_2()
                        .border_color(if active {
                            theme::accent()
                        } else {
                            gpui::transparent_black()
                        })
                        .cursor_pointer()
                        .text_size(theme::text_body())
                        .text_color(if active {
                            theme::text()
                        } else {
                            theme::muted()
                        })
                        .hover(|tab| tab.text_color(theme::text()))
                        .child(
                            svg()
                                .path("terminal.svg")
                                .size(rems(12. / 16.))
                                .flex_none()
                                .text_color(if active {
                                    theme::text()
                                } else {
                                    theme::muted()
                                }),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .max_w(rems(140. / 16.))
                                .truncate()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .child(format!("@{}", session.handle)),
                        )
                        .when(
                            runner_backend::model::Runtime::parse(&session.runtime)
                                != Some(runner_backend::model::Runtime::Shell),
                            |tab| {
                                tab.child(rollup.render(SharedString::from(format!(
                                    "mission-tab-status-{session_id}"
                                ))))
                            },
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!(
                                    "close-mission-tab-{session_id}"
                                )))
                                .group("mission-tab-close")
                                .size_4()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_sm()
                                .text_color(theme::faint())
                                .hover(|button| {
                                    button.bg(theme::raised()).text_color(theme::text())
                                })
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    close_root.update(cx, |this, cx| {
                                        this.close_mission_session_tab(&close_id, window, cx)
                                    });
                                })
                                .child(
                                    svg()
                                        .flex_none()
                                        .path("close.svg")
                                        .size(rems(12. / 16.))
                                        .text_color(theme::faint())
                                        .group_hover("mission-tab-close", |icon| {
                                            icon.text_color(theme::text())
                                        }),
                                ),
                        )
                        .on_click(move |_, window, cx| {
                            select_root.update(cx, |this, cx| {
                                this.select_mission_session(&select_id, window, cx)
                            });
                        }),
                ));
            }
        }
        strip.into_any_element()
    }

    pub(super) fn render_mission_feed(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.feed_was_near_bottom = self.feed_is_near_bottom();
        if self.feed_was_near_bottom {
            self.feed_has_new_messages = false;
        }
        let blocks = self.feed_blocks.clone();
        let rows = if blocks.is_empty() {
            vec![div()
                .px_4()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child("No events yet.")
                .into_any_element()]
        } else {
            blocks
                .into_iter()
                .map(|block| self.render_mission_feed_block(block, cx))
                .collect()
        };
        let root = cx.entity();
        let pill_root = root.clone();
        let feed_up_root = root.clone();
        let feed_out_root = root.clone();
        let pane = div()
            .relative()
            .min_h(px(0.))
            .flex_1()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .child(
                div()
                    .id("mission-feed-scroll")
                    .key_context("MissionFeed")
                    .track_focus(&self.root_focus)
                    .min_h(px(0.))
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.feed_scroll)
                    .px_6()
                    .py_6()
                    .flex()
                    .flex_col()
                    .gap(rems(18. / 16.))
                    .children(rows)
                    .on_action(cx.listener(Self::copy_feed_selection))
                    .on_key_down(cx.listener(Self::on_feed_key_down))
                    .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                        feed_up_root.update(cx, |this, _| {
                            this.feed_selecting = false;
                        });
                    })
                    .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                        feed_out_root.update(cx, |this, _| {
                            this.feed_selecting = false;
                        });
                    }),
            )
            .children(self.feed_has_new_messages.then(|| {
                div()
                    .absolute()
                    .bottom_4()
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .id("mission-feed-new-messages")
                            .px_3()
                            .py_1()
                            .rounded_full()
                            .bg(theme::accent())
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::accent_ink())
                            .shadow_md()
                            .cursor_pointer()
                            .hover(|pill| pill.opacity(0.9))
                            .on_click(move |_, _, cx| {
                                pill_root.update(cx, |this, cx| {
                                    this.feed_scroll.scroll_to_bottom();
                                    this.feed_was_near_bottom = true;
                                    this.feed_has_new_messages = false;
                                    cx.notify();
                                });
                            })
                            .child("New messages ↓"),
                    )
            }));
        pane.into_any_element()
    }

    pub(super) fn clear_feed_selection(&mut self) {
        self.feed_selection = None;
        self.feed_selecting = false;
    }

    fn update_feed_selection(
        &mut self,
        phase: FeedSelectionPhase,
        event_id: &str,
        position: FeedPosition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match phase {
            FeedSelectionPhase::Begin => {
                self.feed_selection = Some(FeedSelection {
                    event_id: event_id.to_owned(),
                    anchor: position,
                    head: position,
                });
                self.feed_selecting = true;
                window.focus(&self.root_focus);
                cx.notify();
            }
            FeedSelectionPhase::Extend if self.feed_selecting => {
                let Some(selection) = self.feed_selection.as_mut() else {
                    return;
                };
                if selection.event_id == event_id && selection.head != position {
                    selection.head = position;
                    cx.notify();
                }
            }
            FeedSelectionPhase::End => {
                if self.feed_selecting {
                    if let Some(selection) = self.feed_selection.as_mut() {
                        if selection.event_id == event_id {
                            selection.head = position;
                        }
                    }
                }
                self.feed_selecting = false;
                cx.notify();
            }
            FeedSelectionPhase::Extend => {}
        }
    }

    fn feed_selection_handler(&self, cx: &mut Context<Self>) -> FeedSelectionHandler {
        let root = cx.entity();
        FeedSelectionHandler::new(move |phase, event_id, position, window, cx| {
            root.update(cx, |this, cx| {
                this.update_feed_selection(phase, event_id, position, window, cx)
            });
        })
    }

    fn render_feed_markdown(
        &self,
        event_id: &str,
        text: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let handler = self.feed_selection_handler(cx);
        let selection_color = crate::terminal::element::to_hsla(
            app_settings::terminal_palette(self.settings(cx), theme::active_variant()).selection,
            1.,
        );
        crate::surfaces::mission_markdown::render_markdown(
            event_id,
            text,
            cx.entity_id(),
            self.feed_selection.as_ref(),
            selection_color,
            Some(&handler),
            cx,
        )
    }

    fn on_feed_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" && self.feed_selection.is_some() {
            self.clear_feed_selection();
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn copy_feed_selection(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = self.feed_selection.as_ref() else {
            return;
        };
        let Some(event) = self
            .events
            .iter()
            .find(|event| event.id == selection.event_id)
        else {
            return;
        };
        let source = selectable_event_text(event);
        let Some(text) = crate::surfaces::mission_markdown::selected_plain_text(&source, selection)
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        cx.stop_propagation();
    }

    fn render_mission_feed_block(&self, block: FeedBlock, cx: &mut Context<Self>) -> AnyElement {
        let id = block.id().to_owned();
        match block {
            FeedBlock::Divider(event) => div()
                .id(SharedString::from(format!("mission-divider-{id}")))
                .px_4()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .h(rems(1. / 16.))
                        .min_w(px(0.))
                        .flex_1()
                        .bg(theme::border()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(theme::text_caption())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::faint())
                        .child(format!("MISSION STARTED · {}", format_event_time(&event))),
                )
                .child(
                    div()
                        .h(rems(1. / 16.))
                        .min_w(px(0.))
                        .flex_1()
                        .bg(theme::border()),
                )
                .into_any_element(),
            FeedBlock::MessageGroup { author, events } => self
                .render_mission_message_group(author, events, cx)
                .into_any_element(),
            FeedBlock::Signal(event) => self.render_mission_signal_row(event),
            FeedBlock::AskCard(event) => self.render_mission_ask_card(event, cx),
        }
    }

    fn render_mission_message_group(
        &self,
        author: String,
        events: Vec<Event>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let first = &events[0];
        let human = author == "human";
        let target = message_target(first, &self.askers_by_question);
        let goal = first.kind == EventKind::Signal
            && first.signal_type.as_ref().map(|kind| kind.as_str()) == Some("mission_goal");
        div()
            .id(SharedString::from(format!(
                "mission-message-group-{}",
                first.id
            )))
            .px_4()
            .flex()
            .items_start()
            .gap_3()
            .child(RoleAvatar::new(author.clone(), 35.))
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(
                                div()
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_body())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(runner_app::ui::hue_for_seed(&author).color())
                                    .child(if human {
                                        "you".into()
                                    } else {
                                        format!("@{author}")
                                    }),
                            )
                            .children(goal.then(|| {
                                div()
                                    .rounded_sm()
                                    .bg(theme::raised())
                                    .px_1()
                                    .py(rems(2. / 16.))
                                    .text_size(theme::text_micro())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::muted())
                                    .child("GOAL")
                            }))
                            .children(target.map(|target| {
                                div()
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::muted())
                                    .child(format!("→ @{target}"))
                            }))
                            .child(div().flex_none().child(format_event_time(first))),
                    )
                    .child(
                        div()
                            .mt_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .text_size(theme::text_body())
                            .text_color(theme::text())
                            .children(events.into_iter().filter_map(|event| {
                                let text = message_text(&event);
                                if text.is_empty() {
                                    goal.then(|| {
                                        div()
                                            .text_color(theme::faint())
                                            .child("(no text)")
                                            .into_any_element()
                                    })
                                } else {
                                    Some(self.render_feed_markdown(&event.id, &text, cx))
                                }
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_mission_signal_row(&self, event: Event) -> AnyElement {
        let event_id = event.id.clone();
        let signal = event
            .signal_type
            .as_ref()
            .map(|kind| kind.as_str())
            .unwrap_or("?");
        let warning = signal == "mission_warning";
        let restart_summary = slot_restart_signal_summary(&event);
        let payload = if signal == "ask_lead" {
            event
                .payload
                .get("question")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        } else if warning {
            event
                .payload
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        } else {
            serde_json::to_string_pretty(&event.payload)
                .unwrap_or_else(|_| event.payload.to_string())
        };
        let avatar_seed = if event.from == "human" {
            "human".to_owned()
        } else {
            event.from.clone()
        };
        div()
            .id(SharedString::from(format!("mission-signal-{event_id}")))
            .px_4()
            .flex()
            .items_start()
            .gap_3()
            .child(RoleAvatar::new(avatar_seed, 35.))
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(theme::text_meta())
                            .child(
                                div()
                                    .flex_none()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_body())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(runner_app::ui::hue_for_seed(&event.from).color())
                                    .child(
                                        if signal == "slot_restarted" && event.from == "human" {
                                            "you".to_owned()
                                        } else {
                                            format!("@{}", event.from)
                                        },
                                    ),
                            )
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_color(if warning {
                                        theme::danger()
                                    } else {
                                        theme::faint()
                                    })
                                    .child(if let Some(summary) = restart_summary {
                                        format!("· {summary}")
                                    } else if warning {
                                        format!("· warning · {}", format_event_time(&event))
                                    } else {
                                        format!(
                                            "· signal · {signal}{} · {}",
                                            event
                                                .to
                                                .as_ref()
                                                .map(|to| format!(" → @{to}"))
                                                .unwrap_or_default(),
                                            format_event_time(&event)
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .mt_2()
                            .rounded_md()
                            .border_1()
                            .border_color(if warning {
                                theme::with_alpha(theme::danger(), 0.3)
                            } else {
                                theme::border()
                            })
                            .bg(if warning {
                                theme::with_alpha(theme::danger(), 0.05)
                            } else {
                                theme::bg()
                            })
                            .p_3()
                            .when(!warning, |payload| {
                                payload.font_family(theme::UI_MONOSPACE_FONT)
                            })
                            .text_size(theme::text_ui())
                            .line_height(rems(17. / 16.))
                            .text_color(if warning {
                                theme::danger()
                            } else {
                                theme::muted()
                            })
                            .child(payload),
                    ),
            )
            .into_any_element()
    }

    fn render_mission_ask_card(&self, event: Event, cx: &mut Context<Self>) -> AnyElement {
        let question_id = event.id.clone();
        let asker = self
            .askers_by_question
            .get(&question_id)
            .cloned()
            .unwrap_or_else(|| "?".into());
        let prompt = event
            .payload
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let choices = event
            .payload
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|items| !items.is_empty())
            .unwrap_or_else(|| vec!["yes".into(), "no".into()]);
        let on_behalf = event
            .payload
            .get("on_behalf_of")
            .and_then(serde_json::Value::as_str);
        let chain = match on_behalf {
            Some(handle) if handle != asker => format!("@{handle} → @{asker} → you"),
            _ => "→ you".to_owned(),
        };
        let resolved = self.resolved_asks.get(&question_id).cloned();
        let pending_choice = self.pending_ask_choices.get(&question_id).cloned();
        let submitting = self.submitting_asks.contains(&question_id);
        let root = cx.entity();
        let mut buttons = div().mt_3().flex().flex_col().gap_1();
        for (index, choice) in choices.into_iter().enumerate() {
            let action_root = root.clone();
            let action_question = question_id.clone();
            let action_choice = choice.clone();
            let picked = resolved.as_deref() == Some(choice.as_str())
                || pending_choice.as_deref() == Some(choice.as_str());
            buttons = buttons.child(
                div()
                    .id(SharedString::from(format!(
                        "mission-answer-{question_id}-{index}"
                    )))
                    .w_full()
                    .rounded_md()
                    .border_1()
                    .border_color(if index == 0 || picked {
                        theme::accent()
                    } else {
                        theme::border()
                    })
                    .bg(if index == 0 {
                        theme::accent()
                    } else {
                        theme::panel()
                    })
                    .px_3()
                    .py_2()
                    .cursor(if submitting || resolved.is_some() {
                        CursorStyle::OperationNotAllowed
                    } else {
                        CursorStyle::PointingHand
                    })
                    .opacity(if submitting || resolved.is_some() {
                        0.6
                    } else {
                        1.
                    })
                    .text_size(theme::text_ui())
                    .font_weight(if index == 0 {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .text_color(if index == 0 {
                        theme::accent_ink()
                    } else if picked {
                        theme::accent()
                    } else {
                        theme::text()
                    })
                    .when(!submitting && resolved.is_none(), |button| {
                        button.hover(|button| button.border_color(theme::border_strong()))
                    })
                    .on_click(move |_, _, cx| {
                        action_root.update(cx, |this, cx| {
                            this.answer_mission_question(
                                action_question.clone(),
                                action_choice.clone(),
                                cx,
                            )
                        });
                    })
                    .child(choice),
            );
        }
        div()
            .id(SharedString::from(format!("mission-ask-{question_id}")))
            .px_4()
            .flex()
            .items_start()
            .gap_3()
            .child(RoleAvatar::new(asker.clone(), 35.))
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_body())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(runner_app::ui::hue_for_seed(&asker).color())
                                    .child(format!("@{asker}")),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .rounded_sm()
                                    .bg(theme::with_alpha(theme::warning(), 0.1))
                                    .px_1()
                                    .py(rems(2. / 16.))
                                    .text_size(theme::text_micro())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::warning())
                                    .child("NEEDS YOUR INPUT"),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::muted())
                                    .child(chain),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(format_event_time(&event)),
                            ),
                    )
                    .child(
                        div()
                            .mt_1()
                            .rounded_lg()
                            .border_1()
                            .border_color(theme::with_alpha(theme::warning(), 0.6))
                            .bg(theme::with_alpha(theme::warning(), 0.1))
                            .p_4()
                            .text_size(theme::text_body())
                            .text_color(theme::text())
                            .child(if prompt.is_empty() {
                                div()
                                    .text_size(theme::text_body())
                                    .text_color(theme::faint())
                                    .child("(no prompt)")
                                    .into_any_element()
                            } else {
                                self.render_feed_markdown(&event.id, &prompt, cx)
                            })
                            .child(buttons)
                            .children(resolved.map(|choice| {
                                div()
                                    .mt_1()
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(format!("answered: {choice}"))
                            })),
                    ),
            )
            .into_any_element()
    }

    fn answer_mission_question(
        &mut self,
        question_id: String,
        choice: String,
        cx: &mut Context<Self>,
    ) {
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        if self.secondary_state(cx).secondary {
            return;
        }
        if self.resolved_asks.contains_key(&question_id)
            || !self.submitting_asks.insert(question_id.clone())
        {
            return;
        }
        self.pending_ask_choices
            .insert(question_id.clone(), choice.clone());
        cx.notify();
        let core = self.core(cx).clone();
        let post_question = question_id.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_post_human_signal_impl(
                &core,
                runner_backend::ops::mission::PostHumanSignalInput {
                    mission_id,
                    signal_type: "human_response".into(),
                    payload: serde_json::json!({
                        "question_id": post_question,
                        "choice": choice,
                    }),
                },
            )
            .await
            .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.submitting_asks.remove(&question_id);
                if let Err(error) = result {
                    this.pending_ask_choices.remove(&question_id);
                    this.error = Some(action_failure("answer the question", error));
                }
                cx.notify();
            });
        })
        .detach();
    }
}
