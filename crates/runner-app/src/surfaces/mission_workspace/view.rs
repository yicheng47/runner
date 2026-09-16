use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
#[cfg(target_os = "macos")]
use gpui::WeakEntity;
use gpui::{
    div, px, rems, svg, AnyElement, App, CursorStyle, DragMoveEvent, FontWeight, MouseButton,
    SharedString, Window, WindowControlArea,
};
use runner_app::ui::{
    Button, ButtonSize, IconButton, SessionControl, SessionControlKind, SessionControlVariant,
    SessionOverlay, SessionOverlayKind,
};
use runner_backend::model::MissionStatus;

use super::*;
#[cfg(target_os = "macos")]
use crate::surfaces::app_shell::{SIDEBAR_TOGGLE_GLYPH_INSET, SIDEBAR_TOGGLE_GLYPH_X};
use crate::surfaces::*;
use crate::*;

impl MissionWorkspace {
    fn workspace_titlebar_padding(&self, window: &Window, cx: &App) -> f32 {
        #[cfg(target_os = "macos")]
        {
            if self.sidebar_collapsed && !window.is_fullscreen() {
                SIDEBAR_TOGGLE_GLYPH_X - SIDEBAR_TOGGLE_GLYPH_INSET * self.settings(cx).app_zoom
            } else {
                16. * self.settings(cx).app_zoom
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = window;
            16. * self.settings(cx).app_zoom
        }
    }

    fn render_open_sidebar_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.sidebar_collapsed {
            return None;
        }
        let toggle = self.render_open_sidebar_toggle(cx);
        #[cfg(target_os = "macos")]
        {
            let shell = self.shell.clone();
            let (can_go_back, can_go_forward) = shell
                .upgrade()
                .map(|shell| shell.read(cx).page_navigation_state())
                .unwrap_or((false, false));
            let navigate = |shell: WeakEntity<NativeRoot>, direction: isize| {
                move |window: &mut Window, cx: &mut App| {
                    if let Some(shell) = shell.upgrade() {
                        shell.update(cx, |shell, shell_cx| {
                            shell.navigate_runtime_page(direction, window, shell_cx)
                        });
                    }
                }
            };
            Some(
                NativeRoot::titlebar_control_cluster("mission-titlebar-controls")
                    .child(toggle)
                    .child(
                        IconButton::new("window-previous-page", "chevron-left.svg")
                            .disabled(!can_go_back)
                            .on_press(navigate(shell.clone(), -1)),
                    )
                    .child(
                        IconButton::new("window-next-page", "chevron-right.svg")
                            .disabled(!can_go_forward)
                            .on_press(navigate(shell, 1)),
                    )
                    .into_any_element(),
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            Some(toggle)
        }
    }

    fn render_open_sidebar_toggle(&self, cx: &mut Context<Self>) -> AnyElement {
        {
            div()
                .id("open-sidebar")
                .group("open-sidebar")
                .flex_none()
                .w(px(28. * self.settings(cx).app_zoom))
                .h(px(28. * self.settings(cx).app_zoom))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .cursor_pointer()
                .text_color(theme::muted())
                .hover(|button| button.bg(theme::raised()).text_color(theme::text()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    svg()
                        .path("panel-left-hidden.svg")
                        .w(px(14. * self.settings(cx).app_zoom))
                        .h(px(14. * self.settings(cx).app_zoom))
                        .flex_none()
                        .text_color(theme::muted())
                        .group_hover("open-sidebar", |icon| icon.text_color(theme::text())),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.sidebar_collapsed = false;
                    if let Some(shell) = this.shell.upgrade() {
                        cx.defer(move |cx| {
                            shell.update(cx, |shell, shell_cx| {
                                shell.set_sidebar_collapsed(false, true, shell_cx);
                                shell.sidebar_preview_open = false;
                                shell.sidebar_preview_peeking = false;
                            });
                        });
                    }
                    this.focus_active_mission_terminal(window, cx);
                    cx.notify();
                }))
                .into_any_element()
        }
    }

