use gpui::{svg, Div};

use crate::surfaces::app_shell::TITLEBAR_DRAG_HEIGHT;
use crate::surfaces::app_shell::{SIDEBAR_TOGGLE_GLYPH_INSET, SIDEBAR_TOGGLE_GLYPH_X};
use crate::*;

const TITLEBAR_HEIGHT: f32 = 52.;

pub(crate) const SETTINGS_DRAG_INSET: f32 = TITLEBAR_DRAG_HEIGHT;
pub(crate) const SETTINGS_CONTENT_TOP: f32 = 56.;

pub(crate) const PRIMARY_MODIFIER: &str = "⌘";
pub(crate) const SHORTCUT_REQUIREMENTS_KEY: &str =
    "ui.shortcuts_must_include_control_or_option_function_keys_can_be_used_alone";

#[cfg(not(test))]
pub(crate) fn sync_traffic_lights(window: &gpui::Window, zoom: f32) {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSView, NSWindowButton, NSWindowStyleMask};
    use raw_window_handle::RawWindowHandle;

    let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    let view_ptr = handle.ns_view.as_ptr().cast::<NSView>();
    let Some(view) = (unsafe { Retained::retain(view_ptr) }) else {
        return;
    };
    let Some(window) = view.window() else {
        return;
    };
    if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
        return;
    }
    let Some(close) = window.standardWindowButton(NSWindowButton::CloseButton) else {
        return;
    };
    let Some(minimize) = window.standardWindowButton(NSWindowButton::MiniaturizeButton) else {
        return;
    };
    let Some(maximize) = window.standardWindowButton(NSWindowButton::ZoomButton) else {
        return;
    };
    let Some(button_group) = (unsafe { close.superview() }) else {
        return;
    };
    let Some(titlebar_container) = (unsafe { button_group.superview() }) else {
        return;
    };

    let titlebar_height = 52. * zoom as f64;
    let close_frame = close.frame();
    let button_height = close_frame.size.height;
    let spacing = minimize.frame().origin.x - close_frame.origin.x;
    let mut titlebar_frame = titlebar_container.frame();
    titlebar_frame.size.height = titlebar_height;
    titlebar_frame.origin.y = window.frame().size.height - titlebar_height;
    titlebar_container.setFrame(titlebar_frame);

    for (index, button) in [close, minimize, maximize].into_iter().enumerate() {
        let mut frame = button.frame();
        frame.origin.x = 24. + index as f64 * spacing;
        frame.origin.y = titlebar_height - 30. * zoom as f64 - button_height / 2.;
        button.setFrameOrigin(frame.origin);
    }
}

#[cfg(test)]
pub(crate) fn sync_traffic_lights(_: &gpui::Window, _: f32) {}

#[cfg(not(test))]
pub(crate) fn show_about_panel() {
    use objc2::{runtime::AnyObject, MainThreadMarker};
    use objc2_app_kit::{
        NSAboutPanelOptionApplicationIcon, NSAboutPanelOptionApplicationName,
        NSAboutPanelOptionApplicationVersion, NSAboutPanelOptionVersion, NSApplication,
    };
    use objc2_foundation::{ns_string, NSDictionary, NSString};

    let main_thread = MainThreadMarker::new().expect("About Runner must open on the main thread");
    let app = NSApplication::sharedApplication(main_thread);
    let icon = app
        .applicationIconImage()
        .expect("Runner app icon is installed");
    let version = NSString::from_str(&runner_app::version::display_version());
    let values: [&AnyObject; 5] = [
        ns_string!("Runner").as_ref(),
        version.as_ref(),
        ns_string!("").as_ref(),
        ns_string!("© 2026 Yicheng Wang (Jason Wang)").as_ref(),
        icon.as_ref(),
    ];
    unsafe {
        let options = NSDictionary::from_slices(
            &[
                NSAboutPanelOptionApplicationName,
                NSAboutPanelOptionApplicationVersion,
                // The shared version already includes the commit or development marker.
                NSAboutPanelOptionVersion,
                ns_string!("Copyright"),
                NSAboutPanelOptionApplicationIcon,
            ],
            &values,
        );
        app.orderFrontStandardAboutPanelWithOptions(&options);
    }
}

pub(crate) fn primary_modifier_held(modifiers: gpui::Modifiers) -> bool {
    modifiers.platform
}

pub(crate) fn other_shortcut_modifiers_held(modifiers: gpui::Modifiers) -> bool {
    modifiers.control || modifiers.alt || modifiers.shift || modifiers.function
}

pub(crate) fn sidebar_row_wrapper(id: SharedString) -> gpui::Stateful<Div> {
    div().id(id).relative()
}

pub(crate) fn sidebar_section() -> Div {
    div().flex_none()
}

pub(crate) fn finish_window_close(_window: &mut Window) -> bool {
    true
}

