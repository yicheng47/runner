use gpui::{deferred, svg, BoxShadow, FontWeight, WindowAppearance, WindowControlArea};

use crate::app_settings::{clamp_sidebar_width, nudge_zoom};
use crate::toast::ToastTone;
use crate::*;

const TITLEBAR_HEIGHT: f32 = 44.;
pub(crate) const TITLEBAR_DRAG_HEIGHT: f32 = 28.;
pub(crate) const SIDEBAR_TOGGLE_GLYPH_X: f32 = 94.3;
pub(crate) const SIDEBAR_TOGGLE_GLYPH_INSET: f32 = 6.3;
const SIDEBAR_TRANSITION_MS: u64 = 200;
// Deliberately differs from main's inherited 19.5px line box to align both footer dividers.
const SETTINGS_FOOTER_LINE_HEIGHT: f32 = 18.;
const WINDOW_SIZE_SAVE_DELAY_MS: u64 = 300;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum AppRoute {
    #[default]
    Chat,
    Runners,
    RunnerDetail(String),
    Crews,
    CrewEditor(String),
    Mission(String),
    ArchivedChat,
    Settings,
}

impl AppRoute {
    pub(crate) fn terminal_visible(&self) -> bool {
        matches!(self, Self::Chat)
    }
}

#[derive(Clone)]
struct SidebarResizeDrag;

impl Render for SidebarResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1.)).h(px(1.))
    }
}

fn alpha(mut color: gpui::Hsla, value: f32) -> gpui::Hsla {
    color.a = value;
    color
}