    fn render_titlebar_drag_area(
        &self,
        id: &'static str,
        area: gpui::Div,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let area = area
            .id(id)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down_out(cx.listener(|this, _, _, _| {
                this.titlebar_drag_armed = false;
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.titlebar_drag_armed = false;
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.titlebar_drag_armed = true;
                }),
            );
        #[cfg(target_os = "macos")]
        let area = area
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.titlebar_drag_armed {
                    this.titlebar_drag_armed = false;
                    window.start_window_move();
                }
            }))
            .on_click(|event, window, cx| {
                if event.click_count() == 2 {
                    cx.stop_propagation();
                    window.titlebar_double_click();
                }
            });
        area
    }

    pub(crate) fn render_mission_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.mission.is_some() {
            self.configure_mission_action_menu(cx);
        }
        let header = self.render_mission_header(window, cx);
        let notices = self.render_mission_notices(cx);
        let body = if self.loading {
            div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .children(self.loading_overlay_visible.then(|| {
                    SessionOverlay::transition("mission-loading", SessionOverlayKind::Starting)
                        .label("Loading mission…")
                }))
                .into_any_element()
        } else if self.mission.is_none() {
            self.render_mission_load_error(cx)
        } else {
            self.render_loaded_mission(window, cx)
        };
        let center = div()
            .min_w(px(0.))
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .child(header)
            .children(notices)
            .child(body)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_terminal_drawer_resize(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_terminal_drawer_resize(cx)),
            );
        let rail_open = self.settings(cx).mission_rail_open;
        let (rail_visibility, rail_animating) = self.rail_visibility.animate_to(
            if rail_open { 1. } else { 0. },
            Instant::now(),
            Duration::from_millis(MISSION_RAIL_TRANSITION_MS),
        );
        if rail_animating {
            window.request_animation_frame();
        }
        let rail = self.render_mission_rail(
            rail_visibility,
            rail_open || rail_animating,
            rail_open && !rail_animating,
            cx,
        );
        div()
            .relative()
            .key_context("Mission")
            .track_focus(&self.root_focus)
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .bg(theme::bg())
            .map(|element| {
                #[cfg(test)]
                let element = crate::theme_snapshot::record_fill("MISSION_BG", element);
                element
            })
            .child(center)
            .child(rail)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if !this.feed_selecting && this.feed_selection.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_action(cx.listener(Self::focus_previous_mission_tab))
            .on_action(cx.listener(Self::focus_next_mission_tab))
            .on_drag_move::<MissionRailResizeDrag>(cx.listener(
                |this, event: &DragMoveEvent<MissionRailResizeDrag>, _, cx| {
                    let width = f32::from(event.bounds.right() - event.event.position.x)
                        / this.settings(cx).app_zoom;
                    let width = app_settings::clamp_mission_rail_width(width);
                    this.update_app_settings(cx, false, |settings| {
                        if settings.mission_rail_width == width {
                            return false;
                        }
                        settings.mission_rail_width = width;
                        true
                    });
                },
            ))
            .on_drop(cx.listener(|this, _: &MissionRailResizeDrag, _, cx| {
                this.save_settings(cx);
            }))
            .into_any_element()
    }

    fn focus_previous_mission_tab(
        &mut self,
        _: &MissionTabPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_mission_tab(-1, window, cx);
    }

    fn focus_next_mission_tab(
        &mut self,
        _: &MissionTabNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_mission_tab(1, window, cx);
    }

    pub(crate) fn render_mission_overlays(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut overlays = Vec::new();
        if self.rename_modal.is_some() {
            overlays.push(self.render_mission_rename_modal(cx));
        }
        if self.stop_all_confirm {
            overlays.push(self.render_stop_all_confirm(cx));
        }
        if self.restart_confirm.is_some() {
            overlays.push(self.render_restart_confirm(cx));
        }
        overlays
    }

    fn render_mission_header(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mission = self.mission.clone();
        let busy = self.lifecycle_busy();
        let secondary = self.secondary;
        let all_live = self.all_sessions_live();
        let any_stopped = !self.sessions.is_empty() && !all_live;
        let root = cx.entity();
        let resume_root = root.clone();
        let stop_root = root.clone();
        let drawer_root = root.clone();
        let open_rail_root = root;
        let controls = mission.as_ref().and_then(|mission| {
            (mission.status == MissionStatus::Running && !secondary).then(|| {
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .children((!self.resuming).then(|| self.action_menu.clone()))
                    .children((any_stopped && !busy).then(|| {
                        SessionControl::new("mission-resume", SessionControlKind::Resume)
                            .variant(SessionControlVariant::Header)
                            .title("Respawn every stopped slot in this mission")
                            .on_press(move |window, cx| {
                                resume_root
                                    .update(cx, |this, cx| this.resume_open_mission(window, cx));
                            })
                    }))
                    .children((self.any_session_live() && !busy).then(|| {
                        SessionControl::new("mission-stop", SessionControlKind::Stop)
                            .variant(SessionControlVariant::Header)
                            .title("Stop all slots")
                            .on_press(move |window, cx| {
                                stop_root
                                    .update(cx, |this, cx| this.open_stop_all_confirm(window, cx));
                            })
                    }))
            })
        });
        let title = mission
            .as_ref()
            .map(|mission| mission.title.clone())
            .unwrap_or_else(|| "…".into());
        let drawer_action = mission_drawer_available(self.archived(), secondary).then(|| {
            let open = self.layout.drawer.open();
            IconButton::new(
                "mission-terminal-drawer-toggle",
                if open {
                    "panel-bottom-open.svg"
                } else {
                    "panel-bottom-hidden.svg"
                },
            )
            .tooltip(super::panes::terminal_drawer_tooltip(
                open,
                &self.settings(cx).keymap_overrides,
            ))
            .on_press(move |window, cx| {
                drawer_root.update(cx, |this, cx| this.toggle_terminal_drawer(window, cx));
            })
            .into_any_element()
        });
        let rail_action = (!self.settings(cx).mission_rail_open).then(|| {
            IconButton::new("open-mission-rail", "panel-right-hidden.svg")
                .tooltip("Open sessions panel")
                .on_press(move |_, cx| {
                    open_rail_root.update(cx, |this, cx| {
                        this.update_app_settings(cx, true, |settings| {
                            settings.mission_rail_open = true;
                            true
                        });
                        cx.notify();
                    });
                })
                .into_any_element()
        });
        let row = WorkspaceHeader::new(
            px(self.workspace_titlebar_padding(window, cx)),
            "flag.svg",
            title,
        )
        .sidebar_toggle(self.render_open_sidebar_button(cx))
        .title_actions(controls.into_iter().map(IntoElement::into_any_element))
        .trailing_actions(drawer_action.into_iter().chain(rail_action))
        .into_div();
        self.render_titlebar_drag_area("mission-titlebar-drag", row, cx)
            .into_any_element()
    }

    fn render_mission_notices(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut notices = Vec::new();
        if let Some(error) = self.error.clone() {
            let root = cx.entity();
            notices.push(
                mission_notice("error", error, theme::danger(), "Dismiss", move |_, cx| {
                    root.update(cx, |this, cx| {
                        this.error = None;
                        cx.notify();
                    });
                })
                .into_any_element(),
            );
        }
        if let Some(warning) = self.warning.clone() {
            let root = cx.entity();
            notices.push(
                mission_notice(
                    "warning",
                    warning,
                    theme::warning(),
                    "Dismiss",
                    move |_, cx| {
                        root.update(cx, |this, cx| {
                            this.warning = None;
                            cx.notify();
                        });
                    },
                )
                .into_any_element(),
            );
        }
        notices
    }

    fn render_mission_load_error(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(mission_id) = self.mission_id.clone() else {
            return div().into_any_element();
        };
        let root = cx.entity();
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .text_center()
                    .child(
                        div()
                            .text_size(theme::text_title())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Mission unavailable"),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child("Runner couldn't attach this mission."),
                    )
                    .child(
                        Button::new("retry-mission-load", "Retry")
                            .size(ButtonSize::Sm)
                            .on_press(move |window, cx| {
                                root.update(cx, |this, cx| {
                                    this.open_mission(mission_id.clone(), window, cx)
                                });
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_mission_terminal_drawer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let active_id = self.layout.drawer.active_shell().map(str::to_owned);
        let labels = self
            .layout
            .drawer
            .shells()
            .iter()
            .map(|session_id| {
                self.drawer_session_entry(session_id, cx)
                    .map(default_session_label)
                    .unwrap_or_else(|| "shell".into())
            })
            .collect::<Vec<_>>();
        let root = cx.entity();
        let activate_root = root.clone();
        let close_root = root.clone();
        let add_root = root.clone();
        let hide_root = root;
        let strip = super::panes::render_terminal_drawer_strip(
            "mission",
            self.layout.drawer.shells(),
            active_id.as_deref(),
            &labels,
            super::panes::TerminalDrawerCallbacks {
                activate: Rc::new(move |session_id, window, cx| {
                    activate_root.update(cx, |this, cx| {
                        this.activate_terminal_drawer_shell(&session_id, window, cx)
                    });
                }),
                close: Rc::new(move |session_id, window, cx| {
                    close_root.update(cx, |this, cx| {
                        this.request_close_terminal_drawer_shell(&session_id, window, cx)
                    });
                }),
                add: Rc::new(move |window, cx| {
                    add_root.update(cx, |this, cx| this.add_terminal_drawer_shell(window, cx));
                }),
                hide: Rc::new(move |window, cx| {
                    hide_root.update(cx, |this, cx| this.hide_terminal_drawer(window, cx));
                }),
            },
        );
        let body = active_id
            .as_deref()
            .and_then(|session_id| self.render_mission_drawer_terminal(session_id, cx))
            .unwrap_or_else(|| {
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(theme::text_ui())
                    .text_color(theme::faint())
                    .child("Terminal unavailable")
                    .into_any_element()
            });
        div()
            .id("mission-terminal-drawer")
            .track_focus(&self.drawer_focus)
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(strip)
            .child(body)
            .into_any_element()
    }

    fn render_mission_drawer_terminal(
        &mut self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let entry = self.drawer_session_entry(session_id, cx)?.clone();
        let session_id = entry.session_id.clone();
        let transition = self.transition_kind(&session_id).map(|kind| match kind {
            MissionTransitionKind::Starting => chat_lifecycle::TransitionKind::Starting,
            MissionTransitionKind::Restarting => chat_lifecycle::TransitionKind::Resuming,
            MissionTransitionKind::Resuming => chat_lifecycle::TransitionKind::Resuming,
        });
        let overlay = chat_lifecycle::resolve_pane_overlay(
            false,
            transition,
            entry.status,
            entry.resumable,
            self.drawer_exit_codes.get(&session_id).copied().flatten(),
        );
        let interactive = self.cached_mission_terminal_interactive(&session_id, cx);
        let scrollable = self.is_active(cx) && transition.is_none();
        let terminal_style = self.terminal_style(cx);
        let terminal_background =
            crate::terminal::element::to_hsla(terminal_style.palette.background, 1.);
        let terminal_surface = if let Some(chat) = self.attached.get(&session_id) {
            let terminal = Arc::clone(&chat.terminal);
            let terminal_interaction = chat.terminal_interaction.clone();
            let terminal_scrollbar = chat.terminal_scrollbar.clone();
            let terminal_input = chat.terminal_input.clone();
            let terminal_focus = chat.terminal_focus.clone();
            let key_id = session_id.clone();
            let copy_id = session_id.clone();
            let scroll_id = session_id.clone();
            let paste_id = session_id.clone();
            let root = cx.entity();
            let copy_root = root.clone();
            let mut surface = div()
                .id(SharedString::from(format!(
                    "mission-drawer-terminal-{session_id}"
                )))
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
                    chat_lifecycle::PaneOverlayState::Starting
                    | chat_lifecycle::PaneOverlayState::Resuming => 0.,
                    chat_lifecycle::PaneOverlayState::Ended { .. } => 0.45,
                    chat_lifecycle::PaneOverlayState::Archiving
                    | chat_lifecycle::PaneOverlayState::None => 1.,
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
                        .child(TerminalElement::new(
                            terminal,
                            terminal_interaction,
                            terminal_input,
                            terminal_focus,
                            interactive,
                            scrollable,
                            terminal_style,
                        ))
                        .child(terminal_scrollbar),
                );
            if interactive {
                let key_root = root.clone();
                let paste_root = root.clone();
                surface = surface
                    .on_key_down(move |event, window, cx| {
                        key_root.update(cx, |this, cx| {
                            this.on_mission_key_down(&key_id, event, window, cx)
                        });
                    })
                    .on_action(move |action: &Paste, window, cx| {
                        paste_root.update(cx, |this, cx| {
                            this.on_mission_paste(&paste_id, action, window, cx)
                        });
                    });
            }
            if scrollable {
                surface = surface.on_scroll_wheel(move |event, window, cx| {
                    root.update(cx, |this, cx| {
                        this.on_mission_scroll(&scroll_id, event, window, cx)
                    });
                });
            }
            surface.into_any_element()
        } else {
            div()
                .absolute()
                .inset_0()
                .bg(terminal_background)
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .when(
                    matches!(overlay, chat_lifecycle::PaneOverlayState::None),
                    |surface| {
                        surface
                            .flex()
                            .items_center()
                            .justify_center()
                            .child("Attaching terminal…")
                    },
                )
                .into_any_element()
        };
        let overlay_element = match overlay {
            chat_lifecycle::PaneOverlayState::Resuming => Some(
                SessionOverlay::transition(
                    format!("mission-drawer-resuming-{session_id}"),
                    SessionOverlayKind::Resuming,
                )
                .label("Restarting terminal…")
                .into_any_element(),
            ),
            chat_lifecycle::PaneOverlayState::Starting => Some(
                SessionOverlay::transition(
                    format!("mission-drawer-starting-{session_id}"),
                    SessionOverlayKind::Starting,
                )
                .label("Starting terminal…")
                .into_any_element(),
            ),
            chat_lifecycle::PaneOverlayState::Ended { exit_code, .. } => {
                let restart_root = cx.entity();
                let close_root = restart_root.clone();
                let restart_id = session_id.clone();
                let close_id = session_id.clone();
                Some(
                    SessionOverlay::shell_exited(
                        format!("mission-drawer-ended-{session_id}"),
                        chat_lifecycle::shell_exited_subtitle(
                            exit_code,
                            &default_session_label(&entry),
                            entry.cwd.as_deref(),
                            runner_backend::app_paths::home_dir()
                                .as_deref()
                                .and_then(|home| home.to_str()),
                        ),
                        move |window, cx| {
                            restart_root.update(cx, |this, cx| {
                                this.resume_terminal_drawer_shell(&restart_id, window, cx)
                            });
                        },
                        move |window, cx| {
                            close_root.update(cx, |this, cx| {
                                this.close_terminal_drawer_shell(&close_id, window, cx)
                            });
                        },
                    )
                    .into_any_element(),
                )
            }
            chat_lifecycle::PaneOverlayState::Archiving
            | chat_lifecycle::PaneOverlayState::None => None,
        };
        let focus_id = session_id.clone();
        Some(
            div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .bg(terminal_background)
                .child(terminal_surface)
                .children(overlay_element)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.focus_mission_drawer_terminal(&focus_id, window, cx);
                    }),
                )
                .into_any_element(),
        )
    }

    fn render_loaded_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let feed_active = self.secondary || self.active_tab == MissionTab::Feed;
        let tabs = self.render_mission_tabs(feed_active, cx);
        let pane = if feed_active {
            self.render_mission_feed_surface(window, cx)
        } else {
            match &self.active_tab {
                MissionTab::Session(session_id) => {
                    let session = self
                        .sessions
                        .iter()
                        .find(|session| session.session.id == *session_id)
                        .cloned();
                    session
                        .map(|session| self.render_mission_terminal_pane(session, window, cx))
                        .unwrap_or_else(|| self.render_mission_feed_surface(window, cx))
                }
                MissionTab::Feed => self.render_mission_feed_surface(window, cx),
            }
        };
        let mut panes = div()
            .relative()
            .min_h(px(0.))
            .flex_1()
            .overflow_hidden()
            .child(pane);
        if self.archiving {
            panes = panes.child(SessionOverlay::transition(
                "mission-archiving",
                SessionOverlayKind::Archiving,
            ));
        }
        if self.secondary && !self.duplicate_dismissed {
            panes = panes.child(self.render_duplicate_mission_overlay(cx));
        }
        let drawer = (mission_drawer_available(self.archived(), self.secondary)
            && self.layout.drawer.open())
        .then(|| self.render_mission_terminal_drawer(cx));
        let drawer_height = self.layout.drawer.height();
        let drawer_resizing = self.drawer_resizing;
        div()
            .min_h(px(0.))
            .flex_1()
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .min_h(px(0.))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(tabs)
                    .child(panes),
            )
            .children(drawer.map(|drawer| {
                div()
                    .flex_none()
                    .h(rems(drawer_height / 16.))
                    .min_h(rems(MIN_DRAWER_HEIGHT / 16.))
                    .max_h(rems(MAX_DRAWER_HEIGHT / 16.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("mission-terminal-drawer-resize")
                            .flex_none()
                            .h(rems(5. / 16.))
                            .w_full()
                            .flex()
                            .items_center()
                            .cursor(CursorStyle::ResizeUpDown)
                            .child(
                                div()
                                    .h(rems(if drawer_resizing { 2. / 16. } else { 1. / 16. }))
                                    .w_full()
                                    .bg(theme::border_strong()),
                            )
                            .on_drag(
                                DrawerResizeDrag,
                                |drag: &DrawerResizeDrag, _, _, cx: &mut App| {
                                    cx.new(|_| drag.clone())
                                },
                            ),
                    )
                    .child(drawer)
            }))
            .on_drag_move::<DrawerResizeDrag>(cx.listener(
                |this, event: &DragMoveEvent<DrawerResizeDrag>, _, cx| {
                    this.resize_terminal_drawer(event, cx);
                },
            ))
            .on_drop(cx.listener(|this, _: &DrawerResizeDrag, _, cx| {
                this.finish_terminal_drawer_resize(cx);
            }))
            .into_any_element()
    }

    fn render_mission_feed_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mission_running = self
            .mission
            .as_ref()
            .is_some_and(|mission| mission.status == MissionStatus::Running);
        let can_compose = mission_running && !self.archived() && !self.secondary;
        let paused = mission_running
            && !self.any_session_live()
            && self.slot_actions.is_empty()
            && !self.resuming
            && !self.archiving
            && !self.secondary;
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .child(self.render_mission_feed(cx))
            .children(can_compose.then(|| self.render_mission_composer(window, cx)))
            .children(paused.then(|| self.render_mission_paused_overlay(cx)))
            .into_any_element()
    }
}
