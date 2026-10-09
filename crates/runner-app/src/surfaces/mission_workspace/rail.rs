use std::process::Command;
use std::rc::Rc;

use chrono::Utc;
use gpui::prelude::*;
#[cfg(windows)]
use gpui::MouseButton;
use gpui::{
    div, px, rems, svg, AnyElement, App, BoxShadow, CursorStyle, FontWeight, KeyDownEvent,
    SharedString, Window,
};
use runner_app::ui::{
    Button, ButtonVariant, Field, IconButton, Modal, OverlayWidth, RoleAvatar, SessionControl,
    SessionControlVariant, Tooltip,
};

use super::*;
use crate::*;

impl MissionWorkspace {
    pub(super) fn render_mission_rail(
        &self,
        visibility: f32,
        show_rail: bool,
        border_on: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let width = self.settings(cx).mission_rail_width;
        let visible_width = width * visibility;
        if !show_rail {
            return div()
                .id("mission-rail")
                .debug_selector(|| "MISSION_RAIL".into())
                .relative()
                .w(rems(visible_width / 16.))
                .h_full()
                .flex_none()
                .overflow_hidden()
                .into_any_element();
        }
        let root = cx.entity();
        let roles_root = root.clone();
        let meta_root = root.clone();
        let collapse_root = root;
        let rail_view = self.rail_view;
        let header = div()
            .h(rems(WORKSPACE_HEADER_HEIGHT / 16.))
            .flex_none()
            .px_4()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(theme::border())
            .rounded_tr(rems(11. / 16.))
            .map(|element| {
                #[cfg(test)]
                let element = crate::theme_snapshot::record_fill("MISSION_RAIL_HEADER", element);
                element
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        rail_view_button(
                            "mission-rail-roles",
                            "users.svg",
                            rail_view == MissionRailView::Roles,
                        )
                        .on_click(move |_, _, cx| {
                            roles_root.update(cx, |this, cx| {
                                this.set_mission_rail_view(MissionRailView::Roles, cx)
                            });
                        }),
                    )
                    .child(
                        rail_view_button(
                            "mission-rail-meta",
                            "info.svg",
                            rail_view == MissionRailView::Meta,
                        )
                        .on_click(move |_, _, cx| {
                            meta_root.update(cx, |this, cx| {
                                this.set_mission_rail_view(MissionRailView::Meta, cx)
                            });
                        }),
                    ),
            )
            .child(
                div().ml_auto().child(
                    IconButton::new("collapse-mission-rail", "panel-right-open.svg")
                        .tooltip("Collapse sessions panel")
                        .on_press(move |_, cx| {
                            collapse_root.update(cx, |this, cx| {
                                this.update_app_settings(cx, true, |settings| {
                                    settings.mission_rail_open = false;
                                    true
                                });
                                cx.notify();
                            });
                        }),
                ),
            );
        let body = match rail_view {
            MissionRailView::Roles => self.render_roles_rail(window, cx),
            MissionRailView::Meta => self.render_mission_meta_panel(cx),
        };
        let drag = MissionRailResizeDrag;
        let rail = div()
            .relative()
            .w(rems(width / 16.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .pb(rems(8. / 16.))
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.))
                            .flex()
                            .flex_col()
                            .bg(theme::panel())
                            .map(|element| {
                                #[cfg(test)]
                                let element =
                                    crate::theme_snapshot::record_fill("MISSION_PANEL", element);
                                element
                            })
                            .child(body),
                    ),
            )
            .child(
                div()
                    .id("mission-rail-resize")
                    .map(|handle| {
                        #[cfg(windows)]
                        let handle = handle.occlude();
                        handle
                    })
                    .absolute()
                    .left_0()
                    .top_0()
                    .h_full()
                    .w(rems(4. / 16.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .hover(|strip| strip.bg(theme::with_alpha(theme::accent(), 0.4)))
                    .on_drag(drag, |drag: &MissionRailResizeDrag, _, _, cx: &mut App| {
                        cx.new(|_| drag.clone())
                    }),
            );
        div()
            .id("mission-rail")
            .debug_selector(|| "MISSION_RAIL".into())
            .relative()
            .w(rems(visible_width / 16.))
            .h_full()
            .flex_none()
            .overflow_hidden()
            .when(border_on, |rail| {
                rail.border_l_1().border_color(theme::border())
            })
            .child(rail)
            .into_any_element()
    }

    fn set_mission_rail_view(&mut self, view: MissionRailView, cx: &mut Context<Self>) {
        self.rail_view = view;
        cx.notify();
    }

    fn render_roles_rail(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let selected = match &self.active_tab {
            MissionTab::Session(session_id) => Some(session_id.as_str()),
            MissionTab::Feed => None,
        };
        let lead_handle = self
            .sessions
            .iter()
            .find(|session| session.lead)
            .or_else(|| self.sessions.first())
            .map(|session| session.handle.as_str())
            .unwrap_or_default()
            .to_owned();
        let mut list = div()
            .id("mission-roles-scroll")
            .debug_selector(|| "MISSION_ROLES_SCROLL".into())
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .px_5()
            .pb_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(rail_section_label("Sessions"));
        if self.sessions.is_empty() {
            return list
                .child(
                    div()
                        .text_size(theme::text_ui())
                        .text_color(theme::faint())
                        .child("No sessions yet."),
                )
                .into_any_element();
        }
        let root = cx.entity();
        for session in &self.sessions {
            let session_id = session.session.id.clone();
            let open_id = session_id.clone();
            let open_root = root.clone();
            let card_key_id = session_id.clone();
            let card_key_root = root.clone();
            let status = runner_app::ui::agent_status::StatusPresentation::new(
                &self.slot_agent_status(&session_id, cx),
            );
            let subtitle = runner_app::ui::agent_status::status_indicator(
                status,
                true,
                SharedString::from(format!("mission-card-status-{session_id}")),
                window,
                cx,
            );
            let disabled = self.stopping
                || self.resuming
                || self.archiving
                || self.slot_actions.contains(&session_id)
                || self.transitions.contains_key(&session_id);
            let controls = mission_slot_actions_available(
                self.mission.as_ref().map(|mission| mission.status),
                self.archived(),
                self.secondary,
            )
            .then(|| {
                div()
                    .debug_selector(|| format!("MISSION_CARD_CONTROLS {session_id}"))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .children(
                        slot_controls(session.session.status)
                            .into_iter()
                            .map(|action| {
                                let action_root = root.clone();
                                let target = session_id.clone();
                                SessionControl::new(
                                    SharedString::from(format!("slot-{action:?}-{session_id}")),
                                    action,
                                )
                                .variant(SessionControlVariant::Header)
                                .title(slot_control_title(
                                    action,
                                    &self.settings(cx).keymap_overrides,
                                ))
                                .header_size(24.)
                                .restarting(
                                    self.transition_kind(&session_id)
                                        == Some(MissionTransitionKind::Restarting),
                                )
                                .lifecycle_disabled(disabled)
                                .on_press(move |window, cx| {
                                    action_root.update(cx, |this, cx| {
                                        this.request_slot_action(&target, action, window, cx)
                                    })
                                })
                            }),
                    )
            });
            let handle_title = SharedString::from(format!("@{}", session.handle));
            let mut header_gap = rems(8. / 16.);
            let mut header_margin = px(0.);
            if controls.is_some() && session.handle == lead_handle {
                let mut badge_style = window.text_style();
                badge_style.font_weight = FontWeight::BOLD;
                let badge_width = window
                    .text_system()
                    .shape_line(
                        "LEAD".into(),
                        theme::text_micro().to_pixels(window.rem_size()),
                        &[badge_style.to_run("LEAD".len())],
                        None,
                    )
                    .width
                    .ceil();
                let mut handle_style = window.text_style();
                handle_style.font_family = theme::UI_MONOSPACE_FONT.into();
                handle_style.font_weight = FontWeight::SEMIBOLD;
                let prefix =
                    SharedString::from(format!("@{}…", session.handle.chars().next().unwrap()));
                let prefix_width = window
                    .text_system()
                    .shape_line(
                        prefix.clone(),
                        theme::text_body().to_pixels(window.rem_size()),
                        &[handle_style.to_run(prefix.len())],
                        None,
                    )
                    .width
                    .ceil();
                let full_width = window
                    .text_system()
                    .shape_line(
                        handle_title.clone(),
                        theme::text_body().to_pixels(window.rem_size()),
                        &[handle_style.to_run(handle_title.len())],
                        None,
                    )
                    .width
                    .ceil();
                let needed_width = full_width.min(prefix_width);
                let fixed_width = rems((40. + 24. + 25. + 52. + 12. + 24.) / 16.)
                    .to_pixels(window.rem_size())
                    + px(2.)
                    + badge_width;
                let rail_width =
                    rems(self.settings(cx).mission_rail_width / 16.).to_pixels(window.rem_size());
                let available_width = rail_width - fixed_width;
                if available_width < needed_width {
                    header_gap = rems(2. / 16.);
                    let compact_width =
                        available_width + rems(18. / 16.).to_pixels(window.rem_size());
                    if compact_width < needed_width {
                        // Borrow only the header's padding when even compact gaps cannot fit @<character>….
                        header_margin = -((needed_width - compact_width) / 2.).ceil();
                    }
                }
            }
            let copy = self.session_key_copies.get(&session_id).cloned();
            let key_value = div()
                .debug_selector(|| format!("MISSION_CARD_KEY {session_id}"))
                .min_w(px(0.))
                .flex_1()
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_color(theme::muted());
            let key_value = match &session.agent_session_key {
                Some(key) => Tooltip::new(
                    SharedString::from(format!("mission-session-key-{session_id}")),
                    key.clone(),
                    key_value.line_clamp(2).text_ellipsis().child(key.clone()),
                )
                .expand()
                .into_any_element(),
                None => key_value.child("NULL").into_any_element(),
            };
            let active = selected == Some(session_id.as_str());
            list = list.child(
                div()
                    .debug_selector(|| format!("MISSION_CARD {session_id}"))
                    .id(SharedString::from(format!(
                        "mission-role-card-{session_id}"
                    )))
                    .w_full()
                    .tab_index(0)
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap(rems(6. / 16.))
                    .rounded_md()
                    .border_1()
                    .border_color(if active {
                        theme::with_alpha(theme::accent(), 0.6)
                    } else {
                        theme::border()
                    })
                    .bg(theme::bg())
                    .cursor_pointer()
                    .when(!active, |card| {
                        card.hover(|card| card.border_color(theme::border_strong()))
                    })
                    .focus_visible(|card| {
                        card.border_color(theme::accent()).shadow(vec![BoxShadow {
                            color: theme::with_alpha(theme::accent(), 0.5),
                            offset: gpui::point(px(0.), px(0.)),
                            blur_radius: px(0.),
                            spread_radius: px(1.),
                            inset: false,
                        }])
                    })
                    .on_click(move |_, window, cx| {
                        open_root.update(cx, |this, cx| {
                            this.select_mission_session(&open_id, window, cx)
                        });
                    })
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            card_key_root.update(cx, |this, cx| {
                                this.select_mission_session(&card_key_id, window, cx)
                            });
                        }
                    })
                    .child(
                        div()
                            .mx(header_margin)
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(header_gap)
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .flex_initial()
                                    .flex()
                                    .items_center()
                                    .gap(header_gap)
                                    .child(
                                        div()
                                            .debug_selector(|| {
                                                format!("MISSION_CARD_AVATAR {session_id}")
                                            })
                                            .flex_none()
                                            .child(RoleAvatar::new(session.handle.clone(), 25.)),
                                    )
                                    .child(
                                        Tooltip::new(
                                            SharedString::from(format!(
                                                "mission-role-title-{session_id}"
                                            )),
                                            self.session_title_tooltip(session, cx),
                                            div()
                                                .debug_selector(|| {
                                                    format!("MISSION_CARD_HANDLE {session_id}")
                                                })
                                                .min_w(px(0.))
                                                .flex_1()
                                                .truncate()
                                                .font_family(theme::UI_MONOSPACE_FONT)
                                                .text_size(theme::text_body())
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(
                                                    runner_app::ui::hue_for_seed(&session.handle)
                                                        .color(),
                                                )
                                                .child(handle_title),
                                        )
                                        .expand(),
                                    )
                                    .children((session.handle == lead_handle).then(|| {
                                        div()
                                            .debug_selector(|| {
                                                format!("MISSION_CARD_LEAD {session_id}")
                                            })
                                            .flex_none()
                                            .child(runner_app::ui::lead_badge())
                                    })),
                            )
                            .children(controls),
                    )
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .when(
                                !runner_core::protocol::model::Runtime::parse(&session.runtime)
                                    .is_some_and(runner_core::protocol::model::Runtime::is_shell),
                                |card| card.child(subtitle),
                            ),
                    )
                    .child(
                        div()
                            .pt_2()
                            .flex()
                            .gap_2()
                            .border_t_1()
                            .border_color(theme::with_alpha(theme::border(), 0.7))
                            .text_size(theme::text_caption())
                            .line_height(rems(14. / 16.))
                            .child(
                                div()
                                    .debug_selector(|| {
                                        format!("MISSION_CARD_KEY_LABEL {session_id}")
                                    })
                                    .w(rems(72. / 16.))
                                    .flex_none()
                                    .text_color(theme::faint())
                                    .child("session_key"),
                            )
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .flex_1()
                                    .flex()
                                    .items_start()
                                    .gap(rems(6. / 16.))
                                    .child(key_value)
                                    .children(copy.map(|copy| {
                                        div()
                                            .debug_selector(|| {
                                                format!("MISSION_CARD_KEY_COPY {session_id}")
                                            })
                                            .flex_none()
                                            .child(copy)
                                    })),
                            ),
                    ),
            );
        }
        list.into_any_element()
    }

    fn reveal_mission_cwd(&mut self, cwd: String, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move {
            #[cfg(target_os = "macos")]
            {
                let status = Command::new("open")
                    .arg("-R")
                    .arg(cwd)
                    .status()
                    .map_err(|error| error.to_string())?;
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("Finder exited with status {status}"))
                }
            }
            #[cfg(windows)]
            {
                Command::new("explorer.exe")
                    .arg(format!("/select,{cwd}"))
                    .spawn()
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.error = Some(format!("Couldn't reveal the working directory: {error}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn render_mission_meta_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(mission) = self.mission.clone() else {
            return div().into_any_element();
        };
        let root = cx.entity();
        let crew_root = root.clone();
        let cwd_root = root;
        let goal = self.goal();
        let permission_mode = self.permission_mode();
        let mut panel = div()
            .id("mission-meta-scroll")
            .debug_selector(|| "MISSION_META_SCROLL".into())
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .px_5()
            .pb_5()
            .flex()
            .flex_col()
            .gap_4()
            .child(rail_section_label("Mission detail"))
            .child(meta_section(
                "Mission ID",
                div()
                    .flex()
                    .items_start()
                    .gap_1()
                    .child(
                        div()
                            .debug_selector(|| "MISSION_META_ID".into())
                            .min_w(px(0.))
                            .flex_1()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .child(mission.id.clone()),
                    )
                    .child(self.mission_id_copy.clone()),
            ))
            .child(meta_section(
                "Goal",
                match goal {
                    Some(goal) if !goal.is_empty() => div()
                        .text_size(theme::text_ui())
                        .line_height(rems(18. / 16.))
                        .text_color(theme::text())
                        .child(goal)
                        .into_any_element(),
                    Some(_) => div()
                        .text_size(theme::text_ui())
                        .italic()
                        .text_color(theme::faint())
                        .child("No goal set.")
                        .into_any_element(),
                    None => div()
                        .text_size(theme::text_ui())
                        .italic()
                        .text_color(theme::faint())
                        .child("Loading…")
                        .into_any_element(),
                },
            ));
        panel = panel.child(meta_section(
            "Working dir",
            mission.cwd.clone().map_or_else(
                || {
                    div()
                        .text_size(theme::text_ui())
                        .italic()
                        .text_color(theme::faint())
                        .child("No cwd set.")
                        .into_any_element()
                },
                |cwd| {
                    let reveal = cwd.clone();
                    #[cfg(target_os = "macos")]
                    let reveal_label = "Reveal in Finder";
                    #[cfg(windows)]
                    let reveal_label = "Reveal in Explorer";
                    Tooltip::new(
                        "reveal-mission-cwd-tooltip",
                        reveal_label,
                        div()
                            .id("reveal-mission-cwd")
                            .w_full()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::bg())
                            .px_2()
                            .py_2()
                            .cursor_pointer()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_meta())
                            .text_color(theme::text())
                            .hover(|button| button.border_color(theme::border_strong()))
                            .on_click(move |_, _, cx| {
                                let reveal = reveal.clone();
                                cwd_root.update(cx, |this, cx| this.reveal_mission_cwd(reveal, cx));
                            })
                            .child(cwd),
                    )
                    .expand()
                    .into_any_element()
                },
            ),
        ));
        let crew_name = self
            .crew
            .as_ref()
            .map(|crew| crew.name.clone())
            .unwrap_or_else(|| "…".into());
        let crew_id = mission.crew_id.clone();
        panel
            .child(meta_section(
                "Crew",
                div()
                    .id("open-mission-crew")
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .text_size(theme::text_ui())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::accent())
                    .hover(|link| link.underline())
                    .on_click(move |_, window, cx| {
                        crew_root.update(cx, |this, cx| {
                            this.open_crew_editor(crew_id.clone(), window, cx)
                        });
                    })
                    .child(
                        svg()
                            .flex_none()
                            .path("users.svg")
                            .size(rems(12. / 16.))
                            .text_color(theme::muted()),
                    )
                    .child(crew_name),
            ))
            .children(permission_mode.map(|mode| {
                meta_section(
                    "Permissions",
                    div()
                        .id("mission-permission-mode")
                        .text_size(theme::text_ui())
                        .text_color(theme::text())
                        .child(mode),
                )
            }))
            .child(meta_section(
                "Started",
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(theme::text_ui())
                    .child(
                        svg()
                            .flex_none()
                            .path("clock.svg")
                            .size(rems(12. / 16.))
                            .text_color(theme::muted()),
                    )
                    .child(format_relative_time(mission.started_at)),
            ))
            .child(
                div()
                    .debug_selector(|| "MISSION_META_END".into())
                    .h(rems(1. / 16.))
                    .w_full()
                    .bg(theme::border()),
            )
            .into_any_element()
    }

    pub(crate) fn render_mission_rename_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let modal = self.rename_modal.as_ref().expect("mission rename modal");
        let submitting = modal.submitting;
        let valid = !modal.input.read(cx).text().trim().is_empty();
        let root = cx.entity();
        let close_root = root.clone();
        let cancel_root = root.clone();
        let submit_root = root.clone();
        let dismiss_root = root;
        let title = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(theme::text_heading())
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Rename mission"),
            )
            .child(
                IconButton::new("close-mission-rename", "close.svg")
                    .focus_handle(modal.close_focus.clone())
                    .tooltip("Close rename")
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_mission_rename(window, cx));
                    }),
            );
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .on_key_down(cx.listener(Self::on_mission_rename_key_down))
            .children(
                modal
                    .error
                    .clone()
                    .filter(|_| !self.app_store.read(cx).daemon_disconnected)
                    .map(|error| {
                        runner_app::ui::notice_banner(error, runner_app::ui::Tone::Danger)
                    }),
            )
            .child(
                Field::new("mission-rename-name", "Name", modal.input.clone())
                    .focus_target(modal.input.read(cx).focus_handle())
                    .emphasized(true),
            );
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cancel-mission-rename", "Cancel")
                    .focus_handle(modal.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_mission_rename(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "submit-mission-rename",
                    if submitting { "Saving…" } else { "Save" },
                )
                .focus_handle(modal.submit_focus.clone())
                .variant(ButtonVariant::Primary)
                .disabled(submitting || !valid)
                .on_press(move |window, cx| {
                    submit_root.update(cx, |this, cx| this.submit_mission_rename(window, cx));
                }),
            );
        Modal::new(
            title,
            body,
            Rc::new(move |window, cx| {
                dismiss_root.update(cx, |this, cx| this.close_mission_rename(window, cx));
            }),
        )
        .width(OverlayWidth::Custom(448.))
        .busy(submitting)
        .focus_order(if submitting {
            Vec::new()
        } else {
            vec![
                modal.input.read(cx).focus_handle(),
                modal.cancel_focus.clone(),
                modal.submit_focus.clone(),
                modal.close_focus.clone(),
            ]
        })
        .footer(footer)
        .into_any_element()
    }
}