pub(crate) fn navigation_left(window: &Window, zoom: f32) -> f32 {
    if window.is_fullscreen() {
        16. * zoom
    } else {
        SIDEBAR_TOGGLE_GLYPH_X - SIDEBAR_TOGGLE_GLYPH_INSET * zoom
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TitlebarAction {
    Move,
    Zoom,
}

#[cfg(test)]
thread_local! {
    static TITLEBAR_ACTIONS: std::cell::RefCell<Vec<TitlebarAction>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn take_titlebar_actions() -> Vec<TitlebarAction> {
    TITLEBAR_ACTIONS.with_borrow_mut(std::mem::take)
}

pub(crate) fn titlebar_mouse_down(event: &gpui::MouseDownEvent, window: &mut Window, cx: &mut App) {
    cx.stop_propagation();
    let action = if event.click_count == 2 {
        TitlebarAction::Zoom
    } else {
        TitlebarAction::Move
    };
    #[cfg(test)]
    {
        let _ = window;
        TITLEBAR_ACTIONS.with_borrow_mut(|actions| actions.push(action));
    }
    #[cfg(not(test))]
    match action {
        TitlebarAction::Zoom => window.titlebar_double_click(),
        TitlebarAction::Move => window.start_window_move(),
    }
}

impl NativeRoot {
    pub(crate) fn decorate_window(
        &self,
        root: Div,
        _window: &Window,
        _cx: &mut Context<Self>,
    ) -> Div {
        root
    }

    pub(crate) fn render_window_top_margin_drag_area(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        Some(
            self.render_titlebar_drag_area(
                "window-top-margin-drag",
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(rems(8. / 16.))
                    .debug_selector(|| "WINDOW_TOP_MARGIN_DRAG".into()),
                cx,
            )
            .into_any_element(),
        )
    }

    pub(crate) fn render_main_titlebar_drag_area(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let height = if self.front_page_has_islands() {
            TITLEBAR_DRAG_HEIGHT
        } else {
            WORKSPACE_HEADER_HEIGHT
        };
        Some(
            self.render_titlebar_drag_area(
                "main-titlebar-drag",
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(height * self.settings(cx).app_zoom))
                    .debug_selector(|| "MAIN_TITLEBAR_DRAG".into()),
                cx,
            )
            .into_any_element(),
        )
    }

    pub(crate) fn render_settings_titlebar_drag_area(
        &self,
        id: &'static str,
        area: Div,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        Some(
            self.render_titlebar_drag_area(id, area, cx)
                .into_any_element(),
        )
    }

    pub(crate) fn render_sidebar_titlebar(
        &self,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        Some(
            self.render_titlebar_drag_area(
                "sidebar-titlebar-drag",
                div()
                    .flex_none()
                    .h(px(TITLEBAR_HEIGHT * self.settings(cx).app_zoom)),
                cx,
            )
            .into_any_element(),
        )
    }

    pub(crate) fn render_window_navigation(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let titlebar_padding = navigation_left(window, self.settings(cx).app_zoom);
        let in_settings = self.route == AppRoute::Settings;
        let panel_path = if self.sidebar_collapsed {
            "panel-left-hidden.svg"
        } else {
            "panel-left-open.svg"
        };
        let (previous_page, next_page) = self.render_page_navigation_buttons(cx);
        let titlebar = self.render_titlebar_drag_area(
            "window-navigation-drag",
            div()
                .flex_none()
                .absolute()
                .top(px(16. * self.settings(cx).app_zoom))
                .left(px(titlebar_padding))
                .h(px(28. * self.settings(cx).app_zoom))
                .flex()
                .items_center()
                .child(
                    Self::titlebar_control_cluster("sidebar-titlebar-controls")
                        .debug_selector(|| "WINDOW_TITLEBAR_CONTROLS".into())
                        .child(
                            div()
                                .id("sidebar-toggle")
                                .debug_selector(|| "WINDOW_SIDEBAR_TOGGLE".into())
                                .group("sidebar-toggle")
                                .flex_none()
                                .w(px(28. * self.settings(cx).app_zoom))
                                .h(px(28. * self.settings(cx).app_zoom))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_sm()
                                .when(!in_settings, |button| button.cursor_pointer())
                                .opacity(if in_settings { 0.5 } else { 1. })
                                .text_color(theme::muted())
                                .hover(|button| {
                                    button
                                        .bg(theme::with_alpha(
                                            theme::chrome_selected(),
                                            theme::chrome_selected().a * 0.6,
                                        ))
                                        .text_color(theme::text())
                                })
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(
                                    svg()
                                        .path(panel_path)
                                        .w(px(14. * self.settings(cx).app_zoom))
                                        .h(px(14. * self.settings(cx).app_zoom))
                                        .flex_none()
                                        .text_color(theme::muted())
                                        .group_hover("sidebar-toggle", |icon| {
                                            icon.text_color(theme::text())
                                        }),
                                )
                                .on_click(cx.listener(|this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.toggle_window_sidebar(window, cx);
                                })),
                        )
                        .child(
                            div()
                                .debug_selector(|| "WINDOW_PREVIOUS_PAGE".into())
                                .child(previous_page),
                        )
                        .child(
                            div()
                                .debug_selector(|| "WINDOW_NEXT_PAGE".into())
                                .child(next_page),
                        ),
                ),
            cx,
        );
        Some(titlebar.into_any_element())
    }

    /// Previous/Next page arrows in the Windows order and treatment, driven by
    /// the same per-window page history. Both disable in Settings and at the
    /// history boundaries. No tooltips on macOS, by decision.
    fn render_page_navigation_buttons(&self, cx: &mut Context<Self>) -> (IconButton, IconButton) {
        let (can_go_back, can_go_forward) = self.page_navigation_state();
        let back_root = cx.entity();
        let forward_root = cx.entity();
        (
            IconButton::new("window-previous-page", "chevron-left.svg")
                .disabled(!can_go_back)
                .on_press(move |window, cx| {
                    back_root.update(cx, |this, cx| this.navigate_runtime_page(-1, window, cx));
                }),
            IconButton::new("window-next-page", "chevron-right.svg")
                .disabled(!can_go_forward)
                .on_press(move |window, cx| {
                    forward_root.update(cx, |this, cx| this.navigate_runtime_page(1, window, cx));
                }),
        )
    }

    /// The leading titlebar cluster. A press anywhere in it, including on a
    /// disabled arrow, must neither arm a window drag nor count as a
    /// titlebar double-click.
    pub(crate) fn titlebar_control_cluster(id: &'static str) -> gpui::Stateful<Div> {
        div()
            .id(id)
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, _, cx| cx.stop_propagation())
    }
}