impl NativeRoot {
    pub(crate) fn render_app_shell(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_theme(window, cx);
        window.set_rem_size(px(16. * self.settings(cx).app_zoom));
        if self.route == AppRoute::Chat {
            let needs_attach = self.tabs.active().is_some_and(|layout| {
                layout
                    .session_ids()
                    .iter()
                    .any(|session_id| !self.attached.contains_key(session_id))
            });
            if needs_attach {
                if let Err(error) = self.ensure_active_tab_attached(window, cx) {
                    self.error = Some(error.to_string());
                } else {
                    self.mark_active_tab_viewed(window, cx);
                }
            }
        }
        if let Some(error) = self.error.take() {
            self.show_toast(error, ToastTone::Error, cx);
        }

        let workspace = self.render_entity_surface(window, cx);
        let sidebar = self.render_app_sidebar(window, cx);
        let preview_trigger = self.render_sidebar_preview_trigger(cx);
        let modal = (self.route != AppRoute::Settings)
            .then_some(self.start_chat_modal.as_ref())
            .flatten()
            .map(|_| self.render_start_chat_modal(cx));
        let chat_rename_modal = (self.route == AppRoute::Chat)
            .then_some(self.chat_rename_modal.as_ref())
            .flatten()
            .map(|_| self.render_chat_rename_modal(cx));
        let sidebar_overlays = if self.route != AppRoute::Settings {
            self.render_sidebar_overlays(cx)
        } else {
            Vec::new()
        };
        let entity_overlays = if self.route != AppRoute::Settings {
            self.render_entity_overlays(cx)
        } else {
            Vec::new()
        };
        let chrome = div()
            .relative()
            .size_full()
            .flex()
            .children(sidebar)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        self.render_titlebar_drag_area(
                            "main-titlebar-drag",
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .h(px(TITLEBAR_DRAG_HEIGHT * self.settings(cx).app_zoom)),
                            cx,
                        ),
                    )
                    .child(workspace),
            )
            .children(preview_trigger)
            .children(modal)
            .children(chat_rename_modal)
            .children(sidebar_overlays)
            .children(entity_overlays);
        let settings =
            (self.route == AppRoute::Settings).then(|| self.render_settings_takeover(window, cx));
        let settings_confirm = self
            .settings_confirm_overlay(cx)
            .map(|overlay| deferred(overlay).with_priority(100));
        let command_palette = deferred(self.command_palette.clone()).with_priority(50);
        let toast = self
            .render_toast(cx)
            .map(|toast| deferred(toast).with_priority(3));

        div()
            .relative()
            .key_context("Root")
            .size_full()
            .overflow_hidden()
            .track_focus(&self.root_focus)
            .font(self.settings(cx).app_font_family.font())
            .bg(theme::bg())
            .text_color(theme::text())
            .child(chrome)
            .children(settings)
            .children(settings_confirm)
            .child(command_palette)
            .children(toast)
            .on_drag_move::<SidebarResizeDrag>(cx.listener(
                |this, event: &DragMoveEvent<SidebarResizeDrag>, _, cx| {
                    let width = f32::from(event.event.position.x - event.bounds.left())
                        / this.settings(cx).app_zoom;
                    let width = clamp_sidebar_width(width);
                    this.update_app_settings(cx, false, |settings| {
                        if settings.sidebar_width == width {
                            return false;
                        }
                        settings.sidebar_width = width;
                        true
                    });
                },
            ))
            .on_drop(cx.listener(|this, _: &SidebarResizeDrag, _, cx| {
                this.save_settings(cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.clear_sidebar_drag(cx);
                    this.clear_crew_slot_drag(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.clear_sidebar_drag(cx);
                    this.clear_crew_slot_drag(cx);
                }),
            )
            .on_action(cx.listener(Self::open_new_tab_modal))
            .on_action(cx.listener(Self::close_focused_chat_pane))
            .on_action(cx.listener(Self::focus_previous_chat_pane))
            .on_action(cx.listener(Self::focus_next_chat_pane))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::open_command_palette))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::navigate_previous_page))
            .on_action(cx.listener(Self::navigate_next_page))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .on_action(cx.listener(Self::toggle_fullscreen))
            .into_any_element()
    }

    fn render_app_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let visible = !self.settings(cx).sidebar_collapsed || self.sidebar_preview_open;
        let visibility_target = if visible { 1. } else { 0. };
        let (visibility, animating) = self.sidebar_visibility.animate_to(
            visibility_target,
            Instant::now(),
            Duration::from_millis(SIDEBAR_TRANSITION_MS),
        );
        if animating {
            window.request_animation_frame();
        }
        let show_panel = visible || animating;
        let full_width = self.settings(cx).sidebar_width * self.settings(cx).app_zoom;
        let width = full_width * visibility;
        if !show_panel {
            return Some(
                div()
                    .id("app-sidebar")
                    .relative()
                    .w(px(width))
                    .h_full()
                    .flex_none()
                    .overflow_hidden()
                    .into_any_element(),
            );
        }
        let preview = self.settings(cx).sidebar_collapsed
            && (self.sidebar_preview_open || self.sidebar_preview_peeking);
        let fullscreen = window.is_fullscreen();
        let titlebar_padding = if fullscreen {
            8. * self.settings(cx).app_zoom
        } else {
            SIDEBAR_TOGGLE_GLYPH_X - SIDEBAR_TOGGLE_GLYPH_INSET * self.settings(cx).app_zoom
        };
        let panel_path = if self.settings(cx).sidebar_collapsed {
            "panel-left-hollow.svg"
        } else {
            "panel-left-filled.svg"
        };
        let titlebar = self.render_titlebar_drag_area(
            "sidebar-titlebar-drag",
            div()
                .flex_none()
                .h(px(TITLEBAR_HEIGHT * self.settings(cx).app_zoom))
                .pl(px(titlebar_padding))
                .pr_3()
                .flex()
                .items_center()
                .child(
                    div()
                        .id("sidebar-toggle")
                        .group("sidebar-toggle")
                        .flex_none()
                        .w(px(28. * self.settings(cx).app_zoom))
                        .h(px(28. * self.settings(cx).app_zoom))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_sm()
                        .cursor_pointer()
                        .text_color(theme::muted())
                        .hover(|button| {
                            button
                                .bg(alpha(theme::sidebar_selected(), 0.6))
                                .text_color(theme::text())
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            svg()
                                .path(panel_path)
                                .w(px(15.4 * self.settings(cx).app_zoom))
                                .h(px(12. * self.settings(cx).app_zoom))
                                .flex_none()
                                .text_color(theme::muted())
                                .group_hover("sidebar-toggle", |icon| {
                                    icon.text_color(theme::text())
                                }),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if this.settings(cx).sidebar_collapsed {
                                this.update_app_settings(cx, true, |settings| {
                                    settings.sidebar_collapsed = false;
                                    true
                                });
                                this.sidebar_preview_open = false;
                                this.sidebar_preview_peeking = false;
                            } else {
                                this.update_app_settings(cx, true, |settings| {
                                    settings.sidebar_collapsed = true;
                                    true
                                });
                                this.sidebar_preview_peeking = false;
                            }
                            cx.notify();
                        })),
                ),
            cx,
        );
        let search_button = div()
            .id("sidebar-search")
            .group("sidebar-search")
            .tab_index(0)
            .flex_none()
            .size(px(24. * self.settings(cx).app_zoom))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .border_1()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .text_color(theme::muted())
            .hover(|button| {
                button
                    .border_color(theme::sidebar_selected_border())
                    .bg(alpha(theme::sidebar_selected(), 0.4))
                    .text_color(theme::text())
            })
            .focus_visible(|button| {
                button
                    .border_color(theme::sidebar_selected_border())
                    .bg(alpha(theme::sidebar_selected(), 0.4))
                    .text_color(theme::text())
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.show_command_palette(window, cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    this.show_command_palette(window, cx);
                }
            }))
            .child(
                svg()
                    .path("search.svg")
                    .size(px(14. * self.settings(cx).app_zoom))
                    .text_color(theme::muted())
                    .group_hover("sidebar-search", |icon| icon.text_color(theme::text())),
            );
        let brand = div()
            .flex_none()
            .px_5()
            .pb_5()
            .pt_1()
            .flex()
            .items_center()
            .gap_2()
            .child(
                svg()
                    .path("brand-mark.svg")
                    .w(px(32. * self.settings(cx).app_zoom))
                    .h(px(32. * self.settings(cx).app_zoom))
                    .text_color(theme::accent()),
            )
            .child(
                div()
                    .flex_1()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child("Runner"),
            )
            .child(Tooltip::new(
                "sidebar-search-tooltip",
                "Search (⌘K)",
                search_button,
            ));
        let settings_button = div()
            .flex_none()
            .px_3()
            .pt_2()
            .border_t_1()
            .border_color(theme::sidebar_selected_border())
            .child(
                div()
                    .id("open-settings")
                    .group("sidebar-settings")
                    .w_full()
                    .px(rems(10. / 16.))
                    .py_2()
                    .flex()
                    .items_center()
                    .gap(rems(10. / 16.))
                    .rounded_sm()
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .cursor_pointer()
                    .text_color(theme::muted())
                    .line_height(px(SETTINGS_FOOTER_LINE_HEIGHT * self.settings(cx).app_zoom))
                    .hover(|button| {
                        button
                            .border_color(theme::sidebar_selected_border())
                            .bg(alpha(theme::sidebar_selected(), 0.4))
                            .text_color(theme::text())
                    })
                    .child(
                        svg()
                            .path("settings.svg")
                            .w(px(14. * self.settings(cx).app_zoom))
                            .h(px(14. * self.settings(cx).app_zoom))
                            .flex_none()
                            .text_color(theme::muted())
                            .group_hover("sidebar-settings", |icon| icon.text_color(theme::text())),
                    )
                    .child(
                        div()
                            .text_size(px(13. * self.settings(cx).app_zoom))
                            .child("Settings"),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.enter_settings_route(None, window, cx);
                    })),
            );
        let resize_handle = visible.then(|| self.render_sidebar_resize_handle(cx));
        let mut sidebar = div()
            .id("app-sidebar")
            .relative()
            .w(px(width))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .pb_3()
            .overflow_hidden()
            .opacity(visibility)
            .bg(theme::sidebar())
            .border_r_1()
            .border_color(theme::border())
            .child(titlebar)
            .child(brand)
            .child(self.sidebar.clone())
            .child(settings_button)
            .children(resize_handle);
        if preview {
            sidebar = sidebar
                .absolute()
                .left_0()
                .top_0()
                .rounded_tr(px(12. * self.settings(cx).app_zoom))
                .rounded_br(px(12. * self.settings(cx).app_zoom))
                .shadow_2xl()
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if !*hovered && this.settings(cx).sidebar_collapsed {
                        this.sidebar_preview_open = false;
                        cx.notify();
                    }
                }));
            return Some(deferred(sidebar).with_priority(1).into_any_element());
        }
        Some(sidebar.into_any_element())
    }

    pub(crate) fn render_sidebar_resize_handle(&self, cx: &App) -> AnyElement {
        div()
            .id("sidebar-resize")
            .absolute()
            .right_0()
            .top_0()
            .w(px(4. * self.settings(cx).app_zoom))
            .h_full()
            .cursor(CursorStyle::ResizeLeftRight)
            .hover(|handle| handle.bg(alpha(theme::accent(), 0.4)))
            .on_drag(
                SidebarResizeDrag,
                |drag: &SidebarResizeDrag, _, _, cx: &mut App| cx.new(|_| drag.clone()),
            )
            .into_any_element()
    }

    fn render_sidebar_preview_trigger(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        (self.settings(cx).sidebar_collapsed && self.route != AppRoute::Settings).then(|| {
            div()
                .id("sidebar-preview-trigger")
                .absolute()
                .left_0()
                .top_0()
                .w(px(16. * self.settings(cx).app_zoom))
                .h_full()
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.sidebar_preview_open = true;
                        this.sidebar_preview_peeking = true;
                        cx.notify();
                    }
                }))
                .into_any_element()
        })
    }

    pub(crate) fn render_open_sidebar_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.settings(cx).sidebar_collapsed.then(|| {
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
                        .path("panel-left-hollow.svg")
                        .w(px(15.4 * self.settings(cx).app_zoom))
                        .h(px(12. * self.settings(cx).app_zoom))
                        .flex_none()
                        .text_color(theme::muted())
                        .group_hover("open-sidebar", |icon| icon.text_color(theme::text())),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.update_app_settings(cx, true, |settings| {
                        settings.sidebar_collapsed = false;
                        true
                    });
                    this.sidebar_preview_open = false;
                    this.sidebar_preview_peeking = false;
                    this.focus_active_terminal(window, cx);
                    cx.notify();
                }))
                .into_any_element()
        })
    }

    pub(crate) fn render_titlebar_drag_area(
        &self,
        id: &'static str,
        area: gpui::Div,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        area.id(id)
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
            )
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
            })
    }

    fn render_toast(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.toasts.active().map(|toast| {
            let (border, text) = match toast.tone {
                ToastTone::Info => (theme::border_strong(), theme::text()),
                ToastTone::Success => (alpha(theme::accent(), 0.4), theme::accent()),
                ToastTone::Error => (alpha(theme::danger(), 0.4), theme::danger()),
            };
            div()
                .absolute()
                .top(px(20. * self.settings(cx).app_zoom))
                .left(px(16. * self.settings(cx).app_zoom))
                .right(px(16. * self.settings(cx).app_zoom))
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("global-toast")
                        .w_full()
                        .max_w(px(420. * self.settings(cx).app_zoom))
                        .px_4()
                        .py_3()
                        .flex()
                        .items_start()
                        .gap_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(border)
                        .bg(theme::panel())
                        .shadow(vec![BoxShadow {
                            color: gpui::hsla(0., 0., 0., 0.5),
                            offset: point(px(0.), px(8. * self.settings(cx).app_zoom)),
                            blur_radius: px(24. * self.settings(cx).app_zoom),
                            spread_radius: px(0.),
                        }])
                        .text_sm()
                        .text_color(text)
                        .child(
                            div()
                                .min_w(px(0.))
                                .flex_1()
                                .whitespace_normal()
                                .line_height(px(20. * self.settings(cx).app_zoom))
                                .child(SharedString::from(toast.message.clone())),
                        )
                        .child(
                            div()
                                .id("dismiss-toast")
                                .mt(rems(2. / 16.))
                                .w(px(20. * self.settings(cx).app_zoom))
                                .h(px(20. * self.settings(cx).app_zoom))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_sm()
                                .cursor_pointer()
                                .opacity(0.7)
                                .hover(|button| button.opacity(1.))
                                .child(
                                    svg()
                                        .path("close.svg")
                                        .w(px(14. * self.settings(cx).app_zoom))
                                        .h(px(14. * self.settings(cx).app_zoom))
                                        .text_color(text),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toasts.dismiss();
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element()
        })
    }

    pub(crate) fn show_toast(
        &mut self,
        message: impl Into<String>,
        tone: ToastTone,
        cx: &mut Context<Self>,
    ) {
        let id = self.toasts.show(message, tone);
        let duration_ms = self.toasts.active().and_then(|toast| toast.duration_ms);
        cx.notify();
        let Some(duration_ms) = duration_ms else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(duration_ms))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.toasts.expire(id) {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn terminal_style(&self, cx: &App) -> crate::terminal::element::TerminalStyle {
        crate::terminal::element::TerminalStyle {
            palette: self.settings(cx).terminal_theme.palette(),
            font_family: self.settings(cx).terminal_font_family.family().into(),
            font_size: self.settings(cx).terminal_font_size as f32 * self.settings(cx).app_zoom,
            app_zoom: self.settings(cx).app_zoom,
        }
    }

    pub(crate) fn workspace_titlebar_padding(&self, window: &Window, cx: &App) -> f32 {
        if self.settings(cx).sidebar_collapsed && !window.is_fullscreen() {
            SIDEBAR_TOGGLE_GLYPH_X - SIDEBAR_TOGGLE_GLYPH_INSET * self.settings(cx).app_zoom
        } else {
            16. * self.settings(cx).app_zoom
        }
    }

    fn sync_theme(&self, window: &Window, cx: &App) {
        let system_is_light = matches!(
            window.appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        );
        theme::set_active_variant(theme::resolve_variant(
            self.settings(cx).app_theme,
            system_is_light,
            self.settings(cx).light_app_theme,
            self.settings(cx).dark_app_theme,
        ));
    }

    pub(crate) fn save_settings(&self, cx: &App) {
        self.app_store.read(cx).save_settings();
    }

    pub(crate) fn update_app_settings(
        &self,
        cx: &mut Context<Self>,
        persist: bool,
        update: impl FnOnce(&mut AppSettings) -> bool,
    ) -> bool {
        self.app_store.update(cx, |store, store_cx| {
            store.update_settings(update, persist, store_cx)
        })
    }

    pub(crate) fn schedule_window_size_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.window_size_save_generation = self.window_size_save_generation.wrapping_add(1);
        let generation = self.window_size_save_generation;
        cx.spawn_in(window, async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(WINDOW_SIZE_SAVE_DELAY_MS))
                .await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.window_size_save_generation == generation {
                    this.save_window_size(window, cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn save_window_size(&mut self, window: &Window, cx: &mut Context<Self>) {
        if window.is_fullscreen() || window.is_maximized() {
            return;
        }
        let size = window.viewport_size();
        let width = f32::from(size.width);
        let height = f32::from(size.height);
        if self.settings(cx).window_width == width && self.settings(cx).window_height == height {
            return;
        }
        self.update_app_settings(cx, true, |settings| {
            settings.window_width = width;
            settings.window_height = height;
            true
        });
    }

    pub(crate) fn leave_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shortcut_recording_active() {
            self.finish_shortcut_recording(window, cx);
        }
        self.save_settings(cx);
        match self.settings_return_route.clone() {
            AppRoute::Chat | AppRoute::Settings => {
                self.set_route(AppRoute::Chat, cx);
                self.mark_active_tab_viewed(window, cx);
                self.focus_active_terminal(window, cx);
                cx.notify();
            }
            AppRoute::Runners => self.open_runners(window, cx),
            AppRoute::RunnerDetail(handle) => self.open_runner_detail(handle, window, cx),
            AppRoute::Crews => self.open_crews(window, cx),
            AppRoute::CrewEditor(crew_id) => self.open_crew_editor(crew_id, window, cx),
            AppRoute::Mission(mission_id) => self.open_mission(mission_id, window, cx),
            AppRoute::ArchivedChat => {
                self.set_route(AppRoute::ArchivedChat, cx);
                self.chat_focus.focus(window);
                cx.notify();
            }
        }
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        if self.route == AppRoute::Settings {
            return;
        }
        let collapsed = !self.settings(cx).sidebar_collapsed;
        self.update_app_settings(cx, true, |settings| {
            settings.sidebar_collapsed = collapsed;
            true
        });
        if !collapsed {
            self.sidebar_preview_open = false;
        }
        self.sidebar_preview_peeking = false;
        cx.notify();
    }

    fn show_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.route == AppRoute::Settings {
            return;
        }
        self.command_palette
            .update(cx, |palette, palette_cx| palette.open(window, palette_cx));
    }

    fn open_command_palette(
        &mut self,
        _: &CommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_command_palette(window, cx);
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.route == AppRoute::Settings {
            return;
        }
        self.enter_settings_route(None, window, cx);
    }

    fn navigate_previous_page(
        &mut self,
        _: &NavigatePreviousPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_runtime_page(-1, window, cx);
    }

    fn navigate_next_page(
        &mut self,
        _: &NavigateNextPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_runtime_page(1, window, cx);
    }

    fn zoom_in(&mut self, _: &ZoomIn, window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(nudge_zoom(self.settings(cx).app_zoom, 1), window, cx);
    }

    fn zoom_out(&mut self, _: &ZoomOut, window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(nudge_zoom(self.settings(cx).app_zoom, -1), window, cx);
    }

    fn zoom_reset(&mut self, _: &ZoomReset, window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(1., window, cx);
    }

    pub(crate) fn set_zoom(&mut self, zoom: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.update_app_settings(cx, true, |settings| {
            if settings.app_zoom == zoom {
                return false;
            }
            settings.app_zoom = zoom;
            true
        });
        window.set_rem_size(px(16. * zoom));
        mac_chrome::sync_traffic_lights(window, zoom);
        cx.notify();
    }

    fn toggle_fullscreen(
        &mut self,
        _: &ToggleFullscreen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.toggle_fullscreen();
        cx.notify();
    }
}
