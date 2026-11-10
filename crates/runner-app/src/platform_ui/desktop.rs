use gpui::{deferred, svg, Div, WindowControlArea};

use crate::surfaces::app_shell::alpha;
use crate::*;
use runner_app::ui::CAPTION_BUTTON_WIDTH;

const TITLEBAR_HEIGHT: f32 = 32.;

// The chrome titlebar already reserves the band macOS Settings pads for, so
// the content starts at the same window height on both platforms.
pub(crate) const SETTINGS_DRAG_INSET: f32 = 0.;
pub(crate) const SETTINGS_CONTENT_TOP: f32 = 56. - TITLEBAR_HEIGHT;

pub(crate) const PRIMARY_MODIFIER: &str = "Ctrl";
#[cfg(windows)]
pub(crate) const SHORTCUT_REQUIREMENTS_KEY: &str =
    "ui.shortcuts_must_include_ctrl_alt_or_win_function_keys_can_be_used_alone";

#[cfg(target_os = "linux")]
pub(crate) const SHORTCUT_REQUIREMENTS_KEY: &str =
    "ui.shortcuts_must_include_ctrl_alt_or_super_function_keys_can_be_used_alone";

pub(crate) fn primary_modifier_held(modifiers: gpui::Modifiers) -> bool {
    modifiers.control
}

pub(crate) fn other_shortcut_modifiers_held(modifiers: gpui::Modifiers) -> bool {
    modifiers.platform || modifiers.alt || modifiers.shift || modifiers.function
}

pub(crate) fn sidebar_row_wrapper(id: SharedString) -> gpui::Stateful<Div> {
    div().id(id).relative().w_full().flex().flex_col()
}

pub(crate) fn sidebar_section() -> Div {
    div().flex_none().w_full().flex().flex_col()
}

pub(crate) fn sync_traffic_lights(_: &Window, _: f32) {}

pub(crate) fn finish_window_close(window: &mut Window) -> bool {
    // Let GPUI own HWND destruction instead of also running Windows' default WM_CLOSE handler.
    window.remove_window();
    false
}

pub(crate) fn navigation_left(_window: &Window, zoom: f32) -> f32 {
    16. * zoom
}

pub(crate) fn titlebar_mouse_down(
    event: &gpui::MouseDownEvent,
    _window: &mut Window,
    _cx: &mut App,
) {
    super::titlebar_double_click(event.click_count);
}

