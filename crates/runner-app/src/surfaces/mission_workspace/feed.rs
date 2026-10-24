use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, ClipboardItem, CursorStyle, FontWeight, KeyDownEvent,
    MouseButton, SharedString, Window,
};
use runner_app::ui::{app_scrollbar_gutter, Button, ButtonVariant, RoleAvatar, Tooltip};
use runner_core::protocol::model::{Event, EventKind};

use super::*;
use crate::surfaces::mission_feed::{message_target, FeedBlock};
use crate::surfaces::mission_markdown::{
    FeedPosition, FeedSelection, FeedSelectionHandler, FeedSelectionPhase,
};
use crate::*;

impl MissionWorkspace {
    pub(super) fn session_title_tooltip(
        &self,
        session: &crate::surfaces::front_page::model::MissionMemberDisplay,
        cx: &App,
    ) -> String {
        if self.remote.is_some() {
            return session
                .live_title
                .as_ref()
                .filter(|title| !title.is_empty())
                .map(|title| format!("@{} · {title}", session.handle))
                .unwrap_or_else(|| format!("@{}", session.handle));
        }
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
                runner_core::protocol::session_title::provider_title(
                    &title,
                    session.session.cwd.as_deref(),
                )
            })
            .or_else(|| {
                session.live_title.as_deref().and_then(|title| {
                    runner_core::protocol::session_title::provider_title(
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
            .id("mission-tabs")
            .debug_selector(|| "MISSION_TABS".into())
            .h(rems(WORKSPACE_TABS_HEIGHT / 16.))
            .w_full()
            .flex_none()
            .px_4()
            .flex()
            .items_center()
            .gap_1()
            .overflow_x_scroll()
            .border_b_1()
            .border_color(theme::border())
            .map(|element| {
                #[cfg(test)]
                let element = crate::theme_snapshot::record_fill("MISSION_TABS", element);
                element
            })
            .child(
                mission_tab(
                    "mission-feed-tab",
                    crate::i18n::text("word.feed"),
                    feed_active,
                )
                .on_click(move |_, window, cx| {
                    feed_root.update(cx, |this, cx| this.select_mission_feed(window, cx));
                }),
            );
        if !self.archived() && !self.secondary {
            let members = self.members();
            for session_id in &self.open_tabs {
                let Some(session) = members
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
                let tab = Tooltip::new(
                    SharedString::from(format!("mission-tab-tooltip-{session_id}")),
                    self.session_title_tooltip(session, cx),
                    div()
                        .id(SharedString::from(format!("mission-tab-{session_id}")))
                        .debug_selector({
                            let session_id = session_id.clone();
                            move || format!("MISSION_TAB {session_id}")
                        })
                        .relative()
                        .h(rems(28. / 16.))
                        .min_w(px(0.))
                        .px(rems(12. / 16.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .when(active, |tab| tab.bg(theme::panel()))
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
                            !runner_core::protocol::model::Runtime::parse(&session.runtime)
                                .is_some_and(runner_core::protocol::model::Runtime::is_shell),
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
                                .flex_none()
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
                );
                strip = strip.child(
                    div()
                        .min_w(rems(MISSION_TAB_MIN_WIDTH / 16.))
                        .overflow_hidden()
                        .child(tab),
                );
            }
        }
        strip.into_any_element()
    }

    pub(super) fn render_mission_feed(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.feed_vertical_padding = px(48. * self.settings(cx).app_zoom);
        self.feed_was_near_bottom = self.feed_is_near_bottom();
        if self.feed_was_near_bottom {
            self.feed_has_new_messages = false;
        }
        let count = self.feed_item_count();
        let old_count = self.feed_list.item_count();
        if count != old_count {
            let start = count.min(old_count);
            self.splice_feed_items(start..old_count, count - start);
        }
        let scroll_root = cx.weak_entity();
        self.feed_list.set_scroll_handler(move |_, window, cx| {
            let root = scroll_root.clone();
            window.defer(cx, move |_, cx| {
                let _ = root.update(cx, |this, cx| {
                    this.feed_was_near_bottom = this.feed_is_near_bottom();
                    if this.feed_was_near_bottom && this.feed_has_new_messages {
                        this.feed_has_new_messages = false;
                        cx.notify();
                    }
                });
            });
        });
        let empty_root = cx.weak_entity();
        let metrics_root = cx.weak_entity();
        let root = cx.entity();
        let pill_root = root.clone();
        let feed_up_root = root.clone();
        let feed_out_root = root.clone();
        let remote_feed = self.remote.as_ref().and_then(|remote| remote.feed.clone());
        let feed_state = remote_feed.as_ref().map(|feed| {
            let feed = feed.read(cx);
            (
                feed.loading,
                feed.older_loading,
                feed.before.is_some(),
                feed.can_load_older(),
                feed.error.clone(),
            )
        });
        let pane = div()
            .relative()
            .min_h(px(0.))
            .flex_1()
            .flex()
            .flex_col()
            .bg(theme::bg())
            .children(feed_state.map(|(loading, older_loading, older, can_load_older, error)| {
                let feed = remote_feed.unwrap().downgrade();
                let retry = feed.clone();
                let failed = error.is_some();
                div()
                    .flex_none().px_4().py_1().flex().items_center().gap_2()
                    .children(error.map(|error| div().text_color(theme::danger()).text_size(theme::text_meta()).child(error)))
                    .children((loading || older_loading).then(|| crate::i18n::text("ui.loading")))
                    .children((older && !loading).then(|| {
                        Button::new("remote-feed-older", crate::i18n::text("mission.load_older"))
                            .variant(ButtonVariant::Ghost).disabled(!can_load_older)
                            .on_press(move |_, cx| { let _ = feed.update(cx, |feed, cx| feed.older(cx)); })
                    }))
                    .children(failed.then(|| {
                        Button::new("remote-feed-refresh", crate::i18n::text("ui.retry"))
                            .variant(ButtonVariant::Ghost).disabled(loading || older_loading)
                            .on_press(move |_, cx| { let _ = retry.update(cx, |feed, cx| feed.retry(cx)); })
                    }))
            }))
            .child(
                div()
                    .id("mission-feed-scroll")
                    .key_context("MissionFeed")
                    .track_focus(&self.root_focus)
                    .min_h(px(0.))
                    .flex_1()
                    .overflow_hidden()
                    .px_6()
                    .flex()
                    .flex_col()
                    .child(
                        gpui::list(self.feed_list.clone(), move |index, _, cx| {
                            root.update(cx, |this, cx| {
                                let entry = this.feed_rows.get(index).copied();
                                let row = if let Some(entry) = entry {
                                    this.render_mission_feed_row(entry, cx)
                                } else if this.feed_rows.is_empty() && index == 0 {
                                    div()
                                        .px_4()
                                        .text_size(theme::text_ui())
                                        .text_color(theme::faint())
                                        .child(crate::i18n::text("ui.no_events_yet"))
                                        .into_any_element()
                                } else {
                                    this.render_empty_mission(cx)
                                };
                                div()
                                    .w_full()
                                    .flex()
                                    .flex_col()
                                    .when(index >= this.feed_rows.len().max(1), |row| {
                                        row.min_h(this.feed_empty_panel_min_height.get())
                                    })
                                    .when(index > 0 && !matches!(entry, Some(FeedRow::Message { event, part, .. }) if event > 0 || part > 0), |row| row.pt(rems(18. / 16.)))
                                    .child(row)
                                    .into_any_element()
                            })
                        })
                        .min_h(px(0.))
                        .flex_1()
                        .w_full()
                        .py_6(),
                    )
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
            .child(
                gpui::canvas(
                    |_, _, _| {},
                    move |_, _, window, cx| {
                        let root = metrics_root.clone();
                        window.defer(cx, move |_, cx| {
                            let _ = root.update(cx, |this, cx| {
                                let width = this.feed_list.viewport_bounds().size.width;
                                if this.feed_measured_width != Some(width) {
                                    this.feed_measured_width = Some(width);
                                    // GPUI discards height hints on width changes, including the first layout.
                                    this.feed_list.clone().with_uniform_item_height(px(
                                        FEED_ROW_HEIGHT_HINT * this.settings(cx).app_zoom,
                                    ));
                                    this.feed_needs_height_hints.set(false);
                                    cx.notify();
                                    return;
                                }
                                let metrics = this.measure_feed_scrollbar();
                                let previous = this.feed_scrollbar_metrics.replace(metrics);
                                if previous != metrics {
                                    this.feed_scrollbar.update(cx, |_, cx| cx.notify());
                                    if (previous.content - previous.viewport >= 1.)
                                        != (metrics.content - metrics.viewport >= 1.)
                                    {
                                        cx.notify();
                                    }
                                }
                            });
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
            .children(
                (self.feed_scrollbar_metrics.get().content
                    - self.feed_scrollbar_metrics.get().viewport
                    >= 1.)
                    .then(|| {
                        div()
                            .id("mission-feed-scrollbar")
                            .debug_selector(|| "MISSION_FEED_SCROLLBAR".into())
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .w(app_scrollbar_gutter())
                            .capture_any_mouse_down(cx.listener(
                                |this, event: &gpui::MouseDownEvent, _, cx| {
                                    if event.button == MouseButton::Left {
                                        this.feed_selecting = false;
                                        this.prepare_feed_scrollbar(cx);
                                        this.feed_list.scrollbar_drag_started();
                                    }
                                },
                            ))
                            .capture_any_mouse_up(cx.listener(
                                |this, event: &gpui::MouseUpEvent, _, cx| {
                                    if event.button == MouseButton::Left {
                                        this.end_feed_scrollbar_drag(cx);
                                    }
                                },
                            ))
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| this.end_feed_scrollbar_drag(cx)),
                            )
                            .on_scroll_wheel(cx.listener(
                                |this, event: &gpui::ScrollWheelEvent, _, cx| {
                                    this.prepare_feed_scrollbar(cx);
                                    let position =
                                        -f32::from(
                                            this.feed_list.scroll_px_offset_for_scrollbar().y,
                                        ) - f32::from(event.delta.pixel_delta(px(20.)).y);
                                    this.feed_list.set_offset_from_scrollbar(gpui::point(
                                        px(0.),
                                        px(-position.max(0.)),
                                    ));
                                    cx.stop_propagation();
                                    cx.notify();
                                },
                            ))
                            .child(self.feed_scrollbar.clone())
                    }),
            )
            .children((count > self.feed_rows.len().max(1)).then(|| {
                gpui::canvas(
                    |_, _, _| {},
                    move |_, _, window, cx| {
                        let root = empty_root.clone();
                        let padding = window.rem_size() * 3.;
                        window.defer(cx, move |_, cx| {
                            let _ = root.update(cx, |this, cx| {
                                if this.feed_item_count() <= this.feed_rows.len().max(1) {
                                    return;
                                }
                                let panel = this.feed_item_count() - 1;
                                if let Some(bounds) = this.feed_list.bounds_for_item(panel) {
                                    let viewport = this.feed_list.viewport_bounds();
                                    let prefix = bounds.top()
                                        - viewport.top()
                                        - this.feed_list.scroll_px_offset_for_scrollbar().y;
                                    let height =
                                        (viewport.size.height - padding - prefix).max(px(0.));
                                    if (height - this.feed_empty_panel_min_height.get()).abs()
                                        > px(0.5)
                                    {
                                        this.feed_empty_panel_min_height.set(height);
                                        this.splice_feed_items(panel..panel + 1, 1);
                                        cx.notify();
                                    }
                                }
                            });
                        });
                    },
                )
                .absolute()
                .inset_0()
            }))
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
                            .debug_selector(|| "mission-feed-new-messages".into())
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
                                    this.scroll_feed_to_bottom();
                                    cx.notify();
                                });
                            })
                            .child(crate::i18n::text("ui.new_messages")),
                    )
            }));
        pane.into_any_element()
    }

    fn prepare_feed_scrollbar(&self, cx: &App) {
        if self.feed_needs_height_hints.replace(false) {
            // GPUI splices omit size hints; repair once per interaction, rather than per append.
            self.feed_list
                .clone()
                .with_uniform_item_height(px(FEED_ROW_HEIGHT_HINT * self.settings(cx).app_zoom));
            self.feed_scrollbar_metrics
                .set(self.measure_feed_scrollbar());
        }
    }

    pub(super) fn measure_feed_scrollbar(&self) -> ScrollbarMetrics {
        let viewport = self.feed_list.viewport_bounds();
        let position = -self.feed_list.scroll_px_offset_for_scrollbar().y;
        let content = if !self.feed_list.is_scrollbar_dragging() {
            self.feed_list
                .bounds_for_item(self.feed_list.item_count().saturating_sub(1))
                .map(|bounds| {
                    bounds.bottom() - viewport.top() + position + self.feed_vertical_padding
                })
        } else {
            None
        }
        .unwrap_or(
            viewport.size.height
                + self.feed_list.max_offset_for_scrollbar().y
                + self.feed_vertical_padding,
        );
        ScrollbarMetrics {
            viewport: f32::from(viewport.size.height),
            content: f32::from(content),
            position: f32::from(position)
                .clamp(0., f32::from((content - viewport.size.height).max(px(0.)))),
        }
    }

    fn end_feed_scrollbar_drag(&mut self, cx: &mut Context<Self>) {
        if self.feed_list.is_scrollbar_dragging() {
            self.feed_list.scrollbar_drag_ended();
            cx.notify();
        }
    }

    pub(super) fn feed_item_count(&self) -> usize {
        self.feed_rows.len().max(1)
            + usize::from(
                self.mission_composer_roster().is_empty()
                    && !self.loading
                    && mission_slot_actions_available(
                        self.mission_view().as_ref().map(|mission| mission.status),
                        self.archived(),
                        self.secondary,
                    ),
            )
    }

    fn render_empty_mission(&self, _cx: &mut Context<Self>) -> AnyElement {
        let shell = self.shell.clone();
        let mission = self.mission.clone();
        let events = self.events.clone();
        let description = crate::i18n::text("ui.roles_join_one_at_a_time_each_as_a_worker_every_role_gets_the_goal_give_each_one_its_own_t");
        div()
            .debug_selector(|| "MISSION_NO_ROLES".into())
            .flex_1()
            .min_h(rems(220. / 16.))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                svg()
                    .path("user-plus.svg")
                    .size(rems(28. / 16.))
                    .text_color(theme::faint()),
            )
            .child(
                div()
                    .text_size(theme::text_heading())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child(crate::i18n::text("ui.no_roles_yet")),
            )
            .child(
                div()
                    .max_w(rems(360. / 16.))
                    .text_center()
                    .text_size(theme::text_ui())
                    .line_height(rems(20. / 16.))
                    .text_color(theme::muted())
                    .child(description),
            )
            .when(self.remote.is_none(), |panel| {
                panel.child(
                    Button::new("empty-mission-add-role", crate::i18n::text("ui.add_role"))
                        .variant(ButtonVariant::Primary)
                        .icon("plus.svg")
                        .on_press(move |window, cx| {
                            if let Some(mission) = mission.clone() {
                                let _ = shell.update(cx, |this, cx| {
                                    this.open_add_role_modal(
                                        mission,
                                        Vec::new(),
                                        events.clone(),
                                        window,
                                        cx,
                                    )
                                });
                            }
                        }),
                )
            })
            .into_any_element()
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
                window.focus(&self.root_focus, cx);
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

    pub(super) fn cached_feed_markdown(&self, event_id: &str, text: &str) -> Rc<FeedMarkdown> {
        self.feed_markdown
            .borrow_mut()
            .entry(event_id.to_owned())
            .or_insert_with(|| Rc::new(FeedMarkdown::new(text)))
            .clone()
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
        let markdown = self.cached_feed_markdown(event_id, text);
        crate::surfaces::mission_markdown::render_markdown(
            event_id,
            &markdown.blocks,
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

    fn render_mission_feed_row(&self, row: FeedRow, cx: &mut Context<Self>) -> AnyElement {
        #[cfg(test)]
        self.feed_block_builds.set(self.feed_block_builds.get() + 1);
        match row {
            FeedRow::Block(group) => {
                self.render_mission_feed_block(self.feed_blocks[group].clone(), cx)
            }
            FeedRow::Message { group, event, part } => {
                let FeedBlock::MessageGroup { author, events } = &self.feed_blocks[group] else {
                    unreachable!()
                };
                self.render_mission_message_row(
                    author.clone(),
                    &events[0],
                    &events[event],
                    part,
                    event == 0 && part == 0,
                    cx,
                )
            }
        }
    }

    fn render_mission_feed_block(&self, block: FeedBlock, cx: &mut Context<Self>) -> AnyElement {
        let id = block.id().to_owned();
        match block {
            FeedBlock::PendingAsk(question) => self.render_question(question, cx),
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
                        .when(
                            matches!(
                                event.signal_type.as_ref().map(|kind| kind.as_str()),
                                Some("role_joined" | "role_removed")
                            ),
                            |label| {
                                label.flex().items_center().gap_2().child(
                                    svg()
                                        .path(
                                            if event.signal_type.as_ref().map(|kind| kind.as_str())
                                                == Some("role_joined")
                                            {
                                                "log-in.svg"
                                            } else {
                                                "log-out.svg"
                                            },
                                        )
                                        .size(rems(12. / 16.))
                                        .text_color(theme::faint()),
                                )
                            },
                        )
                        .child(mission_divider_label(&event)),
                )
                .child(
                    div()
                        .h(rems(1. / 16.))
                        .min_w(px(0.))
                        .flex_1()
                        .bg(theme::border()),
                )
                .into_any_element(),
            FeedBlock::MessageGroup { .. } => unreachable!(),
            FeedBlock::Signal(event) => self.render_mission_signal_row(event),
            FeedBlock::AskCard(event) => self.render_mission_ask_card(event, cx),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_mission_message_row(
        &self,
        author: String,
        first: &Event,
        event: &Event,
        part: usize,
        header: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let markdown = self.feed_markdown.borrow()[&event.id].clone();
        let human = author == "human";
        let system = author == "runner";
        let author = if system { "system".to_owned() } else { author };
        let goal = first.kind == EventKind::Signal
            && first.signal_type.as_ref().map(|kind| kind.as_str()) == Some("mission_goal");
        let target = if goal {
            self.roster
                .iter()
                .find(|member| member.slot.lead)
                .map(|member| member.slot.slot_handle.clone())
        } else {
            message_target(first, &self.askers_by_question)
        };
        div()
            .id(SharedString::from(if header {
                format!("mission-message-group-{}", first.id)
            } else {
                format!("mission-message-part-{}-{part}", event.id)
            }))
            .when(header, |row| {
                row.debug_selector({
                    let id = first.id.clone();
                    move || format!("MISSION_MESSAGE_GROUP {id}")
                })
            })
            .px_4()
            .flex()
            .items_start()
            .gap_3()
            .child(if header {
                RoleAvatar::new(author.clone(), 35.).into_any_element()
            } else {
                div().w(rems(35. / 16.)).flex_none().into_any_element()
            })
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .when(cfg!(test), |body| {
                        let id = event.id.clone();
                        body.debug_selector(move || format!("MISSION_MESSAGE_BODY {id} {part}"))
                    })
                    .children(header.then(|| {
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
                                        crate::i18n::text("word.you").into()
                                    } else if system {
                                        crate::i18n::text("word.system").into()
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
                                    .child(crate::i18n::text("heading.goal"))
                            }))
                            .children(target.map(|target| {
                                div()
                                    .debug_selector(|| format!("MISSION_MESSAGE_TARGET {target}"))
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::muted())
                                    .child(format!("→ @{target}"))
                            }))
                            .child(div().flex_none().child(format_event_time(first)))
                    }))
                    .child(
                        div()
                            .when(header, |body| body.mt_1())
                            .when(!header && part == 0, |body| body.pt_1())
                            .flex()
                            .flex_col()
                            .gap_1()
                            .text_size(theme::text_body())
                            .text_color(if system {
                                theme::muted()
                            } else {
                                theme::text()
                            })
                            .children({
                                if markdown.blocks.is_empty() {
                                    goal.then(|| {
                                        div()
                                            .text_color(theme::faint())
                                            .child(crate::i18n::text("ui.no_text"))
                                            .into_any_element()
                                    })
                                } else {
                                    let selection_color = crate::terminal::element::to_hsla(
                                        app_settings::terminal_palette(
                                            self.settings(cx),
                                            theme::active_variant(),
                                        )
                                        .selection,
                                        1.,
                                    );
                                    Some(markdown.render_row(
                                        part,
                                        &event.id,
                                        cx.entity_id(),
                                        self.feed_selection.as_ref(),
                                        selection_color,
                                        &self.feed_selection_handler(cx),
                                        cx,
                                    ))
                                }
                            }),
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
        let (author_seed, author_name) = mission_signal_author(&event);
        div()
            .id(SharedString::from(format!("mission-signal-{event_id}")))
            .px_4()
            .flex()
            .items_start()
            .gap_3()
            .child(RoleAvatar::new(author_seed.to_owned(), 35.))
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
                                    .text_color(runner_app::ui::hue_for_seed(author_seed).color())
                                    .child(author_name),
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
                                        crate::i18n::tf(
                                            "format.warning",
                                            &[("time", (format_event_time(&event)).to_string())],
                                        )
                                    } else {
                                        crate::i18n::tf(
                                            "format.signal",
                                            &[
                                                ("signal", (signal).to_string()),
                                                (
                                                    "recipient",
                                                    if event.recipients().is_empty() {
                                                        String::new()
                                                    } else {
                                                        format!(
                                                            " → @{}",
                                                            event.recipients().join(", @")
                                                        )
                                                    },
                                                ),
                                                ("time", (format_event_time(&event)).to_string()),
                                            ],
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
                                theme::panel()
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
        let question_id = event
            .payload
            .get("question_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&event.id)
            .to_owned();
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
            Some(handle) if handle != asker => crate::i18n::tf(
                "format.you",
                &[
                    ("handle", (handle).to_string()),
                    ("asker", (asker).to_string()),
                ],
            ),
            _ => crate::i18n::text("ui.you").to_owned(),
        };
        self.render_question(
            crate::surfaces::front_page::model::MissionQuestionDisplay {
                id: question_id,
                markdown_id: event.id.clone(),
                asker,
                prompt,
                choices,
                chain,
                time: Some(format_event_time(&event)),
            },
            cx,
        )
    }

    pub(super) fn render_question(
        &self,
        question: crate::surfaces::front_page::model::MissionQuestionDisplay,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let crate::surfaces::front_page::model::MissionQuestionDisplay {
            id: question_id,
            markdown_id,
            asker,
            prompt,
            mut choices,
            chain,
            time,
        } = question;
        if !self.remote_supports(&["answer"]) {
            choices.clear();
        }
        let available = self.remote.is_none()
            || self
                .remote_summary()
                .is_some_and(|mission| mission.asks.iter().any(|ask| ask.question == question_id));
        let resolved = self.resolved_asks.get(&question_id).cloned();
        let pending_choice = self.pending_ask_choices.get(&question_id).cloned();
        let submitting = self.submitting_asks.contains(&question_id) || !available;
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
                                    .child(crate::i18n::text("ui.needs_your_input")),
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
                                    .children(time),
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
                                    .child(crate::i18n::text("ui.no_prompt"))
                                    .into_any_element()
                            } else {
                                self.render_feed_markdown(&markdown_id, &prompt, cx)
                            })
                            .child(buttons)
                            .children(resolved.map(|choice| {
                                div()
                                    .mt_1()
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(crate::i18n::tf(
                                        "format.answered",
                                        &[("choice", (choice).to_string())],
                                    ))
                            })),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn answer_mission_question(
        &mut self,
        question_id: String,
        choice: String,
        cx: &mut Context<Self>,
    ) {
        if self.remote.is_some() {
            self.remote_answer(question_id, choice, cx);
            return;
        }
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
        self.invalidate_feed_ask(&question_id);
        cx.notify();
        let core = self.core(cx).clone();
        let post_question = question_id.clone();
        let task = cx.background_spawn(async move {
            core.mission_signal_impl(runner_core::protocol::mission::PostSignalInput {
                mission_id,
                from: None,
                signal_type: "human_response".into(),
                payload: serde_json::json!({
                    "question_id": post_question,
                    "choice": choice,
                }),
            })
            .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.submitting_asks.remove(&question_id);
                this.invalidate_feed_ask(&question_id);
                if let Err(error) = result {
                    this.pending_ask_choices.remove(&question_id);
                    this.error = Some(action_failure(crate::i18n::message("action.answer"), error));
                }
                cx.notify();
            });
        })
        .detach();
    }
}
