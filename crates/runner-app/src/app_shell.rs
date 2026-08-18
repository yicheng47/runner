use std::time::Duration;

use gpui::{deferred, svg, BoxShadow, FontWeight, WindowAppearance, WindowControlArea};

use super::*;
use crate::app_settings::{clamp_sidebar_width, nudge_zoom};
use crate::toast::ToastTone;

const TITLEBAR_HEIGHT: f32 = 44.;
const TITLEBAR_DRAG_HEIGHT: f32 = 28.;
const SIDEBAR_TOGGLE_GLYPH_X: f32 = 94.3;
const SIDEBAR_TOGGLE_GLYPH_INSET: f32 = 6.3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AppRoute {
    #[default]
    Chat,
    Settings,
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
        self.sync_theme(window);
        window.set_rem_size(px(16. * self.settings.app_zoom));
        if self.bridge.take_session_refresh() {
            self.refresh_sessions();
        }
        if let Some(error) = self.error.take() {
            self.show_toast(error, ToastTone::Error, cx);
        }

        let workspace = self.render_active_tab(window, cx);
        let sidebar = self.render_app_sidebar(window, cx);
        let preview_trigger = self.render_sidebar_preview_trigger(cx);
        let modal = (self.route != AppRoute::Settings)
            .then_some(self.start_chat_modal.as_ref())
            .flatten()
            .map(|_| deferred(self.render_start_chat_modal(cx)).with_priority(2));
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
                                .h(px(TITLEBAR_DRAG_HEIGHT * self.settings.app_zoom)),
                            cx,
                        ),
                    )
                    .child(workspace),
            )
            .children(preview_trigger)
            .children(modal);
        let settings =
            (self.route == AppRoute::Settings).then(|| self.render_settings_takeover(window, cx));
        let toast = self
            .render_toast(cx)
            .map(|toast| deferred(toast).with_priority(3));

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .track_focus(&self.root_focus)
            .font(self.settings.app_font_family.font())
            .bg(theme::bg())
            .text_color(theme::text())
            .child(chrome)
            .children(settings)
            .children(toast)
            .on_drag_move::<SidebarResizeDrag>(cx.listener(
                |this, event: &DragMoveEvent<SidebarResizeDrag>, _, cx| {
                    let width = f32::from(event.event.position.x - event.bounds.left())
                        / this.settings.app_zoom;
                    this.settings.sidebar_width = clamp_sidebar_width(width);
                    cx.notify();
                },
            ))
            .on_drop(cx.listener(|this, _: &SidebarResizeDrag, _, _| {
                this.save_settings();
            }))
            .on_action(cx.listener(Self::open_new_tab_modal))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .on_action(cx.listener(Self::toggle_fullscreen))
            .into_any_element()
    }

    fn render_app_sidebar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let visible = !self.settings.sidebar_collapsed || self.sidebar_preview_open;
        if !visible || (self.route == AppRoute::Settings && self.settings.sidebar_collapsed) {
            return None;
        }
        let preview = self.settings.sidebar_collapsed && self.sidebar_preview_open;
        let width = self.settings.sidebar_width * self.settings.app_zoom;
        let fullscreen = window.is_fullscreen();
        let titlebar_padding = if fullscreen {
            8. * self.settings.app_zoom
        } else {
            SIDEBAR_TOGGLE_GLYPH_X - SIDEBAR_TOGGLE_GLYPH_INSET * self.settings.app_zoom
        };
        let panel_path = if self.settings.sidebar_collapsed {
            "panel-left-hollow.svg"
        } else {
            "panel-left-filled.svg"
        };
        let titlebar = self.render_titlebar_drag_area(
            "sidebar-titlebar-drag",
            div()
                .flex_none()
                .h(px(TITLEBAR_HEIGHT * self.settings.app_zoom))
                .pl(px(titlebar_padding))
                .pr_3()
                .flex()
                .items_center()
                .child(
                    div()
                        .id("sidebar-toggle")
                        .group("sidebar-toggle")
                        .flex_none()
                        .w(px(28. * self.settings.app_zoom))
                        .h(px(28. * self.settings.app_zoom))
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
                                .w(px(15.4 * self.settings.app_zoom))
                                .h(px(12. * self.settings.app_zoom))
                                .flex_none()
                                .text_color(theme::muted())
                                .group_hover("sidebar-toggle", |icon| {
                                    icon.text_color(theme::text())
                                }),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if this.settings.sidebar_collapsed {
                                this.settings.sidebar_collapsed = false;
                                this.sidebar_preview_open = false;
                            } else {
                                this.settings.sidebar_collapsed = true;
                            }
                            this.save_settings();
                            cx.notify();
                        })),
                ),
            cx,
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
                    .w(px(32. * self.settings.app_zoom))
                    .h(px(32. * self.settings.app_zoom))
                    .text_color(theme::accent()),
            )
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child("Runner"),
            );
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
                    .hover(|button| {
                        button
                            .border_color(theme::sidebar_selected_border())
                            .bg(alpha(theme::sidebar_selected(), 0.4))
                            .text_color(theme::text())
                    })
                    .child(
                        svg()
                            .path("settings.svg")
                            .w(px(14. * self.settings.app_zoom))
                            .h(px(14. * self.settings.app_zoom))
                            .flex_none()
                            .text_color(theme::muted())
                            .group_hover("sidebar-settings", |icon| icon.text_color(theme::text())),
                    )
                    .child(
                        div()
                            .text_size(px(13. * self.settings.app_zoom))
                            .child("Settings"),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.enter_settings(window, cx);
                    })),
            );
        let resize_handle = self.render_sidebar_resize_handle();
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
            .bg(theme::sidebar())
            .border_r_1()
            .border_color(theme::border())
            .child(titlebar)
            .child(brand)
            .child(self.render_sidebar_contents(cx))
            .child(settings_button)
            .child(resize_handle);
        if preview {
            sidebar = sidebar
                .absolute()
                .left_0()
                .top_0()
                .rounded_tr(px(12. * self.settings.app_zoom))
                .rounded_br(px(12. * self.settings.app_zoom))
                .shadow_2xl()
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if !*hovered && this.settings.sidebar_collapsed {
                        this.sidebar_preview_open = false;
                        cx.notify();
                    }
                }));
            return Some(deferred(sidebar).with_priority(1).into_any_element());
        }
        Some(sidebar.into_any_element())
    }

    fn render_sidebar_resize_handle(&self) -> AnyElement {
        div()
            .id("sidebar-resize")
            .absolute()
            .right_0()
            .top_0()
            .w(px(4. * self.settings.app_zoom))
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
        (self.settings.sidebar_collapsed && self.route != AppRoute::Settings).then(|| {
            div()
                .id("sidebar-preview-trigger")
                .absolute()
                .left_0()
                .top_0()
                .w(px(16. * self.settings.app_zoom))
                .h_full()
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.sidebar_preview_open = true;
                        cx.notify();
                    }
                }))
                .into_any_element()
        })
    }

    pub(crate) fn render_open_sidebar_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.settings.sidebar_collapsed.then(|| {
            div()
                .id("open-sidebar")
                .group("open-sidebar")
                .flex_none()
                .w(px(28. * self.settings.app_zoom))
                .h(px(28. * self.settings.app_zoom))
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
                        .w(px(15.4 * self.settings.app_zoom))
                        .h(px(12. * self.settings.app_zoom))
                        .flex_none()
                        .text_color(theme::muted())
                        .group_hover("open-sidebar", |icon| icon.text_color(theme::text())),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.settings.sidebar_collapsed = false;
                    this.sidebar_preview_open = false;
                    this.save_settings();
                    this.focus_active_terminal(window);
                    cx.notify();
                }))
                .into_any_element()
        })
    }

    fn render_settings_takeover(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let width = self.settings.sidebar_width * self.settings.app_zoom;
        div()
            .absolute()
            .inset_0()
            .flex()
            .overflow_hidden()
            .bg(theme::bg())
            .occlude()
            .child(
                div()
                    .relative()
                    .w(px(width))
                    .h_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .bg(theme::sidebar())
                    .border_r_1()
                    .border_color(theme::border())
                    .child(self.render_titlebar_drag_area(
                        "settings-sidebar-titlebar-drag",
                        div().h(px(32. * self.settings.app_zoom)).flex_none(),
                        cx,
                    ))
                    .child(
                        div().px_4().pb_3().pt_1().child(
                            div()
                                .id("settings-back")
                                .group("settings-back")
                                .px_2()
                                .py(rems(6. / 16.))
                                .flex()
                                .items_center()
                                .gap_2()
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
                                .child(
                                    svg()
                                        .path("arrow-left.svg")
                                        .w(px(14. * self.settings.app_zoom))
                                        .h(px(14. * self.settings.app_zoom))
                                        .flex_none()
                                        .text_color(theme::muted())
                                        .group_hover("settings-back", |icon| {
                                            icon.text_color(theme::text())
                                        }),
                                )
                                .child(
                                    div()
                                        .text_size(px(13. * self.settings.app_zoom))
                                        .child("Back to app"),
                                )
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.leave_settings(window, cx);
                                })),
                        ),
                    )
                    .child(self.render_sidebar_resize_handle()),
            )
            .child(
                div().relative().flex_1().h_full().bg(theme::bg()).child(
                    self.render_titlebar_drag_area(
                        "settings-content-titlebar-drag",
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h(px(TITLEBAR_DRAG_HEIGHT * self.settings.app_zoom)),
                        cx,
                    ),
                ),
            )
            .into_any_element()
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
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
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
                .top(px(20. * self.settings.app_zoom))
                .left(px(16. * self.settings.app_zoom))
                .right(px(16. * self.settings.app_zoom))
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("global-toast")
                        .w_full()
                        .max_w(px(420. * self.settings.app_zoom))
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
                            offset: point(px(0.), px(8. * self.settings.app_zoom)),
                            blur_radius: px(24. * self.settings.app_zoom),
                            spread_radius: px(0.),
                        }])
                        .text_sm()
                        .text_color(text)
                        .child(
                            div()
                                .min_w(px(0.))
                                .flex_1()
                                .whitespace_normal()
                                .line_height(px(20. * self.settings.app_zoom))
                                .child(SharedString::from(toast.message.clone())),
                        )
                        .child(
                            div()
                                .id("dismiss-toast")
                                .mt(rems(2. / 16.))
                                .w(px(20. * self.settings.app_zoom))
                                .h(px(20. * self.settings.app_zoom))
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
                                        .w(px(14. * self.settings.app_zoom))
                                        .h(px(14. * self.settings.app_zoom))
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

    pub(crate) fn terminal_style(&self) -> terminal_element::TerminalStyle {
        terminal_element::TerminalStyle {
            palette: self.settings.terminal_theme.palette(),
            font_family: self.settings.terminal_font_family.family().into(),
            font_size: self.settings.terminal_font_size as f32 * self.settings.app_zoom,
            app_zoom: self.settings.app_zoom,
        }
    }

    pub(crate) fn workspace_titlebar_padding(&self, window: &Window) -> f32 {
        if self.settings.sidebar_collapsed && !window.is_fullscreen() {
            SIDEBAR_TOGGLE_GLYPH_X - SIDEBAR_TOGGLE_GLYPH_INSET * self.settings.app_zoom
        } else {
            16. * self.settings.app_zoom
        }
    }

    fn sync_theme(&self, window: &Window) {
        let system_is_light = matches!(
            window.appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        );
        theme::set_active_variant(theme::resolve_variant(
            self.settings.app_theme,
            system_is_light,
            self.settings.light_app_theme,
            self.settings.dark_app_theme,
        ));
    }

    fn save_settings(&self) {
        if let Err(error) = self.settings.save(&self.settings_path) {
            eprintln!("Runner UI settings save failed: {error:#}");
        }
    }

    fn enter_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.route != AppRoute::Settings {
            self.route = AppRoute::Settings;
            window.focus(&self.root_focus);
            cx.notify();
        }
    }

    fn leave_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.route = AppRoute::Chat;
        self.focus_active_terminal(window);
        cx.notify();
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        if self.route == AppRoute::Settings {
            return;
        }
        self.settings.sidebar_collapsed = !self.settings.sidebar_collapsed;
        if !self.settings.sidebar_collapsed {
            self.sidebar_preview_open = false;
        }
        self.save_settings();
        cx.notify();
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        self.enter_settings(window, cx);
    }

    fn zoom_in(&mut self, _: &ZoomIn, window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(nudge_zoom(self.settings.app_zoom, 1), window, cx);
    }

    fn zoom_out(&mut self, _: &ZoomOut, window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(nudge_zoom(self.settings.app_zoom, -1), window, cx);
    }

    fn zoom_reset(&mut self, _: &ZoomReset, window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(1., window, cx);
    }

    fn set_zoom(&mut self, zoom: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.app_zoom = zoom;
        window.set_rem_size(px(16. * zoom));
        mac_chrome::sync_traffic_lights(window, zoom);
        self.save_settings();
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
