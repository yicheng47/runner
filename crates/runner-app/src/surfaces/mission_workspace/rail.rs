#[cfg(not(windows))]
use std::process::Command;
use std::rc::Rc;

use chrono::Utc;
use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, BoxShadow, FontWeight, KeyDownEvent, SharedString, Window,
};
use runner_app::ui::{
    Button, ButtonVariant, Field, IconButton, Modal, OverlayWidth, RoleAvatar, SessionControl,
    SessionControlVariant, Tooltip,
};

use super::*;
use crate::file_links::FileLinkRoute;
use crate::surfaces::files::FilesView;
use crate::surfaces::side_panel::{self, PanelContext, PanelSubject, PanelSurface};
use crate::*;

impl MissionWorkspace {
    pub(super) fn rail_is_open(&self, cx: &App) -> bool {
        side_panel::resolved_open(
            PanelSurface::Mission,
            self.rail_open,
            self.rail_files.available_width(),
            self.settings(cx).mission_rail_width,
            1,
        )
    }

    pub(super) fn set_rail_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.rail_open = Some(open);
        if !open {
            self.rail_maximized = false;
        }
        cx.notify();
    }

    pub(crate) fn rail_contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.rail_focus.contains_focused(window, cx)
    }

    pub(crate) fn focus_rail_shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.settings(cx);
        let shown = side_panel::shown(
            PanelSurface::Mission,
            settings,
            &self.rail_surfaces.selection,
        );
        let subject = self
            .mission_view()
            .map(|mission| PanelSubject::Mission(mission.id));
        self.rail_files
            .focus_shown(subject.as_ref(), shown, &self.rail_focus, window, cx);
    }

    pub(super) fn rail_effectively_maximized(&self, cx: &App) -> bool {
        !self.presentation.is_remote()
            && self.settings(cx).show_side_panel
            && self.rail_is_open(cx)
            && self.rail_maximized
    }

    pub(crate) fn set_rail_maximized(
        &mut self,
        maximized: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let allowed = !self.presentation.is_remote()
            && self.settings(cx).show_side_panel
            && self.rail_is_open(cx);
        let next = maximized && allowed;
        if self.rail_maximized == next {
            return;
        }
        self.rail_maximized = next;
        if next {
            self.rail_visibility = SidebarVisibilityTransition::new(true);
            if !self.rail_contains_focus(window, cx) {
                self.focus_rail_shown(window, cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn toggle_rail_maximized(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = !self.rail_maximized;
        self.set_rail_maximized(next, window, cx);
    }

    pub(crate) fn toggle_maximize_rail_if_focused(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.rail_maximized || self.rail_contains_focus(window, cx) {
            self.toggle_rail_maximized(window, cx);
        }
    }

    pub(super) fn toggle_maximize_rail_action(
        &mut self,
        _: &crate::ToggleMaximizePane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.rail_maximized || self.rail_contains_focus(window, cx) {
            cx.stop_propagation();
            self.toggle_rail_maximized(window, cx);
        }
    }

    /// The rail's width, widened state, and Docked vs Sheet presentation.
    pub(super) fn rail_layout(&self, cx: &App) -> side_panel::PanelLayout {
        let settings = self.settings(cx);
        let subject = self
            .mission_view()
            .map(|mission| PanelSubject::Mission(mission.id));
        self.rail_files.panel_layout(
            PanelSurface::Mission,
            settings,
            &self.rail_surfaces.selection,
            subject.as_ref(),
            settings.mission_rail_width,
            1,
            self.rail_maximized && !self.presentation.is_remote(),
            cx,
        )
    }

    fn rail_context(
        &self,
        mission: crate::surfaces::front_page::model::WorkspaceMission,
    ) -> PanelContext {
        PanelContext {
            subject: PanelSubject::Mission(mission.id),
            surface: PanelSurface::Mission,
            cwd: mission.cwd,
            project: mission.project_id,
            remote: self.presentation.is_remote(),
            following: None,
        }
    }

    /// Where a ⌘-clicked file link in a member's or the drawer's terminal
    /// opens: the rail's preview.
    pub(super) fn file_link_route(&self, cx: &Context<Self>) -> FileLinkRoute {
        let mission = self.mission_id.clone();
        FileLinkRoute::new(
            cx.weak_entity(),
            Self::link_files_view,
            Self::existing_link_files_view,
            move |this, cx| {
                if mission == this.mission_id {
                    this.show_link_preview(cx);
                } else if let Some(id) = mission.as_ref() {
                    let panel = this.rails_by_mission.entry(id.clone()).or_default();
                    panel.selection.select(PanelViewId::Files);
                    panel.open = Some(true);
                    cx.notify();
                }
            },
            |this: &Self, cx| crate::file_links::FileLinkRouteState {
                local: !this.presentation.is_remote(),
                panel_open: this.rail_is_open(cx),
            },
        )
    }

    /// The already-created Files view while its preview can take a link, read
    /// synchronously without creating a view or fetching its root.
    fn existing_link_files_view(&self, cx: &App) -> Option<Entity<FilesView>> {
        let settings = self.settings(cx);
        if !settings.show_side_panel || !settings.files_view || self.presentation.is_remote() {
            return None;
        }
        let mission = self.mission_view()?;
        self.rail_files.view(&PanelSubject::Mission(mission.id))
    }

    /// The rail's Files view while its preview can take a link: the panel
    /// and Files on, and the mission local.
    fn link_files_view(&mut self, cx: &mut Context<Self>) -> Option<Entity<FilesView>> {
        let settings = self.settings(cx);
        if !settings.show_side_panel || !settings.files_view || self.presentation.is_remote() {
            return None;
        }
        let context = self.rail_context(self.mission_view()?);
        Some(self.rail_files.link_view(&context, cx))
    }

    /// ⇧⌘O: opens the rail on Files, as a link does, and its finder. With no
    /// view to search the key goes on, so a terminal program gets it.
    pub(crate) fn search_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.link_files_view(cx) else {
            cx.propagate();
            return;
        };
        self.show_link_preview(cx);
        view.update(cx, |view, cx| view.toggle_finder(window, cx));
    }

    /// A link opened in the preview: the rail opens Files, or switches to it.
    fn show_link_preview(&mut self, cx: &mut Context<Self>) {
        self.rail_surfaces.selection.select(PanelViewId::Files);
        self.set_rail_open(true, cx);
        cx.notify();
    }

    pub(super) fn render_mission_rail(
        &self,
        rail_layout: side_panel::PanelLayout,
        animated_width: f32,
        visibility: f32,
        show_rail: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let width = if rail_layout.is_sheet() {
            rail_layout.width
        } else {
            animated_width
        };
        let widened = rail_layout.widened;
        let sheet = rail_layout.is_sheet();
        let maximized = show_rail && self.rail_maximized && !self.presentation.is_remote();
        let visible_width = if !show_rail {
            0.
        } else if sheet {
            width
        } else {
            width * visibility
        };
        if !show_rail {
            self.rail_files.hide(cx);
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
        let surface_root = cx.entity();
        let collapse_root = cx.entity();
        let maximize_root = cx.entity();
        let req_max_root = cx.entity();
        let settings = self.settings(cx).clone();
        let zoom = settings.app_zoom;
        let shown = side_panel::shown(
            PanelSurface::Mission,
            &settings,
            &self.rail_surfaces.selection,
        );
        let handler: side_panel::SurfaceHandler = Rc::new(move |action, _window, cx| {
            surface_root.update(cx, |this, cx| {
                this.rail_surfaces.apply(action);
                cx.notify();
            })
        });
        let maximize_control = (!self.presentation.is_remote() && (widened.is_some() || maximized))
            .then(|| side_panel::MaximizeControl {
                maximized,
                on_toggle: Rc::new(move |window, cx| {
                    maximize_root.update(cx, |this, cx| {
                        this.toggle_rail_maximized(window, cx);
                    });
                }),
            });
        let on_request_maximize: side_panel::PanelMaximizeCallback =
            Rc::new(move |maximize, window, cx| {
                req_max_root.update(cx, |this, cx| {
                    this.set_rail_maximized(maximize, window, cx);
                });
            });
        let rail_context = self
            .mission_view()
            .map(|mission| self.rail_context(mission));
        let has_viewer = !self.presentation.is_remote()
            && (settings.files_view || settings.changes_view)
            && rail_context
                .as_ref()
                .is_some_and(|ctx| self.rail_files.viewer_has_tabs(&ctx.subject, cx));
        let roles_rail_width = if has_viewer {
            widened.map_or(settings.file_tree_width, |w| w.tree_width)
        } else {
            width
        };
        let sessions_body = (!matches!(shown, Some(PanelViewId::Files | PanelViewId::Changes)))
            .then(|| {
                let roles = self.render_roles_rail(roles_rail_width, window, cx);
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            if !this.rail_contains_focus(window, cx) {
                                this.rail_focus.focus(window, cx);
                            }
                        }),
                    )
                    .map(|element| {
                        #[cfg(test)]
                        let element = crate::theme_snapshot::record_fill("MISSION_PANEL", element);
                        element
                    })
                    .child(roles)
                    .into_any_element()
            });
        let macos_titlebar_inset =
            (maximized && self.sidebar_collapsed && cfg!(target_os = "macos")).then(|| {
                (
                    self.workspace_titlebar_padding(window, cx),
                    self.render_collapsed_titlebar_spacer(),
                )
            });
        let titlebar_weak = cx.weak_entity();
        let wrap_titlebar: crate::surfaces::files::TitlebarWrapper = Rc::new(move |id, area, _| {
            let out_weak = titlebar_weak.clone();
            let up_weak = titlebar_weak.clone();
            let down_weak = titlebar_weak.clone();
            area.id(id)
                .window_control_area(gpui::WindowControlArea::Drag)
                .capture_any_mouse_down(|event, _, _| platform_ui::capture_titlebar_press(event))
                .on_mouse_down_out(move |_, _, cx| {
                    let _ = out_weak.update(cx, |this, _| this.titlebar_drag_armed = false);
                })
                .on_mouse_up(gpui::MouseButton::Left, move |_, _, cx| {
                    let _ = up_weak.update(cx, |this, _| this.titlebar_drag_armed = false);
                })
                .on_mouse_down(gpui::MouseButton::Left, move |event, window, cx| {
                    let _ = down_weak.update(cx, |this, cx| {
                        this.titlebar_drag_armed = true;
                        platform_ui::titlebar_mouse_down(event, window, cx);
                    });
                })
                .into_any_element()
        });
        let make_collapse_button: Rc<dyn Fn(&mut gpui::App) -> AnyElement> = Rc::new(move |_| {
            let collapse_root = collapse_root.clone();
            IconButton::new("collapse-mission-rail", "panel-right-open.svg")
                .tooltip(crate::i18n::text("ui.collapse_side_panel"))
                .on_press(move |_, cx| {
                    collapse_root.update(cx, |this, cx| {
                        this.set_rail_open(false, cx);
                        cx.notify();
                    });
                })
                .into_any_element()
        });
        let owner = cx.entity_id();
        let content = self.rail_files.render_panel(
            crate::surfaces::files::SidePanelRenderProps {
                surface: PanelSurface::Mission,
                settings,
                selection: self.rail_surfaces.selection,
                shown,
                context: rail_context,
                widened,
                maximized,
                remote: self.presentation.is_remote(),
                owner,
                header_selector: "MISSION_RAIL_HEADER",
                titlebar_id: "mission-rail-titlebar-drag",
                round_header_tl_when_compact: true,
                macos_titlebar_inset,
                wrap_titlebar,
                on_surface_action: handler,
                maximize_control,
                on_request_maximize,
                make_collapse_button,
                sessions_body,
                sessions_focus: Some(self.rail_focus.clone()),
                loading_fallback: None,
            },
            cx,
        );
        let rail = div()
            .relative()
            .when(maximized, |r| r.w_full().flex_1().min_w(px(0.)))
            .when(!maximized, |r| {
                if sheet || visibility >= 1. {
                    r.w_full()
                } else {
                    r.w(px(width * zoom - 2.)).flex_none()
                }
            })
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(content);
        div()
            .id("mission-rail")
            .debug_selector(|| "MISSION_RAIL".into())
            .track_focus(&self.rail_focus)
            .relative()
            .when(maximized, |r| r.flex_1().min_w(px(0.)).w_full())
            .when(!maximized, |r| r.w(rems(visible_width / 16.)).flex_none())
            .h_full()
            .overflow_hidden()
            .rounded(rems(12. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .child(rail)
            .into_any_element()
    }

    fn render_roles_rail(
        &self,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // A rail widened by an open file places two cards per row only when the
        // Sessions rail itself is wide enough; when sharing the panel beside the
        // unified viewer at 240 px, cards stay single-column with compact padding.
        let compact = width < 270.;
        let pad_x = if compact { 12. } else { 20. };
        let two_up = width >= side_panel::ROSTER_TWO_UP_WIDTH;
        let card_width = (width - 2. * pad_x - 12.) / 2.;
        let fit_width = if two_up {
            card_width + 2. * pad_x
        } else {
            width
        };
        let mut cards = Vec::new();
        let selected = match &self.active_tab {
            MissionTab::Session(session_id) => Some(session_id.as_str()),
            MissionTab::Feed => None,
        };
        let mut list = div()
            .id("mission-roles-scroll")
            .debug_selector(|| "MISSION_ROLES_SCROLL".into())
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .when(compact, |el| el.px_3())
            .when(!compact, |el| el.px_5())
            .pb_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(rail_section_label(crate::i18n::text("ui.sessions")));
        let root = cx.entity();
        for session in &self.members() {
            let session_id = session.session.id.clone();
            let open_id = session_id.clone();
            let open_root = root.clone();
            let card_key_id = session_id.clone();
            let card_key_root = root.clone();
            let is_shell = runner_core::protocol::model::Runtime::parse(&session.runtime)
                .is_some_and(runner_core::protocol::model::Runtime::is_shell);
            let subtitle = (!is_shell).then(|| {
                let status = self
                    .presentation
                    .status_presentation(&self.slot_agent_status(&session_id, cx));
                runner_app::ui::agent_status::status_indicator(
                    status,
                    true,
                    SharedString::from(format!("mission-card-status-{session_id}")),
                    window,
                    cx,
                )
            });
            let runtime_line = session_runtime_line(session).map(|runtime_text| {
                div().w_full().min_w(px(0.)).flex().child(
                    Tooltip::new(
                        SharedString::from(format!("mission-card-runtime-{session_id}")),
                        runtime_text.clone(),
                        div()
                            .debug_selector(|| format!("MISSION_CARD_RUNTIME {session_id}"))
                            .min_w(px(0.))
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap(rems(6. / 16.))
                            .child(
                                div()
                                    .debug_selector(|| {
                                        format!("MISSION_CARD_RUNTIME_MARK {session_id}")
                                    })
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .child(crate::chat_icon::runtime_mark(&session.runtime, 12.)),
                            )
                            .child(
                                div()
                                    .debug_selector(|| {
                                        format!("MISSION_CARD_RUNTIME_TEXT {session_id}")
                                    })
                                    .min_w(px(0.))
                                    .flex_initial()
                                    .truncate()
                                    .text_size(theme::text_meta())
                                    .line_height(rems(14. / 16.))
                                    .text_color(theme::muted())
                                    .child(runtime_text),
                            ),
                    )
                    .expand(),
                )
            });
            let disabled = self.stopping
                || self.resuming
                || self.archiving
                || self.slot_actions.contains(&session_id)
                || self.transitions.contains_key(&session_id);
            let available_controls = slot_controls(session.session.status)
                .into_iter()
                .filter(|action| {
                    self.remote_supports(&[match action {
                        SessionControlKind::Stop => "stop",
                        SessionControlKind::Restart => "restart",
                        _ => "resume",
                    }])
                })
                .collect::<Vec<_>>();
            let can_remove = self.remote_supports(&["removeMissionRole"]);
            let controls = ((!available_controls.is_empty() || can_remove)
                && mission_slot_actions_available(
                    self.mission_view().as_ref().map(|mission| mission.status),
                    self.archived(),
                    self.secondary,
                ))
            .then(|| {
                let remove_root = root.clone();
                let remove_id = session_id.clone();
                let remove_selector_id = session_id.clone();
                div()
                    .debug_selector(|| format!("MISSION_CARD_CONTROLS {session_id}"))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .children(available_controls.into_iter().map(|action| {
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
                    }))
                    .when(can_remove, |controls| {
                        controls.child(
                            div()
                                .when(cfg!(test), move |el| {
                                    el.debug_selector(move || {
                                        format!("MISSION_CARD_REMOVE {remove_selector_id}")
                                    })
                                })
                                .child(
                                    IconButton::new(
                                        SharedString::from(format!("remove-role-{session_id}")),
                                        "log-out.svg",
                                    )
                                    .size(runner_app::ui::IconButtonSize::Sm)
                                    .tooltip(crate::i18n::text("ui.remove_from_mission"))
                                    .stop_click_propagation(true)
                                    .disabled(disabled)
                                    .on_press(
                                        move |window, cx| {
                                            remove_root.update(cx, |this, cx| {
                                                this.request_remove_role(&remove_id, window, cx)
                                            })
                                        },
                                    ),
                                ),
                        )
                    })
            });
            let handle_title = SharedString::from(format!("@{}", session.handle));
            let mut header_gap = rems(8. / 16.);
            let mut header_margin = px(0.);
            if controls.is_some() {
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
                let extra = rems(33. / 16.).to_pixels(window.rem_size());
                let fixed_width = rems((40. + 24. + 52. + 12. + 24.) / 16.)
                    .to_pixels(window.rem_size())
                    + px(2.)
                    + extra;
                let rail_width = rems(fit_width / 16.).to_pixels(window.rem_size());
                let available_width = rail_width - fixed_width;
                if available_width < needed_width {
                    header_gap = rems(2. / 16.);
                    let compact_width =
                        available_width + rems(12. / 16.).to_pixels(window.rem_size());
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
            cards.push(
                div()
                    .debug_selector(|| format!("MISSION_CARD {session_id}"))
                    .id(SharedString::from(format!(
                        "mission-role-card-{session_id}"
                    )))
                    .map(|card| {
                        if two_up {
                            card.flex_1().min_w(px(0.))
                        } else {
                            card.w_full()
                        }
                    })
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
                    .bg(theme::panel())
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
                                    .flex_1()
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
                                        div()
                                            .min_w(px(0.))
                                            .flex_1()
                                            .flex()
                                            .flex_col()
                                            .gap(rems(3. / 16.))
                                            .child(
                                                div().w_full().min_w(px(0.)).flex().child(
                                                    Tooltip::new(
                                                        SharedString::from(format!(
                                                            "mission-role-title-{session_id}"
                                                        )),
                                                        self.session_title_tooltip(session, cx),
                                                        div()
                                                            .debug_selector(|| {
                                                                format!(
                                                                    "MISSION_CARD_HANDLE {session_id}"
                                                                )
                                                            })
                                                            .min_w(px(0.))
                                                            .flex_initial()
                                                            .truncate()
                                                            .font_family(theme::UI_MONOSPACE_FONT)
                                                            .text_size(theme::text_body())
                                                            .line_height(rems(1.))
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(
                                                                runner_app::ui::hue_for_seed(
                                                                    &session.handle,
                                                                )
                                                                .color(),
                                                            )
                                                            .child(handle_title),
                                                    )
                                                    .expand(),
                                                ),
                                            )
                                            .when_some(subtitle, |col, subtitle| {
                                                col.child(
                                                    div()
                                                        .debug_selector(|| {
                                                            format!(
                                                                "MISSION_CARD_STATUS {session_id}"
                                                            )
                                                        })
                                                        .w_full()
                                                        .min_w(px(0.))
                                                        .text_size(theme::text_meta())
                                                        .line_height(rems(14. / 16.))
                                                        .text_color(theme::muted())
                                                        .child(subtitle),
                                                )
                                            }),
                                    ),
                            )
                            .children(controls),
                    )
                    .when_some(runtime_line, |card, line| card.child(line))
                    .when(!self.presentation.is_remote(), |card| {
                        card.child(
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
                                                .when(session.agent_session_key.is_some(), |el| {
                                                    el.my(rems(-3. / 16.))
                                                })
                                                .flex_none()
                                                .child(copy)
                                        })),
                                ),
                        )
                    }),
            );
        }
        if two_up {
            let mut cards = cards.into_iter();
            while let Some(first) = cards.next() {
                // The filler carries the card's padding and border, which
                // flex adds to a zero basis, so a lone card keeps half a row.
                let second = cards.next().map_or_else(
                    || {
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .p_3()
                            .border_1()
                            .into_any_element()
                    },
                    IntoElement::into_any_element,
                );
                list = list.child(div().flex().gap_3().child(first).child(second));
            }
        } else {
            list = list.children(cards);
        }
        if self.mission_composer_roster().is_empty() {
            list = list.child(
                div()
                    .debug_selector(|| "MISSION_RAIL_NO_ROLES".into())
                    .text_size(theme::text_ui())
                    .text_color(theme::faint())
                    .child(crate::i18n::text("ui.mission_roster_empty")),
            );
        }
        if self.remote_supports(&["catalog", "addMissionRole"])
            && mission_slot_actions_available(
                self.mission_view().as_ref().map(|mission| mission.status),
                self.archived(),
                self.secondary,
            )
        {
            let shell = self.shell.clone();
            let workspace = cx.entity();
            list = list.child(
                div()
                    .debug_selector(|| "MISSION_RAIL_ADD_ROLE".into())
                    .child(
                        Button::new("mission-add-role", crate::i18n::text("ui.add_role"))
                            .full_width(true)
                            .variant(ButtonVariant::Outline)
                            .icon("plus.svg")
                            .on_press(move |window, cx| {
                                if let Some(context) =
                                    workspace.read(cx).add_role_context(window, cx)
                                {
                                    let _ = shell.update(cx, |this, cx| {
                                        this.open_add_role_modal(context, window, cx)
                                    });
                                }
                            }),
                    ),
            );
        }
        list.into_any_element()
    }

    fn reveal_mission_cwd(&mut self, cwd: String, cx: &mut Context<Self>) {
        if self.presentation.is_remote() {
            return;
        }
        #[cfg(windows)]
        {
            crate::surfaces::files::reveal_in_explorer(std::path::Path::new(&cwd), cx);
        }
        #[cfg(not(windows))]
        {
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
                #[cfg(target_os = "linux")]
                {
                    let status = Command::new("xdg-open")
                        .arg(cwd)
                        .status()
                        .map_err(|error| error.to_string())?;
                    if status.success() {
                        Ok(())
                    } else {
                        Err(format!("File manager exited with status {status}"))
                    }
                }
            });
            cx.spawn(async move |weak, cx| {
                let result = task.await;
                let _ = weak.update(cx, |this, cx| {
                    if let Err(error) = result {
                        this.error = Some(crate::i18n::messagef(
                            "format.couldn_t_reveal_the_working_directory",
                            &[("error", (error).to_string())],
                        ));
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    pub(super) fn render_mission_info_popover_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(mission) = self.mission_view() else {
            return div().into_any_element();
        };
        let cwd_root = cx.entity();
        let goal = self.goal();
        let permission_mode = self.permission_mode();
        let shortcut_hint = crate::keymap::effective_binding(
            "toggle-session-info",
            &self.settings(cx).keymap_overrides,
        )
        .map(|combo| crate::keymap::format_combo(&combo));
        let branch_info =
            if let crate::surfaces::front_page::FrontPageTarget::Remote { computer, .. } =
                &self.client.target
            {
                crate::surfaces::panes::remote_info_branch_row(
                    &self.app_store,
                    computer,
                    Some(&mission.id),
                    None,
                    mission.cwd.as_deref(),
                    cx,
                )
            } else {
                crate::surfaces::panes::ensure_info_branch_row(
                    &self.app_store,
                    mission.cwd.as_deref(),
                    mission.project_id.as_deref(),
                    false,
                    cx,
                )
            };
        let location = crate::surfaces::panes::resolve_info_location_rows(
            mission.cwd.as_deref(),
            branch_info.as_ref(),
        );
        self.mission_worktree_copy.update(cx, |btn, btn_cx| {
            btn.set_value(
                location.worktree.as_ref().and_then(|wt| wt.path.clone()),
                btn_cx,
            );
        });
        let info_row = |label: &'static str, value: AnyElement| {
            div()
                .flex()
                .items_start()
                .gap_3()
                .text_size(theme::text_meta())
                .child(
                    div()
                        .w(rems(72. / 16.))
                        .flex_none()
                        .text_color(theme::faint())
                        .child(label),
                )
                .child(div().flex_1().min_w(px(0.)).child(value))
        };

        let mut panel = div()
            .id("mission-meta-scroll")
            .debug_selector(|| "MISSION_META_SCROLL".into())
            .p(rems(12. / 16.))
            .flex()
            .flex_col()
            .gap(rems(8. / 16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(rail_section_label(crate::i18n::text("ui.mission_detail")))
                    .children(shortcut_hint.map(|hint| {
                        div()
                            .flex_none()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_micro())
                            .text_color(theme::faint())
                            .child(hint)
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(3. / 16.))
                    .child(
                        div()
                            .text_size(theme::text_caption())
                            .text_color(theme::faint())
                            .child(crate::i18n::text("ui.goal")),
                    )
                    .child(match goal {
                        Some(goal) if !goal.is_empty() => div()
                            .id("mission-info-goal-scroll")
                            .max_h(rems(120. / 16.))
                            .overflow_y_scroll()
                            .text_size(theme::text_meta())
                            .line_height(rems(17. / 16.))
                            .text_color(theme::text())
                            .child(goal)
                            .into_any_element(),
                        Some(_) => div()
                            .text_size(theme::text_meta())
                            .italic()
                            .text_color(theme::faint())
                            .child(crate::i18n::text("ui.no_goal_set"))
                            .into_any_element(),
                        None => div()
                            .text_size(theme::text_meta())
                            .italic()
                            .text_color(theme::faint())
                            .child(crate::i18n::text("ui.loading"))
                            .into_any_element(),
                    }),
            )
            .child(div().h(px(1.)).w_full().bg(theme::border()));

        if let Some(worktree) = location.worktree {
            let full_path = worktree.path.clone();
            let branch_text = div()
                .min_w(px(0.))
                .flex_1()
                .truncate()
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_color(if worktree.missing {
                    theme::warning()
                } else {
                    theme::text()
                })
                .child(worktree.branch);
            let branch_label = if let Some(path) = full_path {
                Tooltip::new("mission-info-worktree-path", path, branch_text)
                    .expand()
                    .into_any_element()
            } else {
                branch_text.into_any_element()
            };
            panel = panel.child(info_row(
                crate::i18n::text("info.worktree"),
                div()
                    .when(cfg!(test), |el| {
                        el.debug_selector(|| "MISSION_META_BRANCH".into())
                    })
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .child(
                        div()
                            .w_full()
                            .min_w(px(0.))
                            .flex()
                            .items_center()
                            .gap(rems(6. / 16.))
                            .child(branch_label)
                            .child(self.mission_worktree_copy.clone()),
                    )
                    .child(crate::surfaces::panes::info_worktree_marker(
                        worktree.marker,
                    ))
                    .into_any_element(),
            ));
        }

        if let Some(cwd) = location.cwd {
            let reveal_path = mission.cwd.clone().unwrap_or_else(|| cwd.clone());
            let cwd_el = if !self.presentation.is_remote() {
                #[cfg(target_os = "macos")]
                let reveal_label = crate::i18n::text("ui.reveal_in_finder");
                #[cfg(target_os = "linux")]
                let reveal_label = crate::i18n::text("ui.open_working_directory");
                #[cfg(windows)]
                let reveal_label = crate::i18n::text("ui.reveal_in_explorer");
                Tooltip::new(
                    "reveal-mission-cwd-tooltip",
                    reveal_label,
                    div()
                        .id("reveal-mission-cwd")
                        .min_w(px(0.))
                        .truncate()
                        .cursor_pointer()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_color(theme::muted())
                        .hover(|el| el.text_color(theme::text()).underline())
                        .on_click(move |_, _, cx| {
                            let reveal = reveal_path.clone();
                            cwd_root.update(cx, |this, cx| this.reveal_mission_cwd(reveal, cx));
                        })
                        .child(cwd),
                )
                .expand()
                .into_any_element()
            } else {
                div()
                    .min_w(px(0.))
                    .truncate()
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_color(theme::muted())
                    .child(cwd)
                    .into_any_element()
            };
            panel = panel.child(info_row("cwd", cwd_el));
        }

        panel
            .children(permission_mode.map(|mode| {
                info_row(
                    crate::i18n::text("ui.permissions"),
                    div()
                        .id("mission-permission-mode")
                        .text_color(theme::text())
                        .child(mode)
                        .into_any_element(),
                )
            }))
            .children(mission.started_at.map(|started_at| {
                info_row(
                    crate::i18n::text("ui.started"),
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(5. / 16.))
                        .text_color(theme::muted())
                        .child(
                            svg()
                                .flex_none()
                                .path("clock.svg")
                                .size(rems(11. / 16.))
                                .text_color(theme::faint()),
                        )
                        .child(runner_app::text_util::format_relative_time(
                            started_at,
                            Utc::now(),
                        ))
                        .into_any_element(),
                )
            }))
            .child(info_row(
                "mission_id",
                div()
                    .flex()
                    .items_center()
                    .gap(rems(6. / 16.))
                    .child(
                        div()
                            .debug_selector(|| "MISSION_META_ID".into())
                            .min_w(px(0.))
                            .flex_1()
                            .truncate()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_color(theme::muted())
                            .child(mission.id.clone()),
                    )
                    .child(self.mission_id_copy.clone())
                    .into_any_element(),
            ))
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
                    .child(crate::i18n::text("ui.rename_mission")),
            )
            .child(
                IconButton::new("close-mission-rename", "close.svg")
                    .focus_handle(modal.close_focus.clone())
                    .tooltip(crate::i18n::text("ui.close_rename"))
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
                Field::new(
                    "mission-rename-name",
                    crate::i18n::text("ui.name"),
                    modal.input.clone(),
                )
                .focus_target(modal.input.read(cx).focus_handle())
                .emphasized(true),
            );
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cancel-mission-rename", crate::i18n::text("ui.cancel"))
                    .focus_handle(modal.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_mission_rename(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "submit-mission-rename",
                    if submitting {
                        crate::i18n::text("ui.saving")
                    } else {
                        crate::i18n::text("ui.save")
                    },
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

pub(super) fn session_runtime_line(
    session: &crate::surfaces::front_page::model::MissionMemberDisplay,
) -> Option<String> {
    let runtime = session.runtime.trim();
    let parsed = runner_core::protocol::model::Runtime::parse(runtime);
    if runtime.is_empty() || parsed.is_some_and(runner_core::protocol::model::Runtime::is_shell) {
        return None;
    }
    let display_name = parsed
        .map(runner_core::protocol::model::Runtime::display_name)
        .unwrap_or(runtime);
    let mut parts = vec![display_name.to_owned()];
    if let Some(model) = session
        .agent_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("default"))
    {
        parts.push(model.to_owned());
    }
    if let Some(effort) = session
        .agent_effort
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("default"))
    {
        parts.push(
            runner_app::ui::select::effort_display_label(effort)
                .resolve()
                .to_string(),
        );
    }
    Some(parts.join(" · "))
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

fn tracked_uppercase(label: &str) -> String {
    label
        .to_uppercase()
        .chars()
        .map(|character| character.to_string())
        .collect::<Vec<_>>()
        .join("\u{2009}")
}

impl remote::MissionPresentation {
    pub(super) fn status_presentation(
        &self,
        status: &runner_core::protocol::status::AgentStatus,
    ) -> runner_app::ui::agent_status::StatusPresentation {
        let mut display = runner_app::ui::agent_status::StatusPresentation::new(status);
        if self.is_remote() {
            display.waiting_since = None;
            display.estimated = false;
        }
        display
    }
}