impl NativeRoot {
    pub(crate) fn decorate_window(
        &self,
        root: Div,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let frame = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::sidebar())
            .child(
                div()
                    .debug_selector(|| "WINDOW_CAPTION_BAR".into())
                    .relative()
                    .w_full()
                    .h(px(TITLEBAR_HEIGHT * self.settings(cx).app_zoom))
                    .flex_none()
                    .child(deferred(self.render_windows_titlebar(window, cx)).with_priority(200)),
            )
            .child(div().relative().w_full().flex_1().min_h(px(0.)).child(root));
        #[cfg(target_os = "linux")]
        let frame = super::linux::resize_frame(frame, window);
        frame
    }

    pub(crate) fn render_window_top_margin_drag_area(
        &self,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub(crate) fn render_main_titlebar_drag_area(
        &self,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub(crate) fn render_settings_titlebar_drag_area(
        &self,
        _id: &'static str,
        _area: Div,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub(crate) fn render_sidebar_titlebar(
        &self,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        Some(
            div()
                .flex_none()
                .h(px(12. * self.settings(cx).app_zoom))
                .into_any_element(),
        )
    }

    pub(crate) fn render_window_navigation(
        &self,
        _window: &Window,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    fn render_windows_navigation(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let zoom = self.settings(cx).app_zoom;
        let sidebar_root = cx.entity();
        let back_root = cx.entity();
        let forward_root = cx.entity();
        let in_settings = self.route == AppRoute::Settings;
        let (can_go_back, can_go_forward) = self.page_navigation_state();
        Some(
            div()
                .occlude()
                .debug_selector(|| "WINDOW_TITLEBAR_CONTROLS".into())
                .absolute()
                .left(px(navigation_left(window, zoom)))
                .top(px(2. * zoom))
                .flex_none()
                .h(px(28. * zoom))
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .debug_selector(|| "WINDOW_SIDEBAR_TOGGLE".into())
                        .child(
                            IconButton::new(
                                "sidebar-toggle",
                                if self.sidebar_collapsed {
                                    "panel-left-hidden.svg"
                                } else {
                                    "panel-left-open.svg"
                                },
                            )
                            .tooltip(crate::i18n::text("ui.toggle_sidebar"))
                            .disabled(in_settings)
                            .on_press(move |window, cx| {
                                sidebar_root.update(cx, |this, cx| {
                                    this.toggle_window_sidebar(window, cx);
                                });
                            }),
                        ),
                )
                .child(
                    div()
                        .debug_selector(|| "WINDOW_PREVIOUS_PAGE".into())
                        .child(
                            IconButton::new("window-previous-page", "chevron-left.svg")
                                .tooltip(crate::i18n::text("ui.previous_page"))
                                .disabled(!can_go_back)
                                .on_press(move |window, cx| {
                                    back_root.update(cx, |this, cx| {
                                        this.navigate_runtime_page(-1, window, cx)
                                    });
                                }),
                        ),
                )
                .child(
                    div().debug_selector(|| "WINDOW_NEXT_PAGE".into()).child(
                        IconButton::new("window-next-page", "chevron-right.svg")
                            .tooltip(crate::i18n::text("ui.next_page"))
                            .disabled(!can_go_forward)
                            .on_press(move |window, cx| {
                                forward_root.update(cx, |this, cx| {
                                    this.navigate_runtime_page(1, window, cx)
                                });
                            }),
                    ),
                )
                .into_any_element(),
        )
    }

    fn render_windows_titlebar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let zoom = self.settings(cx).app_zoom;
        let caption_width = if window.is_fullscreen() {
            0.
        } else {
            3. * CAPTION_BUTTON_WIDTH * zoom
        };
        div()
            .relative()
            .size_full()
            .flex()
            .items_center()
            .bg(theme::sidebar())
            .map(|titlebar| {
                #[cfg(target_os = "linux")]
                let titlebar = super::linux::round_window_corners(titlebar, window)
                    .rounded_bl(px(0.))
                    .rounded_br(px(0.));
                titlebar
            })
            .font(crate::app_settings::app_font())
            .child(
                self.render_titlebar_drag_area(
                    "windows-titlebar-drag",
                    div()
                        .flex_1()
                        .h_full()
                        .ml(px(128. * zoom))
                        .mr(px(caption_width))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12. * zoom))
                        .text_color(theme::muted())
                        .child("Runner"),
                    cx,
                ),
            )
            .children(self.render_windows_navigation(window, cx))
            .children(self.render_caption_buttons(window, cx))
            .into_any_element()
    }

    pub(crate) fn render_caption_buttons(&self, window: &Window, cx: &App) -> Option<AnyElement> {
        if window.is_fullscreen() {
            return None;
        }
        let zoom = self.settings(cx).app_zoom;
        let maximize_icon = if window.is_maximized() {
            "copy.svg"
        } else {
            "square.svg"
        };
        Some(
            div()
                .debug_selector(|| "WINDOW_CAPTION_CONTROLS".into())
                .absolute()
                .top_0()
                .right_0()
                .h(px(TITLEBAR_HEIGHT * zoom))
                .flex()
                .children(
                    [
                        ("caption-minimize", WindowControlArea::Min, "minus.svg"),
                        ("caption-maximize", WindowControlArea::Max, maximize_icon),
                        ("caption-close", WindowControlArea::Close, "close.svg"),
                    ]
                    .into_iter()
                    .map(|(id, area, icon)| {
                        let close = matches!(area, WindowControlArea::Close);
                        div()
                            .id(id)
                            .group(id)
                            .w(px(CAPTION_BUTTON_WIDTH * zoom))
                            .h_full()
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .occlude()
                            .window_control_area(area)
                            .map(|button| {
                                #[cfg(target_os = "linux")]
                                let button = if close {
                                    super::linux::round_window_corners(button, window)
                                        .rounded_tl(px(0.))
                                        .rounded_bl(px(0.))
                                        .rounded_br(px(0.))
                                } else {
                                    button
                                };
                                button
                            })
                            .when(cfg!(target_os = "linux"), |button| {
                                button.on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    match id {
                                        "caption-minimize" => window.minimize_window(),
                                        "caption-maximize" => window.zoom_window(),
                                        _ => window.dispatch_action(Box::new(CloseWindow), cx),
                                    }
                                })
                            })
                            .hover(move |button| {
                                button.bg(if close {
                                    theme::window_close_hover()
                                } else {
                                    alpha(theme::sidebar_selected(), 0.6)
                                })
                            })
                            .child(
                                svg()
                                    .flex_none()
                                    .path(icon)
                                    .size(px(10. * zoom))
                                    .text_color(theme::muted())
                                    .group_hover(id, move |icon| {
                                        icon.text_color(if close {
                                            theme::window_close_hover_ink()
                                        } else {
                                            theme::text()
                                        })
                                    }),
                            )
                    }),
                )
                .into_any_element(),
        )
    }
}