fn rail_view_button(
    id: impl Into<gpui::ElementId>,
    icon: &'static str,
    active: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .map(|button| {
            #[cfg(windows)]
            let button = button.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
            button
        })
        .group("mission-rail-view-button")
        .size(rems(28. / 16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .cursor_pointer()
        .text_color(if active {
            theme::text()
        } else {
            theme::muted()
        })
        .when(active, |button| {
            button.bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
        })
        .hover(|button| {
            button
                .bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
                .text_color(theme::text())
        })
        .child(
            svg()
                .flex_none()
                .path(icon)
                .size(rems(14. / 16.))
                .text_color(if active {
                    theme::text()
                } else {
                    theme::muted()
                })
                .group_hover("mission-rail-view-button", |icon| {
                    icon.text_color(theme::text())
                }),
        )
}

fn rail_section_label(label: &'static str) -> AnyElement {
    div()
        .pt_5()
        .text_size(theme::text_caption())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::faint())
        .child(tracked_uppercase(label))
        .into_any_element()
}

fn meta_section(label: &'static str, body: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(theme::text_caption())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::faint())
                .child(tracked_uppercase(label)),
        )
        .child(body)
        .into_any_element()
}

fn tracked_uppercase(label: &str) -> String {
    label
        .to_uppercase()
        .chars()
        .map(|character| character.to_string())
        .collect::<Vec<_>>()
        .join("\u{2009}")
}

fn format_relative_time(started_at: chrono::DateTime<Utc>) -> String {
    let minutes = Utc::now()
        .signed_duration_since(started_at)
        .num_minutes()
        .max(0);
    if minutes < 1 {
        "just now".into()
    } else if minutes < 60 {
        format!(
            "{minutes} minute{} ago",
            if minutes == 1 { "" } else { "s" }
        )
    } else {
        let hours = minutes / 60;
        if hours < 24 {
            format!("{hours} hour{} ago", if hours == 1 { "" } else { "s" })
        } else {
            let days = hours / 24;
            format!("{days} day{} ago", if days == 1 { "" } else { "s" })
        }
    }
}
